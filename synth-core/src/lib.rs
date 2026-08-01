//! Virtual-analog synthesis engine for Rust.
//!
//! `synth-core` is a `#![no_std]` library that implements a complete subtractive
//! synthesizer voice: dual band-limited oscillators, sub oscillator, noise, a
//! nonlinear ladder filter, three envelopes, and four LFOs. Sixteen voices are
//! rendered using SIMD ([`crate::math::WideF32::LANES`]) so each [`VoiceBlock`]
//! processes several notes in parallel.
//!
//! DSP algorithms are based on *Designing Software Synthesizer Plugins in C++*
//! by Will C. Pirkle, adapted to the Prophet Rev2 architecture and ported to
//! Rust with SIMD voice rendering.
//!
//! # Architecture
//!
//! ```text
//! ControlMessage ──► SynthEngine ──► LayerEngine(s) ──► VoicePool ──► stereo sum
//!                         │                │                 │
//!                         │                │                 └── [VoiceBlock; VOICE_PACKS]
//!                         │                └── allocation / patch / effects state
//!                         └── layer targeting / note routing / topology
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
//! - [`voice`] — layer allocation, shared voice storage, and per-block DSP chain
//! - [`patch`] — parameter bundles and LFO destinations

#![no_std]

#[cfg(not(any(
    all(feature = "wide-8", not(feature = "wide-4"), not(feature = "wide-1")),
    all(feature = "wide-4", not(feature = "wide-8"), not(feature = "wide-1")),
    all(feature = "wide-1", not(feature = "wide-8"), not(feature = "wide-4")),
)))]
compile_error!("Exactly one of the `wide-8`, `wide-4`, or `wide-1` features must be enabled.");

pub(crate) mod arp;
pub mod dsp;
pub mod effects;
pub mod engine;
pub mod fixed_index_list;
pub mod math;
pub mod midi;
pub mod patch;
pub mod patch_storage;
pub(crate) mod pressed_keys;
pub mod profiling;
pub mod program;
mod rate_adapter;
pub mod sequencer;
pub mod tuning;
pub mod voice;

use crate::math::WideF32;

pub use effects::{EffectModulation, Effects, EffectsState, EffectsWithMemory};
pub use engine::{EngineInitError, LayerPlaybackStatus, SynthEngine, SynthEngineWithMemory};
pub use patch::{
    AmplifierParams, ArpMode, ArpParams, ArpSustainMode, AuxEnvelopeParams, ChordMemory,
    ClockDivision, DedicatedModSlot, DedicatedModSource, EffectParams, EffectType, FilterParams,
    GlideMode, KeyMode, LFO_COUNT, LayerPatch, LfoParams, LfoSyncDivision,
    MOD_MATRIX_FREE_SLOT_COUNT, ModDestination, ModMatrix, ModMatrixSlot, ModRoute, ModSource,
    OscillatorPatch, PanModMode, PatchName, UnisonMode,
};
pub use patch_storage::{PATCH_RECORD_SIZE, PatchRecord, PatchRecordError};
pub use profiling::{RenderContext, RenderProfiler, RenderStage};
pub use program::{
    DEFAULT_SPLIT_POINT, LayerId, LayerMode, LayerTarget, MAX_SPLIT_POINT, MIN_SPLIT_POINT, Patch,
};
pub use sequencer::model::{
    GATED_STEP_COUNT, GATED_TRACK_COUNT, GatedDestination, GatedSequence, GatedSequencerMode,
    GatedStep, GatedTrack, LayerSequence, POLY_LANE_COUNT, POLY_STEP_COUNT, PolyLaneStep, PolyNote,
    PolySequence, PolyStep, PolyVelocity, SequenceClear, SequenceUpdate, SequencerFeedback,
    SequencerRecordCommand, SequencerTransportCommand, SequencerType,
};
pub use tuning::midi_to_hz;
pub use voice::{
    ActiveNotes, LayerEngine, OscillatorModulation, OscillatorParams, Oscillators,
    OscillatorsOutput, OscillatorsParams, PerformanceModulation, REV2_VOICE_PAN_POSITIONS,
    VoiceBlock, VoicePool, VoiceRegion, glide_seconds, voice_pan_position,
};

use crate::dsp::{FilterOversampling, FilterType};
use crate::midi::clock::{MidiClockMode, MidiRealtimeEvent};

/// Identifies a single synthesizer parameter for [`ControlMessage::SetParam`].
///
/// The UI, MIDI mapping layer, and [`LayerEngine`] all use this enum
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
    ArpEnabled,
    ArpMode,
    ArpRange,
    ArpRepeats,
    ArpRelatch,
    ArpHold,
    ArpBeatSync,
    ArpSustainMode,
    SequencerType,
    GatedSequencerMode,
    ProgramVolume,
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
            Self::ArpEnabled => "Arp On/Off",
            Self::ArpMode => "Arp Mode",
            Self::ArpRange => "Arp Range",
            Self::ArpRepeats => "Arp Repeats",
            Self::ArpRelatch => "Arp Relatch",
            Self::ArpHold => "Arp Hold",
            Self::ArpBeatSync => "Arp Beat Sync",
            Self::ArpSustainMode => "Arp Sustain Mode",
            Self::SequencerType => "Sequencer Type",
            Self::GatedSequencerMode => "Gated Sequencer Mode",
            Self::ProgramVolume => "Program Volume",
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
    SetParam {
        target: LayerTarget,
        param: ParamId,
        value: f32,
    },
    /// Replaces the native chord-memory voicing used by unison Chord mode.
    SetUnisonChord {
        target: LayerTarget,
        chord: ChordMemory,
    },
    /// Updates the local tempo used when an external MIDI clock is not active.
    SetTempoBpm {
        target: LayerTarget,
        bpm: f32,
    },
    SetMidiClockMode(MidiClockMode),
    /// Sets the device-global master output level (not stored in the patch).
    SetMasterVolume(f32),
    MidiRealtime(MidiRealtimeEvent),
    SetModulation {
        target: LayerTarget,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    },
    SetModulationParam {
        target: LayerTarget,
        route: ModRoute,
        parameter: ModulationParam,
    },
    SetSequence {
        target: LayerTarget,
        update: SequenceUpdate,
    },
    SetSequencerTransport {
        target: LayerTarget,
        command: SequencerTransportCommand,
    },
    /// Convenience transport control used by UI and Rev2 play/stop NRPN paths.
    SetSequencerRunning {
        target: LayerTarget,
        running: bool,
    },
    SequencerRecord {
        target: LayerTarget,
        command: SequencerRecordCommand,
    },
    ClearSequence {
        target: LayerTarget,
        section: SequenceClear,
    },
    SetLayerMode(LayerMode),
    SetSplitPoint(u8),
    SetEditLayer(LayerId),
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

impl ControlMessage {
    /// Builds an edit-layer parameter update for controls without an explicit layer.
    pub const fn edit_param(param: ParamId, value: f32) -> Self {
        Self::SetParam {
            target: LayerTarget::Edit,
            param,
            value,
        }
    }

    /// Builds an edit-layer chord-memory update.
    pub const fn edit_unison_chord(chord: ChordMemory) -> Self {
        Self::SetUnisonChord {
            target: LayerTarget::Edit,
            chord,
        }
    }
}

/// Total polyphonic voice count.
pub const VOICE_COUNT: usize = 16;
/// Number of [`VoiceBlock`] instances (`VOICE_COUNT / WideF32::LANES`).
pub const VOICE_PACKS: usize = VOICE_COUNT / WideF32::LANES;
const _: () = assert!(VOICE_COUNT % WideF32::LANES == 0);

/// Default sample rate used when constructing DSP objects (44.1 kHz).
pub const DEFAULT_SAMPLE_RATE: f32 = 44100.0;
/// Default transport tempo used by clock-synchronized effects.
pub const DEFAULT_TEMPO_BPM: f32 = 120.0;

/// Wrap a phase value into the `[0, 1)` range.
#[inline]
pub(crate) fn wrap01(phase: WideF32) -> WideF32 {
    phase.wrap01()
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
        assert_eq!(ParamId::ProgramVolume.name(), "Program Volume");
    }
}
