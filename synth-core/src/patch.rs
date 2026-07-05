//! Patch parameter bundles and modulation routing targets.

use crate::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    LfoWaveform, MIN_LFO_RATE_HZ,
};

/// Target for an LFO or auxiliary envelope modulation route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoDestination {
    Off,
    Osc1Frequency,
    Osc2Frequency,
    OscAllFrequency,
    Osc1Level,
    OscMix,
    NoiseLevel,
    SubOscLevel,
    Osc1Shape,
    Osc2Shape,
    OscAllShape,
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
}

impl LfoDestination {
    pub const ALL: [Self; 26] = [
        Self::Off,
        Self::Osc1Frequency,
        Self::Osc2Frequency,
        Self::OscAllFrequency,
        Self::Osc1Level,
        Self::OscMix,
        Self::NoiseLevel,
        Self::SubOscLevel,
        Self::Osc1Shape,
        Self::Osc2Shape,
        Self::OscAllShape,
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
            Self::Osc1Level => "Osc 1 Level",
            Self::OscMix => "Osc Mix",
            Self::NoiseLevel => "Noise Level",
            Self::SubOscLevel => "Sub Osc Level",
            Self::Osc1Shape => "Osc 1 Shape",
            Self::Osc2Shape => "Osc 2 Shape",
            Self::OscAllShape => "Osc All Shape",
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
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LfoParams {
    pub rate_hz: f32,
    pub depth: f32,
    pub waveform: LfoWaveform,
    pub destination: LfoDestination,
    pub clock_sync: bool,
    pub key_sync: bool,
}

impl Default for LfoParams {
    fn default() -> Self {
        Self {
            rate_hz: MIN_LFO_RATE_HZ,
            depth: 0.0,
            waveform: LfoWaveform::Triangle,
            destination: LfoDestination::Off,
            clock_sync: false,
            key_sync: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuxEnvelopeParams {
    pub destination: LfoDestination,
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
            destination: LfoDestination::Off,
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

#[derive(Debug, Clone)]
pub struct AmplifierParams {
    pub pan_spread: f32,
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
