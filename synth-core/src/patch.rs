//! Patch parameter bundles and modulation routing targets.

use crate::ParamId;
use crate::dsp::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    LfoWaveform, MIN_LFO_RATE_HZ,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub const PATCH_NAME_CAPACITY: usize = 20;
pub const CHORD_MEMORY_CAPACITY: usize = 16;
pub const LFO_COUNT: usize = 4;
pub const MOD_MATRIX_FREE_SLOT_COUNT: usize = 8;
pub const MAX_ARP_NOTES: usize = 16;
pub const MAX_ARP_STEPS: usize = MAX_ARP_NOTES * 3 * 3;

pub type PatchName = heapless::String<PATCH_NAME_CAPACITY>;

pub fn decode_patch_name(bytes: &[u8]) -> PatchName {
    let mut name = PatchName::new();
    for &byte in bytes {
        if (0x20..=0x7e).contains(&byte) {
            let _ = name.push(byte as char);
        }
    }
    while name.ends_with(' ') {
        name.pop();
    }
    name
}

/// Target for a modulation route.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModDestination {
    Off,
    Osc1Frequency,
    Osc2Frequency,
    OscAllFrequency,
    OscMix,
    NoiseLevel,
    SubOscLevel,
    Osc1ShapeMod,
    Osc2ShapeMod,
    OscAllShapeMod,
    FilterCutoff,
    FilterResonance,
    FilterAudioMod,
    Vca,
    Pan,
    Lfo1Frequency,
    Lfo2Frequency,
    Lfo3Frequency,
    Lfo4Frequency,
    LfoAllFrequency,
    Lfo1Amount,
    Lfo2Amount,
    Lfo3Amount,
    Lfo4Amount,
    LfoAllAmount,
    LpFilterEnvAmount,
    AmpEnvAmount,
    Env3Amount,
    EnvAllAmount,
    LpFilterAttack,
    VcaAttack,
    Env3Attack,
    EnvAllAttack,
    LpFilterDecay,
    VcaDecay,
    Env3Decay,
    EnvAllDecay,
    LpFilterRelease,
    VcaRelease,
    Env3Release,
    EnvAllRelease,
    Mod1Amount,
    Mod2Amount,
    Mod3Amount,
    Mod4Amount,
    Mod5Amount,
    Mod6Amount,
    Mod7Amount,
    Mod8Amount,
    OscSlop,
    FxMix,
    FxParam1,
    FxParam2,
}

impl ModDestination {
    pub const COUNT: usize = 53;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Off,
        Self::Osc1Frequency,
        Self::Osc2Frequency,
        Self::OscAllFrequency,
        Self::OscMix,
        Self::NoiseLevel,
        Self::SubOscLevel,
        Self::Osc1ShapeMod,
        Self::Osc2ShapeMod,
        Self::OscAllShapeMod,
        Self::FilterCutoff,
        Self::FilterResonance,
        Self::FilterAudioMod,
        Self::Vca,
        Self::Pan,
        Self::Lfo1Frequency,
        Self::Lfo2Frequency,
        Self::Lfo3Frequency,
        Self::Lfo4Frequency,
        Self::LfoAllFrequency,
        Self::Lfo1Amount,
        Self::Lfo2Amount,
        Self::Lfo3Amount,
        Self::Lfo4Amount,
        Self::LfoAllAmount,
        Self::LpFilterEnvAmount,
        Self::AmpEnvAmount,
        Self::Env3Amount,
        Self::EnvAllAmount,
        Self::LpFilterAttack,
        Self::VcaAttack,
        Self::Env3Attack,
        Self::EnvAllAttack,
        Self::LpFilterDecay,
        Self::VcaDecay,
        Self::Env3Decay,
        Self::EnvAllDecay,
        Self::LpFilterRelease,
        Self::VcaRelease,
        Self::Env3Release,
        Self::EnvAllRelease,
        Self::Mod1Amount,
        Self::Mod2Amount,
        Self::Mod3Amount,
        Self::Mod4Amount,
        Self::Mod5Amount,
        Self::Mod6Amount,
        Self::Mod7Amount,
        Self::Mod8Amount,
        Self::OscSlop,
        Self::FxMix,
        Self::FxParam1,
        Self::FxParam2,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Off)
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|destination| *destination == self)
            .unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Osc1Frequency => "Osc 1 Freq",
            Self::Osc2Frequency => "Osc 2 Freq",
            Self::OscAllFrequency => "Osc All Freq",
            Self::OscMix => "Osc Mix",
            Self::NoiseLevel => "Noise Level",
            Self::SubOscLevel => "Sub Osc Level",
            Self::Osc1ShapeMod => "Osc 1 Shape Mod",
            Self::Osc2ShapeMod => "Osc 2 Shape Mod",
            Self::OscAllShapeMod => "Osc All Shape Mod",
            Self::FilterCutoff => "Filter Cutoff",
            Self::FilterResonance => "Filter Resonance",
            Self::FilterAudioMod => "Filter Audio Mod",
            Self::Vca => "VCA",
            Self::Pan => "Pan",
            Self::Lfo1Frequency => "LFO 1 Freq",
            Self::Lfo2Frequency => "LFO 2 Freq",
            Self::Lfo3Frequency => "LFO 3 Freq",
            Self::Lfo4Frequency => "LFO 4 Freq",
            Self::LfoAllFrequency => "LFO All Freq",
            Self::Lfo1Amount => "LFO 1 Amount",
            Self::Lfo2Amount => "LFO 2 Amount",
            Self::Lfo3Amount => "LFO 3 Amount",
            Self::Lfo4Amount => "LFO 4 Amount",
            Self::LfoAllAmount => "LFO All Amount",
            Self::LpFilterEnvAmount => "LP Filter Env Amount",
            Self::AmpEnvAmount => "Amp Env Amount",
            Self::Env3Amount => "Env 3 Amount",
            Self::EnvAllAmount => "Env All Amount",
            Self::LpFilterAttack => "LPF Attack",
            Self::VcaAttack => "VCA Attack",
            Self::Env3Attack => "Env 3 Attack",
            Self::EnvAllAttack => "Env All Attack",
            Self::LpFilterDecay => "LPF Decay",
            Self::VcaDecay => "VCA Decay",
            Self::Env3Decay => "Env 3 Decay",
            Self::EnvAllDecay => "Env All Decay",
            Self::LpFilterRelease => "LPF Release",
            Self::VcaRelease => "VCA Release",
            Self::Env3Release => "Env 3 Release",
            Self::EnvAllRelease => "Env All Release",
            Self::Mod1Amount => "Mod 1 Amount",
            Self::Mod2Amount => "Mod 2 Amount",
            Self::Mod3Amount => "Mod 3 Amount",
            Self::Mod4Amount => "Mod 4 Amount",
            Self::Mod5Amount => "Mod 5 Amount",
            Self::Mod6Amount => "Mod 6 Amount",
            Self::Mod7Amount => "Mod 7 Amount",
            Self::Mod8Amount => "Mod 8 Amount",
            Self::OscSlop => "Osc Slop",
            Self::FxMix => "FX Mix",
            Self::FxParam1 => "FX Param 1",
            Self::FxParam2 => "FX Param 2",
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    Off,
    Seq1,
    Seq2,
    Seq3,
    Seq4,
    Lfo1,
    Lfo2,
    Lfo3,
    Lfo4,
    EnvLpf,
    EnvVca,
    Env3,
    PitchBend,
    ModWheel,
    Pressure,
    Breath,
    FootPedal,
    ExpressionPedal,
    Velocity,
    NoteNumber,
    Noise,
    Dc,
    AudioOut,
}

impl ModSource {
    pub const ALL: [Self; 23] = [
        Self::Off,
        Self::Seq1,
        Self::Seq2,
        Self::Seq3,
        Self::Seq4,
        Self::Lfo1,
        Self::Lfo2,
        Self::Lfo3,
        Self::Lfo4,
        Self::EnvLpf,
        Self::EnvVca,
        Self::Env3,
        Self::PitchBend,
        Self::ModWheel,
        Self::Pressure,
        Self::Breath,
        Self::FootPedal,
        Self::ExpressionPedal,
        Self::Velocity,
        Self::NoteNumber,
        Self::Noise,
        Self::Dc,
        Self::AudioOut,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Off)
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|source| *source == self)
            .unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Seq1 => "Seq 1",
            Self::Seq2 => "Seq 2",
            Self::Seq3 => "Seq 3",
            Self::Seq4 => "Seq 4",
            Self::Lfo1 => "LFO 1",
            Self::Lfo2 => "LFO 2",
            Self::Lfo3 => "LFO 3",
            Self::Lfo4 => "LFO 4",
            Self::EnvLpf => "Env LPF",
            Self::EnvVca => "Env VCA",
            Self::Env3 => "Env 3",
            Self::PitchBend => "Pitch Bend",
            Self::ModWheel => "Mod Wheel",
            Self::Pressure => "Pressure",
            Self::Breath => "Breath",
            Self::FootPedal => "Foot Pedal",
            Self::ExpressionPedal => "Expression Pedal",
            Self::Velocity => "Velocity",
            Self::NoteNumber => "Note Number",
            Self::Noise => "Noise",
            Self::Dc => "DC",
            Self::AudioOut => "Audio Out",
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedicatedModSource {
    ModWheel,
    Pressure,
    Breath,
    Velocity,
    Footswitch,
}

impl DedicatedModSource {
    pub const COUNT: usize = 5;
    pub const ALL: [Self; Self::COUNT] = [
        Self::ModWheel,
        Self::Pressure,
        Self::Breath,
        Self::Velocity,
        Self::Footswitch,
    ];

    pub fn source(self) -> ModSource {
        match self {
            Self::ModWheel => ModSource::ModWheel,
            Self::Pressure => ModSource::Pressure,
            Self::Breath => ModSource::Breath,
            Self::Velocity => ModSource::Velocity,
            Self::Footswitch => ModSource::FootPedal,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ModWheel => "Mod Wheel",
            Self::Pressure => "Pressure",
            Self::Breath => "Breath",
            Self::Velocity => "Velocity",
            Self::Footswitch => "MIDI Footswitch",
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModRoute {
    Free(usize),
    Dedicated(DedicatedModSource),
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct ModMatrixSlot {
    pub enabled: bool,
    pub source: ModSource,
    pub destination: ModDestination,
    pub amount: f32,
}

impl Default for ModMatrixSlot {
    fn default() -> Self {
        Self {
            enabled: false,
            source: ModSource::Off,
            destination: ModDestination::Off,
            amount: 0.0,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct DedicatedModSlot {
    pub enabled: bool,
    pub destination: ModDestination,
    pub amount: f32,
}

impl Default for DedicatedModSlot {
    fn default() -> Self {
        Self {
            enabled: false,
            destination: ModDestination::Off,
            amount: 0.0,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct ModMatrix {
    pub free_slots: [ModMatrixSlot; MOD_MATRIX_FREE_SLOT_COUNT],
    pub dedicated: [DedicatedModSlot; DedicatedModSource::COUNT],
}

impl Default for ModMatrix {
    fn default() -> Self {
        Self {
            free_slots: [ModMatrixSlot::default(); MOD_MATRIX_FREE_SLOT_COUNT],
            dedicated: [DedicatedModSlot::default(); DedicatedModSource::COUNT],
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectType {
    DelayMono,
    DdlStereo,
    #[cfg_attr(feature = "serde", serde(alias = "BbdDelay"))]
    BucketBrigadeDelay,
    Chorus,
    PhaserHigh,
    PhaserLow,
    PhaserMst,
    Flanger1,
    Flanger2,
    Reverb,
    RingMod,
    Distortion,
    HighPassFilter,
}

impl EffectType {
    pub const ALL: [Self; 13] = [
        Self::DelayMono,
        Self::DdlStereo,
        Self::BucketBrigadeDelay,
        Self::Chorus,
        Self::PhaserHigh,
        Self::PhaserLow,
        Self::PhaserMst,
        Self::Flanger1,
        Self::Flanger2,
        Self::Reverb,
        Self::RingMod,
        Self::Distortion,
        Self::HighPassFilter,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::DelayMono)
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|effect| *effect == self)
            .unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::DelayMono => "Delay Mono",
            Self::DdlStereo => "DDL Stereo",
            Self::BucketBrigadeDelay => "Bucket Brigade Delay",
            Self::Chorus => "Chorus",
            Self::PhaserHigh => "Phaser High",
            Self::PhaserLow => "Phaser Low",
            Self::PhaserMst => "Phaser Mst",
            Self::Flanger1 => "Flanger 1",
            Self::Flanger2 => "Flanger 2",
            Self::Reverb => "Reverb",
            Self::RingMod => "Ring Mod",
            Self::Distortion => "Distortion",
            Self::HighPassFilter => "HP Filter",
        }
    }

    pub fn is_delay(self) -> bool {
        matches!(
            self,
            Self::DelayMono | Self::DdlStereo | Self::BucketBrigadeDelay
        )
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct EffectParams {
    pub enabled: bool,
    pub effect_type: EffectType,
    pub mix: f32,
    pub clock_sync: bool,
    pub param1: f32,
    pub param2: f32,
}

impl Default for EffectParams {
    fn default() -> Self {
        Self {
            enabled: false,
            effect_type: EffectType::DelayMono,
            mix: 0.0,
            clock_sync: false,
            param1: 0.25,
            param2: 0.25,
        }
    }
}

/// Master-clock step division relative to one quarter note.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockDivision {
    Half,
    #[default]
    Quarter,
    Eighth,
    EighthHalfSwing,
    EighthSwing,
    EighthTriplet,
    Sixteenth,
    SixteenthHalfSwing,
    SixteenthSwing,
    SixteenthTriplet,
    ThirtySecond,
    ThirtySecondTriplet,
    SixtyFourthTriplet,
}

impl ClockDivision {
    pub const ALL: [Self; 13] = [
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::EighthHalfSwing,
        Self::EighthSwing,
        Self::EighthTriplet,
        Self::Sixteenth,
        Self::SixteenthHalfSwing,
        Self::SixteenthSwing,
        Self::SixteenthTriplet,
        Self::ThirtySecond,
        Self::ThirtySecondTriplet,
        Self::SixtyFourthTriplet,
    ];

    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Half,
            2 => Self::Eighth,
            3 => Self::EighthHalfSwing,
            4 => Self::EighthSwing,
            5 => Self::EighthTriplet,
            6 => Self::Sixteenth,
            7 => Self::SixteenthHalfSwing,
            8 => Self::SixteenthSwing,
            9 => Self::SixteenthTriplet,
            10 => Self::ThirtySecond,
            11 => Self::ThirtySecondTriplet,
            12 => Self::SixtyFourthTriplet,
            _ => Self::Quarter,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Half => 0,
            Self::Quarter => 1,
            Self::Eighth => 2,
            Self::EighthHalfSwing => 3,
            Self::EighthSwing => 4,
            Self::EighthTriplet => 5,
            Self::Sixteenth => 6,
            Self::SixteenthHalfSwing => 7,
            Self::SixteenthSwing => 8,
            Self::SixteenthTriplet => 9,
            Self::ThirtySecond => 10,
            Self::ThirtySecondTriplet => 11,
            Self::SixtyFourthTriplet => 12,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::EighthHalfSwing => "1/8 Half",
            Self::EighthSwing => "1/8 Swing",
            Self::EighthTriplet => "1/8 Trip",
            Self::Sixteenth => "1/16",
            Self::SixteenthHalfSwing => "1/16 Half",
            Self::SixteenthSwing => "1/16 Swing",
            Self::SixteenthTriplet => "1/16 Trip",
            Self::ThirtySecond => "1/32",
            Self::ThirtySecondTriplet => "1/32 Trip",
            Self::SixtyFourthTriplet => "1/64 Trip",
        }
    }

    /// Nominal master-clock steps per quarter note. Swing changes event timing,
    /// but not the continuous LFO's average frequency.
    pub const fn steps_per_quarter(self) -> f32 {
        match self {
            Self::Half => 0.5,
            Self::Quarter => 1.0,
            Self::Eighth | Self::EighthHalfSwing | Self::EighthSwing => 2.0,
            Self::EighthTriplet => 3.0,
            Self::Sixteenth | Self::SixteenthHalfSwing | Self::SixteenthSwing => 4.0,
            Self::SixteenthTriplet => 6.0,
            Self::ThirtySecond => 8.0,
            Self::ThirtySecondTriplet => 12.0,
            Self::SixtyFourthTriplet => 16.0,
        }
    }
}

/// LFO cycles relative to one master-clock step while clock sync is enabled.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoSyncDivision {
    Steps32,
    Steps16,
    Steps8,
    Steps6,
    Steps4,
    Steps3,
    Steps2,
    StepsOneAndHalf,
    #[default]
    Step1,
    StepTwoThirds,
    StepOneHalf,
    StepOneThird,
    StepOneQuarter,
    StepOneSixth,
    StepOneEighth,
    StepOneSixteenth,
}

impl LfoSyncDivision {
    pub const ALL: [Self; 16] = [
        Self::Steps32,
        Self::Steps16,
        Self::Steps8,
        Self::Steps6,
        Self::Steps4,
        Self::Steps3,
        Self::Steps2,
        Self::StepsOneAndHalf,
        Self::Step1,
        Self::StepTwoThirds,
        Self::StepOneHalf,
        Self::StepOneThird,
        Self::StepOneQuarter,
        Self::StepOneSixth,
        Self::StepOneEighth,
        Self::StepOneSixteenth,
    ];

    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Steps32,
            1 => Self::Steps16,
            2 => Self::Steps8,
            3 => Self::Steps6,
            4 => Self::Steps4,
            5 => Self::Steps3,
            6 => Self::Steps2,
            7 => Self::StepsOneAndHalf,
            9 => Self::StepTwoThirds,
            10 => Self::StepOneHalf,
            11 => Self::StepOneThird,
            12 => Self::StepOneQuarter,
            13 => Self::StepOneSixth,
            14 => Self::StepOneEighth,
            15 => Self::StepOneSixteenth,
            _ => Self::Step1,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Steps32 => 0,
            Self::Steps16 => 1,
            Self::Steps8 => 2,
            Self::Steps6 => 3,
            Self::Steps4 => 4,
            Self::Steps3 => 5,
            Self::Steps2 => 6,
            Self::StepsOneAndHalf => 7,
            Self::Step1 => 8,
            Self::StepTwoThirds => 9,
            Self::StepOneHalf => 10,
            Self::StepOneThird => 11,
            Self::StepOneQuarter => 12,
            Self::StepOneSixth => 13,
            Self::StepOneEighth => 14,
            Self::StepOneSixteenth => 15,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Steps32 => "32 Steps",
            Self::Steps16 => "16 Steps",
            Self::Steps8 => "8 Steps",
            Self::Steps6 => "6 Steps",
            Self::Steps4 => "4 Steps",
            Self::Steps3 => "3 Steps",
            Self::Steps2 => "2 Steps",
            Self::StepsOneAndHalf => "1.5 Steps",
            Self::Step1 => "1 Step",
            Self::StepTwoThirds => "2/3 Step",
            Self::StepOneHalf => "1/2 Step",
            Self::StepOneThird => "1/3 Step",
            Self::StepOneQuarter => "1/4 Step",
            Self::StepOneSixth => "1/6 Step",
            Self::StepOneEighth => "1/8 Step",
            Self::StepOneSixteenth => "1/16 Step",
        }
    }

    pub const fn cycles_per_step(self) -> f32 {
        match self {
            Self::Steps32 => 1.0 / 32.0,
            Self::Steps16 => 1.0 / 16.0,
            Self::Steps8 => 1.0 / 8.0,
            Self::Steps6 => 1.0 / 6.0,
            Self::Steps4 => 1.0 / 4.0,
            Self::Steps3 => 1.0 / 3.0,
            Self::Steps2 => 1.0 / 2.0,
            Self::StepsOneAndHalf => 2.0 / 3.0,
            Self::Step1 => 1.0,
            Self::StepTwoThirds => 3.0 / 2.0,
            Self::StepOneHalf => 2.0,
            Self::StepOneThird => 3.0,
            Self::StepOneQuarter => 4.0,
            Self::StepOneSixth => 6.0,
            Self::StepOneEighth => 8.0,
            Self::StepOneSixteenth => 16.0,
        }
    }

    pub fn rate_hz(self, bpm: f32, clock_division: ClockDivision) -> f32 {
        bpm.clamp(30.0, 250.0) / 60.0 * clock_division.steps_per_quarter() * self.cycles_per_step()
    }

    pub const fn from_rev2_raw(raw: u16) -> Self {
        Self::from_index(if raw >= 128 { 15 } else { raw as usize / 8 })
    }

    pub const fn rev2_raw(self) -> u16 {
        self.index() as u16 * 8
    }

    pub const fn from_p08_raw(raw: u16) -> Self {
        let index = raw.saturating_sub(151);
        Self::from_index(if index > 15 { 15 } else { index } as usize)
    }

    pub const fn p08_raw(self) -> u16 {
        151 + self.index() as u16
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct LfoParams {
    pub rate_hz: f32,
    pub sync_division: LfoSyncDivision,
    pub depth: f32,
    pub waveform: LfoWaveform,
    pub destination: ModDestination,
    pub clock_sync: bool,
    pub key_sync: bool,
}

impl Default for LfoParams {
    fn default() -> Self {
        Self {
            rate_hz: MIN_LFO_RATE_HZ,
            sync_division: LfoSyncDivision::default(),
            depth: 0.0,
            waveform: LfoWaveform::Triangle,
            destination: ModDestination::Off,
            clock_sync: false,
            key_sync: true,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct AuxEnvelopeParams {
    pub destination: ModDestination,
    pub amount: f32,
    pub velocity: f32,
    pub delay: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub repeat: bool,
}

impl Default for AuxEnvelopeParams {
    fn default() -> Self {
        Self {
            destination: ModDestination::Off,
            amount: 0.0,
            velocity: 0.0,
            delay: 0.0,
            attack: DEFAULT_ATTACK_SECONDS,
            decay: DEFAULT_DECAY_SECONDS,
            sustain: DEFAULT_SUSTAIN_LEVEL,
            release: DEFAULT_RELEASE_SECONDS,
            repeat: false,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct FilterParams {
    pub cutoff: f32,
    pub resonance: f32,
    pub poles: u8,
    pub key_track: f32,
    pub env_amount: f32,
    pub velocity: f32,
    pub audio_mod: f32,
    pub eg_delay: f32,
    pub eg_attack: f32,
    pub eg_decay: f32,
    pub eg_sustain: f32,
    pub eg_release: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            cutoff: 1.0,
            resonance: 0.0,
            poles: 4,
            key_track: 0.0,
            env_amount: 0.0,
            velocity: 0.0,
            audio_mod: 0.0,
            eg_delay: 0.0,
            eg_attack: DEFAULT_ATTACK_SECONDS,
            eg_decay: DEFAULT_DECAY_SECONDS,
            eg_sustain: DEFAULT_SUSTAIN_LEVEL,
            eg_release: DEFAULT_RELEASE_SECONDS,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanModMode {
    /// Pan modulation changes the width of the per-voice spread pattern.
    #[default]
    Alternate,
    /// Pan modulation moves the complete program left or right.
    Fixed,
}

impl PanModMode {
    pub const fn from_param(value: f32) -> Self {
        if value >= 0.5 {
            Self::Fixed
        } else {
            Self::Alternate
        }
    }

    pub const fn as_param(self) -> f32 {
        match self {
            Self::Alternate => 0.0,
            Self::Fixed => 1.0,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct AmplifierParams {
    pub pan_spread: f32,
    pub pan_mod_mode: PanModMode,
    pub initial_level: f32,
    pub env_amount: f32,
    pub velocity: f32,
    pub eg_delay: f32,
    pub eg_attack: f32,
    pub eg_decay: f32,
    pub eg_sustain: f32,
    pub eg_release: f32,
}

impl Default for AmplifierParams {
    fn default() -> Self {
        Self {
            pan_spread: 0.0,
            pan_mod_mode: PanModMode::Alternate,
            initial_level: 0.0,
            env_amount: 1.0,
            velocity: 1.0,
            eg_delay: 0.0,
            eg_attack: DEFAULT_ATTACK_SECONDS,
            eg_decay: DEFAULT_DECAY_SECONDS,
            eg_sustain: DEFAULT_SUSTAIN_LEVEL,
            eg_release: DEFAULT_RELEASE_SECONDS,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlideMode {
    #[default]
    FixedRate,
    FixedRateAuto,
    FixedTime,
    FixedTimeAuto,
}

impl GlideMode {
    pub const ALL: [Self; 4] = [
        Self::FixedRate,
        Self::FixedRateAuto,
        Self::FixedTime,
        Self::FixedTimeAuto,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    pub const fn is_fixed_time(self) -> bool {
        matches!(self, Self::FixedTime | Self::FixedTimeAuto)
    }

    pub const fn is_auto(self) -> bool {
        matches!(self, Self::FixedRateAuto | Self::FixedTimeAuto)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::FixedRate => "Fixed Rate",
            Self::FixedRateAuto => "Fixed Rate Auto",
            Self::FixedTime => "Fixed Time",
            Self::FixedTimeAuto => "Fixed Time Auto",
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMode {
    #[default]
    Low,
    LowRetrigger,
    High,
    HighRetrigger,
    Last,
    LastRetrigger,
}

impl KeyMode {
    pub const ALL: [Self; 6] = [
        Self::Low,
        Self::LowRetrigger,
        Self::High,
        Self::HighRetrigger,
        Self::Last,
        Self::LastRetrigger,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low Note",
            Self::LowRetrigger => "Low / Retrig",
            Self::High => "High Note",
            Self::HighRetrigger => "High / Retrig",
            Self::Last => "Last Note",
            Self::LastRetrigger => "Last / Retrig",
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnisonMode {
    #[default]
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    V9,
    V10,
    V11,
    V12,
    V13,
    V14,
    V15,
    V16,
    Chord,
}

impl UnisonMode {
    pub const ALL: [Self; 17] = [
        Self::V1,
        Self::V2,
        Self::V3,
        Self::V4,
        Self::V5,
        Self::V6,
        Self::V7,
        Self::V8,
        Self::V9,
        Self::V10,
        Self::V11,
        Self::V12,
        Self::V13,
        Self::V14,
        Self::V15,
        Self::V16,
        Self::Chord,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::V1 => "1 Voice",
            Self::V2 => "2 Voices",
            Self::V3 => "3 Voices",
            Self::V4 => "4 Voices",
            Self::V5 => "5 Voices",
            Self::V6 => "6 Voices",
            Self::V7 => "7 Voices",
            Self::V8 => "8 Voices",
            Self::V9 => "9 Voices",
            Self::V10 => "10 Voices",
            Self::V11 => "11 Voices",
            Self::V12 => "12 Voices",
            Self::V13 => "13 Voices",
            Self::V14 => "14 Voices",
            Self::V15 => "15 Voices",
            Self::V16 => "16 Voices",
            Self::Chord => "Chord",
        }
    }

    pub fn voice_count(self) -> Option<usize> {
        (self != Self::Chord).then_some(self.index() + 1)
    }
}

/// Rev2 chord-memory voicing stored as ascending intervals from its lowest note.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordMemory {
    intervals: [u8; CHORD_MEMORY_CAPACITY],
    len: u8,
}

impl Default for ChordMemory {
    fn default() -> Self {
        Self {
            intervals: [0; CHORD_MEMORY_CAPACITY],
            len: 0,
        }
    }
}

impl ChordMemory {
    /// Build chord memory from root-relative semitone intervals.
    pub fn from_intervals(intervals: &[u8]) -> Self {
        let mut memory = Self::default();
        let count = intervals.len().min(CHORD_MEMORY_CAPACITY);
        memory.intervals[..count].copy_from_slice(&intervals[..count]);
        memory.len = count as u8;
        memory
    }

    pub fn from_notes(notes: impl IntoIterator<Item = u8>) -> Self {
        let mut present = [false; 128];
        for note in notes {
            if note < 128 {
                present[usize::from(note)] = true;
            }
        }
        let Some(root) = present.iter().position(|held| *held) else {
            return Self::default();
        };
        let mut memory = Self::default();
        for (note, held) in present.iter().copied().enumerate().skip(root) {
            if held && memory.len() < CHORD_MEMORY_CAPACITY {
                memory.intervals[usize::from(memory.len)] = (note - root) as u8;
                memory.len += 1;
            }
        }
        memory
    }

    pub fn intervals(&self) -> &[u8] {
        &self.intervals[..self.len()]
    }

    pub fn len(&self) -> usize {
        usize::from(self.len).min(CHORD_MEMORY_CAPACITY)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct OscillatorPatch {
    pub waveform: u8,
    pub enabled: bool,
    pub frequency: f32,
    pub fine_tune: f32,
    #[cfg_attr(feature = "serde", serde(alias = "shape"))]
    pub shape_mod: f32,
    pub level: f32,
    pub note_reset: bool,
    pub keyboard_on: bool,
    pub glide: f32,
}

impl Default for OscillatorPatch {
    fn default() -> Self {
        Self {
            waveform: 0,
            enabled: false,
            frequency: 24.0,
            fine_tune: 0.0,
            shape_mod: 0.0,
            level: 1.0,
            note_reset: true,
            keyboard_on: true,
            glide: 0.0,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArpMode {
    #[default]
    Up,
    Down,
    UpDown,
    Assign,
    Random,
}

impl ArpMode {
    pub const ALL: [Self; 5] = [
        Self::Up,
        Self::Down,
        Self::UpDown,
        Self::Random,
        Self::Assign,
    ];

    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Down,
            2 => Self::UpDown,
            3 => Self::Assign,
            4 => Self::Random,
            _ => Self::Up,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Up => 0,
            Self::Down => 1,
            Self::UpDown => 2,
            Self::Assign => 3,
            Self::Random => 4,
        }
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArpSustainMode {
    ArpHold,
    #[default]
    Sustain,
    ArpHoldMom,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct ArpParams {
    pub enabled: bool,
    pub mode: ArpMode,
    pub range: u8,
    pub repeats: u8,
    pub relatch: bool,
    pub hold: bool,
    pub beat_sync: bool,
    pub sustain_mode: ArpSustainMode,
}

impl Default for ArpParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ArpMode::Up,
            range: 1,
            repeats: 1,
            relatch: false,
            hold: false,
            beat_sync: false,
            sustain_mode: ArpSustainMode::default(),
        }
    }
}

/// Complete synthesizer patch capturing every parameter in one serializable snapshot.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct Patch {
    pub osc1: OscillatorPatch,
    pub osc2: OscillatorPatch,
    pub osc_mix: f32,
    pub sub_osc_level: f32,
    pub noise_level: f32,
    pub hard_sync: bool,
    pub osc_slop: f32,
    pub glide_mode: GlideMode,
    pub glide_enabled: bool,
    pub pitch_bend_range: f32,
    pub key_mode: KeyMode,
    pub unison_enabled: bool,
    pub unison_mode: UnisonMode,
    pub unison_detune: f32,
    #[cfg_attr(feature = "serde", serde(default))]
    pub unison_chord: ChordMemory,
    pub bpm: f32,
    pub clock_divide: ClockDivision,
    pub filter: FilterParams,
    pub amplifier: AmplifierParams,
    pub aux_envelope: AuxEnvelopeParams,
    pub lfos: [LfoParams; LFO_COUNT],
    pub mod_matrix: ModMatrix,
    pub effects: EffectParams,
    #[cfg_attr(feature = "serde", serde(default))]
    pub arp: ArpParams,
    pub master_volume: f32,
    pub name: PatchName,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            osc1: OscillatorPatch {
                enabled: true,
                ..OscillatorPatch::default()
            },
            osc2: OscillatorPatch::default(),
            osc_mix: 0.0,
            sub_osc_level: 0.0,
            noise_level: 0.0,
            hard_sync: false,
            osc_slop: 0.0,
            glide_mode: GlideMode::default(),
            glide_enabled: false,
            pitch_bend_range: 2.0,
            key_mode: KeyMode::default(),
            unison_enabled: false,
            unison_mode: UnisonMode::default(),
            unison_detune: 0.0,
            unison_chord: ChordMemory::default(),
            bpm: crate::DEFAULT_TEMPO_BPM,
            clock_divide: ClockDivision::default(),
            filter: FilterParams {
                cutoff: 20_000.0,
                ..FilterParams::default()
            },
            amplifier: AmplifierParams::default(),
            aux_envelope: AuxEnvelopeParams::default(),
            lfos: [LfoParams::default(); LFO_COUNT],
            mod_matrix: ModMatrix::default(),
            effects: EffectParams::default(),
            arp: ArpParams::default(),
            master_volume: 0.8,
            name: PatchName::new(),
        }
    }
}

impl Patch {
    fn bool_f32(b: bool) -> f32 {
        if b { 1.0 } else { 0.0 }
    }

    fn lfo_waveform_index(lfo_waveform: LfoWaveform) -> f32 {
        lfo_waveform.index() as f32
    }

    /// Calls `f` once per parameter with the corresponding [`ParamId`] and value
    /// formatted for [`ControlMessage::SetParam`].
    pub fn for_each_param(&self, mut f: impl FnMut(ParamId, f32)) {
        let s = Self::bool_f32;
        let wi = Self::lfo_waveform_index;

        f(ParamId::Osc1Waveform, self.osc1.waveform as f32);
        f(ParamId::Osc1Enabled, s(self.osc1.enabled));
        f(ParamId::Osc1Frequency, self.osc1.frequency);
        f(ParamId::Osc1FineTune, self.osc1.fine_tune);
        f(ParamId::Osc1ShapeMod, self.osc1.shape_mod);
        f(ParamId::Osc1Level, self.osc1.level);
        f(ParamId::Osc1NoteReset, s(self.osc1.note_reset));
        f(ParamId::Osc1KeyboardOn, s(self.osc1.keyboard_on));
        f(ParamId::Osc1Glide, self.osc1.glide);

        f(ParamId::Osc2Waveform, self.osc2.waveform as f32);
        f(ParamId::Osc2Enabled, s(self.osc2.enabled));
        f(ParamId::Osc2Frequency, self.osc2.frequency);
        f(ParamId::Osc2FineTune, self.osc2.fine_tune);
        f(ParamId::Osc2ShapeMod, self.osc2.shape_mod);
        f(ParamId::Osc2Level, self.osc2.level);
        f(ParamId::Osc2NoteReset, s(self.osc2.note_reset));
        f(ParamId::Osc2KeyboardOn, s(self.osc2.keyboard_on));
        f(ParamId::Osc2Glide, self.osc2.glide);

        f(ParamId::OscMix, self.osc_mix);
        f(ParamId::SubOscLevel, self.sub_osc_level);
        f(ParamId::NoiseLevel, self.noise_level);
        f(ParamId::HardSync, s(self.hard_sync));
        f(ParamId::OscSlop, self.osc_slop);
        f(ParamId::GlideMode, self.glide_mode.index() as f32);
        f(ParamId::GlideEnabled, s(self.glide_enabled));
        f(ParamId::PitchBendRange, self.pitch_bend_range);
        f(ParamId::KeyMode, self.key_mode.index() as f32);
        f(ParamId::UnisonEnabled, s(self.unison_enabled));
        f(ParamId::UnisonMode, self.unison_mode.index() as f32);
        f(ParamId::UnisonDetune, self.unison_detune);
        f(ParamId::Bpm, self.bpm);
        f(ParamId::ClockDivide, self.clock_divide.index() as f32);

        f(ParamId::FilterCutoff, self.filter.cutoff);
        f(ParamId::FilterResonance, self.filter.resonance);
        f(
            ParamId::FilterPoles,
            if self.filter.poles <= 2 { 0.0 } else { 1.0 },
        );
        f(ParamId::FilterKeyTrack, self.filter.key_track);
        f(ParamId::FilterEnvAmount, self.filter.env_amount);
        f(ParamId::FilterVelocity, self.filter.velocity);
        f(ParamId::FilterAudioMod, self.filter.audio_mod);
        f(ParamId::FilterEgDelay, self.filter.eg_delay);
        f(ParamId::FilterEgAttack, self.filter.eg_attack);
        f(ParamId::FilterEgDecay, self.filter.eg_decay);
        f(ParamId::FilterEgSustain, self.filter.eg_sustain);
        f(ParamId::FilterEgRelease, self.filter.eg_release);

        f(ParamId::PanSpread, self.amplifier.pan_spread);
        f(ParamId::PanModMode, self.amplifier.pan_mod_mode.as_param());
        f(ParamId::VcaInitialLevel, self.amplifier.initial_level);
        f(ParamId::AmpEnvAmount, self.amplifier.env_amount);
        f(ParamId::AmpVelocity, self.amplifier.velocity);
        f(ParamId::AmpEgDelay, self.amplifier.eg_delay);
        f(ParamId::AmpEgAttack, self.amplifier.eg_attack);
        f(ParamId::AmpEgDecay, self.amplifier.eg_decay);
        f(ParamId::AmpEgSustain, self.amplifier.eg_sustain);
        f(ParamId::AmpEgRelease, self.amplifier.eg_release);

        f(
            ParamId::AuxEgDestination,
            self.aux_envelope.destination.index() as f32,
        );
        f(ParamId::AuxEgAmount, self.aux_envelope.amount);
        f(ParamId::AuxEgVelocity, self.aux_envelope.velocity);
        f(ParamId::AuxEgDelay, self.aux_envelope.delay);
        f(ParamId::AuxEgAttack, self.aux_envelope.attack);
        f(ParamId::AuxEgDecay, self.aux_envelope.decay);
        f(ParamId::AuxEgSustain, self.aux_envelope.sustain);
        f(ParamId::AuxEgRelease, self.aux_envelope.release);
        f(ParamId::AuxEgLoop, s(self.aux_envelope.repeat));

        let rate = [
            ParamId::Lfo1Rate,
            ParamId::Lfo2Rate,
            ParamId::Lfo3Rate,
            ParamId::Lfo4Rate,
        ];
        let depth = [
            ParamId::Lfo1Depth,
            ParamId::Lfo2Depth,
            ParamId::Lfo3Depth,
            ParamId::Lfo4Depth,
        ];
        let waveform = [
            ParamId::Lfo1Waveform,
            ParamId::Lfo2Waveform,
            ParamId::Lfo3Waveform,
            ParamId::Lfo4Waveform,
        ];
        let destination = [
            ParamId::Lfo1Destination,
            ParamId::Lfo2Destination,
            ParamId::Lfo3Destination,
            ParamId::Lfo4Destination,
        ];
        let clock = [
            ParamId::Lfo1ClockSync,
            ParamId::Lfo2ClockSync,
            ParamId::Lfo3ClockSync,
            ParamId::Lfo4ClockSync,
        ];
        let sync_division = [
            ParamId::Lfo1SyncDivision,
            ParamId::Lfo2SyncDivision,
            ParamId::Lfo3SyncDivision,
            ParamId::Lfo4SyncDivision,
        ];
        let key = [
            ParamId::Lfo1KeySync,
            ParamId::Lfo2KeySync,
            ParamId::Lfo3KeySync,
            ParamId::Lfo4KeySync,
        ];

        for i in 0..4 {
            let lfo = &self.lfos[i];
            f(rate[i], lfo.rate_hz);
            f(depth[i], lfo.depth);
            f(waveform[i], wi(lfo.waveform));
            f(destination[i], lfo.destination.index() as f32);
            f(clock[i], s(lfo.clock_sync));
            f(sync_division[i], lfo.sync_division.index() as f32);
            f(key[i], s(lfo.key_sync));
        }

        f(ParamId::EffectEnabled, s(self.effects.enabled));
        f(ParamId::EffectType, self.effects.effect_type.index() as f32);
        f(ParamId::EffectMix, self.effects.mix);
        f(ParamId::EffectClockSync, s(self.effects.clock_sync));
        f(ParamId::EffectParam1, self.effects.param1);
        f(ParamId::EffectParam2, self.effects.param2);

        f(ParamId::ArpEnabled, s(self.arp.enabled));
        f(ParamId::ArpMode, self.arp.mode.index() as f32);
        f(ParamId::ArpRange, (self.arp.range.saturating_sub(1)) as f32);
        f(
            ParamId::ArpRepeats,
            (self.arp.repeats.saturating_sub(1)) as f32,
        );
        f(ParamId::ArpRelatch, s(self.arp.relatch));
        f(ParamId::ArpHold, s(self.arp.hold));
        f(ParamId::ArpBeatSync, s(self.arp.beat_sync));
        f(
            ParamId::ArpSustainMode,
            match self.arp.sustain_mode {
                ArpSustainMode::ArpHold => 0.0,
                ArpSustainMode::Sustain => 1.0,
                ArpSustainMode::ArpHoldMom => 2.0,
            },
        );

        f(ParamId::MasterVolume, self.master_volume);
        f(ParamId::AnalogDrift, self.osc_slop);
    }

    pub fn for_each_modulation(&self, mut f: impl FnMut(ModRoute, ModMatrixSlot)) {
        for (index, slot) in self.mod_matrix.free_slots.iter().copied().enumerate() {
            f(ModRoute::Free(index), slot);
        }

        for (index, slot) in self.mod_matrix.dedicated.iter().copied().enumerate() {
            let source = DedicatedModSource::ALL[index];
            f(
                ModRoute::Dedicated(source),
                ModMatrixSlot {
                    enabled: slot.enabled,
                    source: source.source(),
                    destination: slot.destination,
                    amount: slot.amount,
                },
            );
        }
    }

    /// Updates one patch parameter using the same normalization as MIDI and UI hosts.
    pub fn set_param(&mut self, id: ParamId, value: f32) {
        let flag = value >= 0.5;
        match id {
            ParamId::Osc1Waveform => self.osc1.waveform = value as u8,
            ParamId::Osc1Enabled => self.osc1.enabled = flag,
            ParamId::Osc1Frequency => self.osc1.frequency = value,
            ParamId::Osc1FineTune => self.osc1.fine_tune = value,
            ParamId::Osc1ShapeMod => self.osc1.shape_mod = value,
            ParamId::Osc1Level => self.osc1.level = value,
            ParamId::Osc1NoteReset => self.osc1.note_reset = flag,
            ParamId::Osc1KeyboardOn => self.osc1.keyboard_on = flag,
            ParamId::Osc1Glide => self.osc1.glide = value,
            ParamId::Osc2Waveform => self.osc2.waveform = value as u8,
            ParamId::Osc2Enabled => self.osc2.enabled = flag,
            ParamId::Osc2Frequency => self.osc2.frequency = value,
            ParamId::Osc2FineTune => self.osc2.fine_tune = value,
            ParamId::Osc2ShapeMod => self.osc2.shape_mod = value,
            ParamId::Osc2Level => self.osc2.level = value,
            ParamId::Osc2NoteReset => self.osc2.note_reset = flag,
            ParamId::Osc2KeyboardOn => self.osc2.keyboard_on = flag,
            ParamId::Osc2Glide => self.osc2.glide = value,
            ParamId::OscMix => self.osc_mix = value,
            ParamId::SubOscLevel => self.sub_osc_level = value,
            ParamId::NoiseLevel => self.noise_level = value,
            ParamId::HardSync => self.hard_sync = flag,
            ParamId::OscSlop | ParamId::AnalogDrift => self.osc_slop = value,
            ParamId::GlideMode => self.glide_mode = GlideMode::from_index(value as usize),
            ParamId::GlideEnabled => self.glide_enabled = flag,
            ParamId::PitchBendRange => self.pitch_bend_range = value.clamp(0.0, 12.0),
            ParamId::KeyMode => self.key_mode = KeyMode::from_index(value as usize),
            ParamId::UnisonEnabled => self.unison_enabled = flag,
            ParamId::UnisonMode => self.unison_mode = UnisonMode::from_index(value as usize),
            ParamId::UnisonDetune => self.unison_detune = value.clamp(0.0, 16.0),
            ParamId::Bpm => self.bpm = value.clamp(30.0, 250.0),
            ParamId::ClockDivide => self.clock_divide = ClockDivision::from_index(value as usize),
            ParamId::FilterCutoff => self.filter.cutoff = value,
            ParamId::FilterResonance => self.filter.resonance = value,
            ParamId::FilterPoles => self.filter.poles = if flag { 4 } else { 2 },
            ParamId::FilterKeyTrack => self.filter.key_track = value,
            ParamId::FilterEnvAmount => self.filter.env_amount = value,
            ParamId::FilterVelocity => self.filter.velocity = value,
            ParamId::FilterAudioMod => self.filter.audio_mod = value,
            ParamId::FilterEgDelay => self.filter.eg_delay = value,
            ParamId::FilterEgAttack => self.filter.eg_attack = value,
            ParamId::FilterEgDecay => self.filter.eg_decay = value,
            ParamId::FilterEgSustain => self.filter.eg_sustain = value,
            ParamId::FilterEgRelease => self.filter.eg_release = value,
            ParamId::PanSpread => self.amplifier.pan_spread = value,
            ParamId::PanModMode => self.amplifier.pan_mod_mode = PanModMode::from_param(value),
            ParamId::VcaInitialLevel => self.amplifier.initial_level = value.clamp(0.0, 1.0),
            ParamId::AmpEnvAmount => self.amplifier.env_amount = value,
            ParamId::AmpVelocity => self.amplifier.velocity = value,
            ParamId::AmpEgDelay => self.amplifier.eg_delay = value,
            ParamId::AmpEgAttack => self.amplifier.eg_attack = value,
            ParamId::AmpEgDecay => self.amplifier.eg_decay = value,
            ParamId::AmpEgSustain => self.amplifier.eg_sustain = value,
            ParamId::AmpEgRelease => self.amplifier.eg_release = value,
            ParamId::AuxEgDestination => {
                self.aux_envelope.destination = ModDestination::from_index(value as usize)
            }
            ParamId::AuxEgAmount => self.aux_envelope.amount = value,
            ParamId::AuxEgVelocity => self.aux_envelope.velocity = value,
            ParamId::AuxEgDelay => self.aux_envelope.delay = value,
            ParamId::AuxEgAttack => self.aux_envelope.attack = value,
            ParamId::AuxEgDecay => self.aux_envelope.decay = value,
            ParamId::AuxEgSustain => self.aux_envelope.sustain = value,
            ParamId::AuxEgRelease => self.aux_envelope.release = value,
            ParamId::AuxEgLoop => self.aux_envelope.repeat = flag,
            ParamId::Lfo1Rate => self.lfos[0].rate_hz = value,
            ParamId::Lfo1Depth => self.lfos[0].depth = value,
            ParamId::Lfo1Waveform => {
                self.lfos[0].waveform = LfoWaveform::from_index(value as usize)
            }
            ParamId::Lfo1Destination => {
                self.lfos[0].destination = ModDestination::from_index(value as usize)
            }
            ParamId::Lfo1ClockSync => self.lfos[0].clock_sync = flag,
            ParamId::Lfo1SyncDivision => {
                self.lfos[0].sync_division = LfoSyncDivision::from_index(value as usize)
            }
            ParamId::Lfo1KeySync => self.lfos[0].key_sync = flag,
            ParamId::Lfo2Rate => self.lfos[1].rate_hz = value,
            ParamId::Lfo2Depth => self.lfos[1].depth = value,
            ParamId::Lfo2Waveform => {
                self.lfos[1].waveform = LfoWaveform::from_index(value as usize)
            }
            ParamId::Lfo2Destination => {
                self.lfos[1].destination = ModDestination::from_index(value as usize)
            }
            ParamId::Lfo2ClockSync => self.lfos[1].clock_sync = flag,
            ParamId::Lfo2SyncDivision => {
                self.lfos[1].sync_division = LfoSyncDivision::from_index(value as usize)
            }
            ParamId::Lfo2KeySync => self.lfos[1].key_sync = flag,
            ParamId::Lfo3Rate => self.lfos[2].rate_hz = value,
            ParamId::Lfo3Depth => self.lfos[2].depth = value,
            ParamId::Lfo3Waveform => {
                self.lfos[2].waveform = LfoWaveform::from_index(value as usize)
            }
            ParamId::Lfo3Destination => {
                self.lfos[2].destination = ModDestination::from_index(value as usize)
            }
            ParamId::Lfo3ClockSync => self.lfos[2].clock_sync = flag,
            ParamId::Lfo3SyncDivision => {
                self.lfos[2].sync_division = LfoSyncDivision::from_index(value as usize)
            }
            ParamId::Lfo3KeySync => self.lfos[2].key_sync = flag,
            ParamId::Lfo4Rate => self.lfos[3].rate_hz = value,
            ParamId::Lfo4Depth => self.lfos[3].depth = value,
            ParamId::Lfo4Waveform => {
                self.lfos[3].waveform = LfoWaveform::from_index(value as usize)
            }
            ParamId::Lfo4Destination => {
                self.lfos[3].destination = ModDestination::from_index(value as usize)
            }
            ParamId::Lfo4ClockSync => self.lfos[3].clock_sync = flag,
            ParamId::Lfo4SyncDivision => {
                self.lfos[3].sync_division = LfoSyncDivision::from_index(value as usize)
            }
            ParamId::Lfo4KeySync => self.lfos[3].key_sync = flag,
            ParamId::EffectEnabled => self.effects.enabled = flag,
            ParamId::EffectType => {
                self.effects.effect_type = EffectType::from_index(value as usize)
            }
            ParamId::EffectMix => self.effects.mix = value,
            ParamId::EffectClockSync => self.effects.clock_sync = flag,
            ParamId::EffectParam1 => self.effects.param1 = value,
            ParamId::EffectParam2 => self.effects.param2 = value,
            ParamId::MasterVolume => self.master_volume = value,
            ParamId::ArpEnabled => self.arp.enabled = flag,
            ParamId::ArpMode => self.arp.mode = ArpMode::from_index(value as usize),
            ParamId::ArpRange => self.arp.range = (value as u8).clamp(0, 2) + 1,
            ParamId::ArpRepeats => self.arp.repeats = (value as u8).clamp(0, 2) + 1,
            ParamId::ArpRelatch => self.arp.relatch = flag,
            ParamId::ArpHold => self.arp.hold = flag,
            ParamId::ArpBeatSync => self.arp.beat_sync = flag,
            ParamId::ArpSustainMode => {
                self.arp.sustain_mode = match value as usize {
                    0 => ArpSustainMode::ArpHold,
                    2 => ArpSustainMode::ArpHoldMom,
                    _ => ArpSustainMode::Sustain,
                }
            }
            ParamId::VcaDrive => {}
        }
    }

    pub(crate) fn set_modulation_param(
        &mut self,
        route: ModRoute,
        parameter: crate::ModulationParam,
    ) {
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = self.mod_matrix.free_slots.get_mut(index) {
                    match parameter {
                        crate::ModulationParam::Source(source) => slot.source = source,
                        crate::ModulationParam::Destination(destination) => {
                            slot.destination = destination
                        }
                        crate::ModulationParam::Amount(amount) => slot.amount = amount,
                    }
                    slot.enabled =
                        slot.source != ModSource::Off && slot.destination != ModDestination::Off;
                }
            }
            ModRoute::Dedicated(source) => {
                if let Some(slot) = self.mod_matrix.dedicated.get_mut(source.index()) {
                    match parameter {
                        crate::ModulationParam::Destination(destination) => {
                            slot.destination = destination
                        }
                        crate::ModulationParam::Amount(amount) => slot.amount = amount,
                        crate::ModulationParam::Source(_) => {}
                    }
                    slot.enabled = slot.destination != ModDestination::Off;
                }
            }
        }
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn patch_name_round_trips_through_serde() {
        let mut patch = Patch::default();
        patch.name.push_str("LosVangelis2041").unwrap();
        let encoded = serde_json::to_value(&patch).unwrap();
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "LosVangelis2041");
    }

    #[test]
    fn obsolete_glide_time_field_is_ignored_when_loading_old_patches() {
        let mut encoded = serde_json::to_value(Patch::default()).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert("glide_time".into(), serde_json::json!(8.0));

        let decoded: Patch = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded.osc1.glide, 0.0);
        assert_eq!(decoded.osc2.glide, 0.0);
    }

    #[test]
    fn clock_divisions_have_stable_indices_and_nominal_rates() {
        let expected = [
            0.5, 1.0, 2.0, 2.0, 2.0, 3.0, 4.0, 4.0, 4.0, 6.0, 8.0, 12.0, 16.0,
        ];
        for (index, division) in ClockDivision::ALL.iter().copied().enumerate() {
            assert_eq!(division.index(), index);
            assert_eq!(ClockDivision::from_index(index), division);
            assert_eq!(division.steps_per_quarter(), expected[index]);
        }
    }

    #[test]
    fn lfo_sync_divisions_have_stable_hardware_ratios() {
        let expected = [
            1.0 / 32.0,
            1.0 / 16.0,
            1.0 / 8.0,
            1.0 / 6.0,
            1.0 / 4.0,
            1.0 / 3.0,
            1.0 / 2.0,
            2.0 / 3.0,
            1.0,
            3.0 / 2.0,
            2.0,
            3.0,
            4.0,
            6.0,
            8.0,
            16.0,
        ];
        for (index, division) in LfoSyncDivision::ALL.iter().copied().enumerate() {
            assert_eq!(division.index(), index);
            assert_eq!(LfoSyncDivision::from_index(index), division);
            assert_eq!(division.cycles_per_step(), expected[index]);
            assert_eq!(LfoSyncDivision::from_rev2_raw((index * 8) as u16), division);
            assert_eq!(LfoSyncDivision::from_p08_raw(151 + index as u16), division);
            assert_eq!(division.rev2_raw(), (index * 8) as u16);
            assert_eq!(division.p08_raw(), 151 + index as u16);
        }
        for raw in 0..=150 {
            assert_eq!(
                LfoSyncDivision::from_rev2_raw(raw),
                LfoSyncDivision::from_index((usize::from(raw) / 8).min(15))
            );
        }
    }

    #[test]
    fn typed_clock_fields_round_trip_through_serde() {
        let mut patch = Patch::default();
        patch.clock_divide = ClockDivision::SixteenthTriplet;
        patch.lfos[2].clock_sync = true;
        patch.lfos[2].sync_division = LfoSyncDivision::StepTwoThirds;
        let encoded = serde_json::to_value(&patch).unwrap();
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.clock_divide, ClockDivision::SixteenthTriplet);
        assert_eq!(
            decoded.lfos[2].sync_division,
            LfoSyncDivision::StepTwoThirds
        );
    }

    #[test]
    fn decode_patch_name_trims_trailing_spaces() {
        let mut bytes = [b' '; PATCH_NAME_CAPACITY];
        bytes[..15].copy_from_slice(b"LosVangelis2041");
        assert_eq!(decode_patch_name(&bytes).as_str(), "LosVangelis2041");
    }

    #[test]
    fn vca_initial_level_round_trips_through_serde() {
        let mut patch = Patch::default();
        patch.amplifier.initial_level = 0.5;
        let encoded = serde_json::to_value(&patch).unwrap();
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.amplifier.initial_level, 0.5);
    }

    #[test]
    fn pan_mod_mode_round_trips_through_serde() {
        let mut patch = Patch::default();
        patch.amplifier.pan_mod_mode = PanModMode::Fixed;
        let encoded = serde_json::to_value(&patch).unwrap();
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.amplifier.pan_mod_mode, PanModMode::Fixed);
    }

    #[test]
    fn chord_memory_preserves_voicing_and_round_trips() {
        let mut patch = Patch::default();
        patch.unison_mode = UnisonMode::Chord;
        patch.unison_chord = ChordMemory::from_notes([64, 67, 72]);
        assert_eq!(patch.unison_chord.intervals(), &[0, 3, 8]);
        let encoded = serde_json::to_value(&patch).unwrap();
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.unison_mode, UnisonMode::Chord);
        assert_eq!(decoded.unison_chord, patch.unison_chord);
    }

    #[test]
    fn old_patch_without_chord_memory_gets_an_empty_default() {
        let patch = Patch::default();
        let mut encoded = serde_json::to_value(&patch).unwrap();
        encoded.as_object_mut().unwrap().remove("unison_chord");
        let decoded: Patch = serde_json::from_value(encoded).unwrap();
        assert!(decoded.unison_chord.is_empty());
    }
}
