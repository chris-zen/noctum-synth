//! Virtual-analog synthesis engine for Rust.
//!
//! `synth-core` is a `#![no_std]` library that implements a complete subtractive
//! synthesizer voice: dual band-limited oscillators, sub oscillator, noise, a
//! nonlinear ladder filter, three envelopes, and four LFOs. Sixteen voices are
//! rendered using four-wide SIMD ([`LANES`]) so each [`VoiceBlock`] processes
//! four notes in parallel.
//!
//! DSP algorithms are based on *Designing Software Synthesizer Plugins in C++*
//! by Will C. Pirkle, adapted to the Prophet Rev2 architecture and ported to
//! Rust with SIMD voice rendering.
//!
//! # Architecture
//!
//! ```text
//! ControlMessage ──► Voices ──► [VoiceBlock; VOICE_PACKS] ──► stereo sum
//!                         │
//!                         └── ParamId / patch state per voice
//! ```
//!
//! Host applications send [`ControlMessage`] values (note events, MIDI
//! controllers, and [`ParamId`] parameter updates). [`SynthEngine`] drains
//! those messages and fills an output buffer each audio callback.
//!
//! # Quick start
//!
//! ```
//! use synth_core::SynthEngine;
//!
//! let mut engine = SynthEngine::new(synth_core::DEFAULT_SAMPLE_RATE);
//! engine.note_on(60, 1.0);
//!
//! let mut mono = [0.0f32; 256];
//! engine.process(&mut mono);
//! ```
//!
//! # Modules
//!
//! - [`engine`] — top-level [`SynthEngine`] and master gain
//! - [`voices`] — polyphony, sustain pedal, and voice stealing
//! - [`voice`] — per-block DSP chain
//! - [`analog_oscillator`] / [`analog_oscillators`] — waveform generation and mixing
//! - [`filter`] — ladder low-pass with key track and audio modulation
//! - [`envelope`] — delayed ADSR ([`DadsrEnvelope`])
//! - [`lfo`] — low-frequency modulation
//! - [`patch`] — parameter bundles and LFO destinations

#![no_std]

#[cfg(test)]
extern crate std;

pub mod analog_oscillator;
pub mod analog_oscillators;
pub mod analog_sub_oscillator;
pub mod blep;
pub mod effects;
pub mod engine;
pub mod envelope;
pub mod filter;
pub mod fixed_index_list;
pub mod lfo;
pub(crate) mod math;
pub mod noise;
pub mod patch;
pub(crate) mod rng;
pub mod tuning;
pub mod voice;
pub mod voices;

use wide::f32x4;

pub use analog_oscillator::{AnalogOscillator, SawMethod, Waveform};
pub use analog_oscillators::{
    OscillatorModulation, OscillatorParams, Oscillators, OscillatorsOutput, OscillatorsParams,
};
pub use analog_sub_oscillator::AnalogSubOscillator;
pub use engine::SynthEngine;
pub use envelope::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    DadsrEnvelope,
};
pub use filter::LadderFilter;
pub use lfo::{Lfo, LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
pub use noise::WhiteNoise;
pub use patch::{AmplifierParams, AuxEnvelopeParams, FilterParams, LfoDestination, LfoParams};
pub use tuning::midi_to_hz;
pub use voice::VoiceBlock;
pub use voices::{ActiveNotes, Voices};

/// Identifies a single synthesizer parameter for [`ControlMessage::SetParam`].
///
/// The UI, MIDI mapping layer, and [`Voices`] voice allocator all use this enum
/// to address patch state uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamId {
    Osc1Waveform,
    Osc1Enabled,
    Osc1Frequency,
    Osc1FineTune,
    Osc1Shape,
    Osc1Level,
    Osc2Waveform,
    Osc2Enabled,
    Osc2Frequency,
    Osc2FineTune,
    Osc2Shape,
    Osc2Level,
    OscMix,
    SubOscLevel,
    NoiseLevel,
    HardSync,
    OscSlop,
    Osc1NoteReset,
    Osc2NoteReset,
    Osc1KeyboardOn,
    Osc2KeyboardOn,
    Osc1Glide,
    Osc2Glide,
    GlideTime,
    FilterCutoff,
    FilterResonance,
    FilterPoles,
    FilterKeyTrack,
    FilterEnvAmount,
    FilterVelocity,
    FilterAudioMod,
    FilterEgDelay,
    FilterEgAttack,
    FilterEgDecay,
    FilterEgSustain,
    FilterEgRelease,
    AmpEnvAmount,
    AmpVelocity,
    AmpEgDelay,
    AmpEgAttack,
    AmpEgDecay,
    AmpEgSustain,
    AmpEgRelease,
    AuxEgDestination,
    AuxEgAmount,
    AuxEgVelocity,
    AuxEgDelay,
    AuxEgAttack,
    AuxEgDecay,
    AuxEgSustain,
    AuxEgRelease,
    AuxEgLoop,
    Lfo1Rate,
    Lfo1Depth,
    Lfo1Waveform,
    Lfo1Destination,
    Lfo1ClockSync,
    Lfo1KeySync,
    Lfo2Rate,
    Lfo2Depth,
    Lfo2Waveform,
    Lfo2Destination,
    Lfo2ClockSync,
    Lfo2KeySync,
    Lfo3Rate,
    Lfo3Depth,
    Lfo3Waveform,
    Lfo3Destination,
    Lfo3ClockSync,
    Lfo3KeySync,
    Lfo4Rate,
    Lfo4Depth,
    Lfo4Waveform,
    Lfo4Destination,
    Lfo4ClockSync,
    Lfo4KeySync,
    AnalogDrift,
    VcaDrive,
    PanSpread,
    MasterVolume,
}

/// Host-to-engine control and performance input.
pub enum ControlMessage {
    SetParam(ParamId, f32),
    NoteOn { note: u8, velocity: f32 },
    NoteOff { note: u8 },
    AllNotesOff,
    PitchBend { value: f32 },
    ModWheel { value: f32 },
    SustainPedal { pressed: bool },
    ControlChange { controller: u8, value: f32 },
}

/// Circle constant π.
pub const PI: f32 = core::f32::consts::PI;
/// Full circle in radians (2π).
pub const TAU: f32 = 2.0 * PI;
/// SIMD width: number of voices rendered per [`VoiceBlock`] step.
pub const LANES: usize = 4;
/// Total polyphonic voice count.
pub const VOICE_COUNT: usize = 16;
/// Number of [`VoiceBlock`] instances (`VOICE_COUNT / LANES`).
pub const VOICE_PACKS: usize = VOICE_COUNT / LANES;
const _: () = assert!(VOICE_COUNT % LANES == 0);

/// Default sample rate used when constructing DSP objects (44.1 kHz).
pub const DEFAULT_SAMPLE_RATE: f32 = 44100.0;

/// Wrap a phase value into the `[0, 1)` range.
#[inline]
pub(crate) fn wrap01(phase: f32x4) -> f32x4 {
    phase - phase.floor()
}
