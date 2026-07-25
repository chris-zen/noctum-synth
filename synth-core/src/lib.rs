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
//! ControlMessage ──► VoiceManager ──► [VoiceBlock; VOICE_PACKS] ──► stereo sum
//!                              │
//!                              └── ParamId / patch state per voice
//! ```
//!
//! Host applications send [`ControlMessage`] values (note events, MIDI
//! controllers, and [`ParamId`] parameter updates). [`SynthEngine`] drains
//! those messages and fills an output buffer each audio callback.
//!
//! # Quick start
//!
//! ```
//! use synth_core::{SynthEngine, VOICE_PACKS};
//!
//! let mut engine = SynthEngine::<VOICE_PACKS>::new(synth_core::DEFAULT_SAMPLE_RATE);
//! engine.note_on(60, 1.0);
//!
//! let mut mono = [0.0f32; 256];
//! engine.process(&mut mono);
//! ```
//!
//! # Modules
//!
//! - [`dsp`] — generic signal processing (oscillators, filter, envelopes, LFOs)
//! - [`engine`] — top-level [`SynthEngine`] and master gain
//! - [`midi`] — MIDI clock, program import, and instrument SysEx codecs
//! - [`voice`] — voice manager (polyphony, stealing) and per-block DSP chain
//! - [`patch`] — parameter bundles and LFO destinations

#![no_std]

pub mod dsp;
pub mod effects;
pub mod engine;
pub mod fixed_index_list;
pub(crate) mod math;
#[cfg(feature = "embedded-math")]
mod micromath;
pub mod midi;
pub mod patch;
pub mod patch_storage;
pub mod profiling;
mod rate_adapter;
pub mod tuning;
pub mod voice;

#[cfg(feature = "embedded-math")]
pub use crate::micromath::f32x4;
#[cfg(not(feature = "embedded-math"))]
pub use wide::f32x4;

#[cfg(feature = "embedded-math")]
pub(crate) use crate::micromath::i32x4;
#[cfg(not(feature = "embedded-math"))]
pub(crate) use wide::i32x4;

pub use effects::{EffectModulation, Effects, EffectsWithMemory};
pub use engine::{SynthEngine, SynthEngineWithMemory};
pub use midi::{
    MidiClockMode, MidiClockStatus, MidiProgramImport, MidiProgramSource, MidiRealtimeEvent,
    MidiTransportState, P08MidiDecoder, P08ProgramData, Rev2MidiDecoder, Rev2MidiEncoder,
    Rev2MidiUpdate, Rev2ProgramData, Rev2SysexError, P08_PROGRAM_DATA_LEN,
    P08_PROGRAM_DATA_SYSEX_LEN, P08_PROGRAM_EDIT_BUFFER_SYSEX_LEN, P08_PROGRAM_PACKED_LEN,
    REV2_PROGRAM_DATA_LEN, REV2_PROGRAM_DATA_SYSEX_LEN, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN,
    REV2_PROGRAM_PACKED_LEN,
};
pub use patch::{
    AmplifierParams, AuxEnvelopeParams, ChordMemory, ClockDivision, DedicatedModSlot,
    DedicatedModSource, EffectParams, EffectType, FilterParams, GlideMode, KeyMode, LfoParams,
    LfoSyncDivision, ModDestination, ModMatrix, ModMatrixSlot, ModRoute, ModSource,
    OscillatorPatch, PanModMode, Patch, PatchName, UnisonMode, LFO_COUNT,
    MOD_MATRIX_FREE_SLOT_COUNT,
};
pub use patch_storage::{PatchRecord, PatchRecordError, PATCH_RECORD_SIZE};
pub use profiling::{RenderContext, RenderProfiler, RenderStage};
pub use tuning::midi_to_hz;
pub use voice::{
    glide_seconds, voice_pan_position, ActiveNotes, OscillatorModulation, OscillatorParams,
    Oscillators, OscillatorsOutput, OscillatorsParams, PerformanceModulation, VoiceBlock,
    VoiceManager, REV2_VOICE_PAN_POSITIONS,
};

use crate::dsp::{FilterOversampling, FilterType};

pub trait F32x4Ext {
    #[must_use]
    fn replace_lane(self, lane: usize, value: f32) -> Self;
}

#[cfg(not(feature = "embedded-math"))]
impl F32x4Ext for f32x4 {
    #[inline(always)]
    fn replace_lane(self, lane: usize, value: f32) -> Self {
        debug_assert!(lane < 4);
        let mut values = self.to_array();
        values[lane] = value;
        Self::new(values)
    }
}

/// Identifies a single synthesizer parameter for [`ControlMessage::SetParam`].
///
/// The UI, MIDI mapping layer, and [`VoiceManager`] all use this enum
/// to address patch state uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamId {
    Osc1Waveform,
    Osc1Enabled,
    Osc1Frequency,
    Osc1FineTune,
    Osc1ShapeMod,
    Osc1Level,
    Osc2Waveform,
    Osc2Enabled,
    Osc2Frequency,
    Osc2FineTune,
    Osc2ShapeMod,
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
    VcaInitialLevel,
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
    GlideMode,
    GlideEnabled,
    KeyMode,
    UnisonEnabled,
    UnisonMode,
    UnisonDetune,
    Bpm,
    ClockDivide,
    Lfo1Rate,
    Lfo1Depth,
    Lfo1Waveform,
    Lfo1Destination,
    Lfo1ClockSync,
    Lfo1SyncDivision,
    Lfo1KeySync,
    Lfo2Rate,
    Lfo2Depth,
    Lfo2Waveform,
    Lfo2Destination,
    Lfo2ClockSync,
    Lfo2SyncDivision,
    Lfo2KeySync,
    Lfo3Rate,
    Lfo3Depth,
    Lfo3Waveform,
    Lfo3Destination,
    Lfo3ClockSync,
    Lfo3SyncDivision,
    Lfo3KeySync,
    Lfo4Rate,
    Lfo4Depth,
    Lfo4Waveform,
    Lfo4Destination,
    Lfo4ClockSync,
    Lfo4SyncDivision,
    Lfo4KeySync,
    EffectEnabled,
    EffectType,
    EffectMix,
    EffectClockSync,
    EffectParam1,
    EffectParam2,
    AnalogDrift,
    VcaDrive,
    PanSpread,
    PanModMode,
    MasterVolume,
    PitchBendRange,
}

impl ParamId {
    /// Human-readable parameter label shared by hosts and diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Osc1Waveform => "Osc 1 Waveform",
            Self::Osc1Enabled => "Osc 1 Enabled",
            Self::Osc1Frequency => "Osc 1 Frequency",
            Self::Osc1FineTune => "Osc 1 Fine Tune",
            Self::Osc1ShapeMod => "Osc 1 Shape Mod",
            Self::Osc1Level => "Osc 1 Level",
            Self::Osc2Waveform => "Osc 2 Waveform",
            Self::Osc2Enabled => "Osc 2 Enabled",
            Self::Osc2Frequency => "Osc 2 Frequency",
            Self::Osc2FineTune => "Osc 2 Fine Tune",
            Self::Osc2ShapeMod => "Osc 2 Shape Mod",
            Self::Osc2Level => "Osc 2 Level",
            Self::OscMix => "Osc Mix",
            Self::SubOscLevel => "Sub Osc Level",
            Self::NoiseLevel => "Noise Level",
            Self::HardSync => "Hard Sync",
            Self::OscSlop => "Osc Slop",
            Self::Osc1NoteReset => "Osc 1 Note Reset",
            Self::Osc2NoteReset => "Osc 2 Note Reset",
            Self::Osc1KeyboardOn => "Osc 1 Keyboard",
            Self::Osc2KeyboardOn => "Osc 2 Keyboard",
            Self::Osc1Glide => "Osc 1 Glide",
            Self::Osc2Glide => "Osc 2 Glide",
            Self::FilterCutoff => "Filter Cutoff",
            Self::FilterResonance => "Filter Resonance",
            Self::FilterPoles => "Filter Poles",
            Self::FilterKeyTrack => "Filter Key Track",
            Self::FilterEnvAmount => "Filter Env Amount",
            Self::FilterVelocity => "Filter Velocity",
            Self::FilterAudioMod => "Filter Audio Mod",
            Self::FilterEgDelay => "Filter Delay",
            Self::FilterEgAttack => "Filter Attack",
            Self::FilterEgDecay => "Filter Decay",
            Self::FilterEgSustain => "Filter Sustain",
            Self::FilterEgRelease => "Filter Release",
            Self::VcaInitialLevel => "VCA Level",
            Self::AmpEnvAmount => "Amp Env Amount",
            Self::AmpVelocity => "Amp Velocity",
            Self::AmpEgDelay => "Amp Delay",
            Self::AmpEgAttack => "Amp Attack",
            Self::AmpEgDecay => "Amp Decay",
            Self::AmpEgSustain => "Amp Sustain",
            Self::AmpEgRelease => "Amp Release",
            Self::AuxEgDestination => "Aux Env Destination",
            Self::AuxEgAmount => "Aux Env Amount",
            Self::AuxEgVelocity => "Aux Env Velocity",
            Self::AuxEgDelay => "Aux Env Delay",
            Self::AuxEgAttack => "Aux Env Attack",
            Self::AuxEgDecay => "Aux Env Decay",
            Self::AuxEgSustain => "Aux Env Sustain",
            Self::AuxEgRelease => "Aux Env Release",
            Self::AuxEgLoop => "Aux Env Loop",
            Self::GlideMode => "Glide Mode",
            Self::GlideEnabled => "Glide On/Off",
            Self::KeyMode => "Key Mode",
            Self::UnisonEnabled => "Unison",
            Self::UnisonMode => "Unison Mode",
            Self::UnisonDetune => "Unison Detune",
            Self::Bpm => "BPM",
            Self::ClockDivide => "Clock Divide",
            Self::Lfo1Rate => "LFO 1 Rate",
            Self::Lfo1Depth => "LFO 1 Depth",
            Self::Lfo1Waveform => "LFO 1 Waveform",
            Self::Lfo1Destination => "LFO 1 Destination",
            Self::Lfo1ClockSync => "LFO 1 Clock Sync",
            Self::Lfo1SyncDivision => "LFO 1 Sync Division",
            Self::Lfo1KeySync => "LFO 1 Key Sync",
            Self::Lfo2Rate => "LFO 2 Rate",
            Self::Lfo2Depth => "LFO 2 Depth",
            Self::Lfo2Waveform => "LFO 2 Waveform",
            Self::Lfo2Destination => "LFO 2 Destination",
            Self::Lfo2ClockSync => "LFO 2 Clock Sync",
            Self::Lfo2SyncDivision => "LFO 2 Sync Division",
            Self::Lfo2KeySync => "LFO 2 Key Sync",
            Self::Lfo3Rate => "LFO 3 Rate",
            Self::Lfo3Depth => "LFO 3 Depth",
            Self::Lfo3Waveform => "LFO 3 Waveform",
            Self::Lfo3Destination => "LFO 3 Destination",
            Self::Lfo3ClockSync => "LFO 3 Clock Sync",
            Self::Lfo3SyncDivision => "LFO 3 Sync Division",
            Self::Lfo3KeySync => "LFO 3 Key Sync",
            Self::Lfo4Rate => "LFO 4 Rate",
            Self::Lfo4Depth => "LFO 4 Depth",
            Self::Lfo4Waveform => "LFO 4 Waveform",
            Self::Lfo4Destination => "LFO 4 Destination",
            Self::Lfo4ClockSync => "LFO 4 Clock Sync",
            Self::Lfo4SyncDivision => "LFO 4 Sync Division",
            Self::Lfo4KeySync => "LFO 4 Key Sync",
            Self::EffectEnabled => "Effect Enabled",
            Self::EffectType => "Effect Type",
            Self::EffectMix => "Effect Mix",
            Self::EffectClockSync => "Effect Clock Sync",
            Self::EffectParam1 => "Effect Param 1",
            Self::EffectParam2 => "Effect Param 2",
            Self::AnalogDrift => "Analog Drift",
            Self::VcaDrive => "VCA Drive",
            Self::PanSpread => "Pan Spread",
            Self::PanModMode => "Pan Mod Mode",
            Self::MasterVolume => "Master Volume",
            Self::PitchBendRange => "Pitch Bend Range",
        }
    }
}

/// One independently addressable field of a modulation route.
///
/// MIDI NRPN messages update modulation source, amount, and destination as
/// separate parameters, whereas the UI normally submits a complete route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModulationParam {
    Source(ModSource),
    Destination(ModDestination),
    Amount(f32),
}

/// Host-to-engine control and performance input.
pub enum ControlMessage {
    SetParam(ParamId, f32),
    /// Replaces the native chord-memory voicing used by unison Chord mode.
    SetUnisonChord(ChordMemory),
    /// Updates the local tempo used when an external MIDI clock is not active.
    SetTempoBpm {
        bpm: f32,
    },
    SetMidiClockMode(MidiClockMode),
    MidiRealtime(MidiRealtimeEvent),
    SetModulation {
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    },
    SetModulationParam {
        route: ModRoute,
        parameter: ModulationParam,
    },
    /// Changes nonlinear filter self-oscillation oversampling without rebuilding
    /// the audio stream.
    SetFilterOversampling(FilterOversampling),
    /// Selects a filter model and resets its per-voice DSP state.
    SetFilterType(FilterType),
    NoteOn {
        note: u8,
        velocity: f32,
    },
    NoteOff {
        note: u8,
    },
    AllNotesOff,
    PitchBend {
        value: f32,
    },
    ModWheel {
        value: f32,
    },
    Pressure {
        value: f32,
    },
    SustainPedal {
        pressed: bool,
    },
    ControlChange {
        controller: u8,
        value: f32,
    },
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
/// Default transport tempo used by clock-synchronized effects.
pub const DEFAULT_TEMPO_BPM: f32 = 120.0;

/// Wrap a phase value into the `[0, 1)` range.
#[inline]
pub(crate) fn wrap01(phase: f32x4) -> f32x4 {
    phase - phase.floor()
}

#[cfg(test)]
mod tests {
    use super::ParamId;

    #[test]
    fn parameter_names_are_human_readable() {
        assert_eq!(ParamId::Osc1Frequency.name(), "Osc 1 Frequency");
        assert_eq!(ParamId::FilterEgAttack.name(), "Filter Attack");
        assert_eq!(ParamId::AmpEgAttack.name(), "Amp Attack");
        assert_eq!(ParamId::Lfo4Destination.name(), "LFO 4 Destination");
        assert_eq!(ParamId::EffectParam2.name(), "Effect Param 2");
        assert_eq!(ParamId::MasterVolume.name(), "Master Volume");
    }
}
