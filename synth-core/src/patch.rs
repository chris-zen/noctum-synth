//! Patch parameter bundles and modulation routing targets.

use crate::{
    DEFAULT_ATTACK_SECONDS, DEFAULT_DECAY_SECONDS, DEFAULT_RELEASE_SECONDS, DEFAULT_SUSTAIN_LEVEL,
    LfoWaveform, MIN_LFO_RATE_HZ, ParamId,
};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

/// Target for a modulation route.
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModDestination {
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
    pub const ALL: [Self; 54] = [
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedicatedModSource {
    ModWheel,
    Pressure,
    Breath,
    Velocity,
    Footswitch,
}

impl DedicatedModSource {
    pub const ALL: [Self; 5] = [
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub enum ModRoute {
    Free(usize),
    Dedicated(DedicatedModSource),
}

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct ModMatrix {
    pub free_slots: [ModMatrixSlot; 8],
    pub dedicated: [DedicatedModSlot; 5],
}

impl Default for ModMatrix {
    fn default() -> Self {
        Self {
            free_slots: [ModMatrixSlot::default(); 8],
            dedicated: [DedicatedModSlot::default(); 5],
        }
    }
}

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy)]
pub struct LfoParams {
    pub rate_hz: f32,
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
            depth: 0.0,
            waveform: LfoWaveform::Triangle,
            destination: ModDestination::Off,
            clock_sync: false,
            key_sync: true,
        }
    }
}

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
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

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct OscillatorPatch {
    pub waveform: u8,
    pub enabled: bool,
    pub frequency: f32,
    pub fine_tune: f32,
    pub shape: f32,
    pub level: f32,
    pub note_reset: bool,
    pub keyboard_on: bool,
    pub glide: bool,
}

impl Default for OscillatorPatch {
    fn default() -> Self {
        Self {
            waveform: 0,
            enabled: false,
            frequency: 60.0,
            fine_tune: 0.0,
            shape: 0.0,
            level: 1.0,
            note_reset: true,
            keyboard_on: true,
            glide: false,
        }
    }
}

/// Complete synthesizer patch capturing every parameter in one serializable snapshot.
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct Patch {
    pub osc1: OscillatorPatch,
    pub osc2: OscillatorPatch,
    pub osc_mix: f32,
    pub sub_osc_level: f32,
    pub noise_level: f32,
    pub hard_sync: bool,
    pub osc_slop: f32,
    pub glide_time: f32,
    pub filter: FilterParams,
    pub amplifier: AmplifierParams,
    pub aux_envelope: AuxEnvelopeParams,
    pub lfos: [LfoParams; 4],
    #[cfg_attr(feature = "std", serde(default))]
    pub mod_matrix: ModMatrix,
    pub master_volume: f32,
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
            glide_time: 0.0,
            filter: FilterParams {
                cutoff: 20_000.0,
                ..FilterParams::default()
            },
            amplifier: AmplifierParams::default(),
            aux_envelope: AuxEnvelopeParams::default(),
            lfos: [LfoParams::default(); 4],
            mod_matrix: ModMatrix::default(),
            master_volume: 1.0,
        }
    }
}

impl Patch {
    fn bool_f32(b: bool) -> f32 {
        if b { 1.0 } else { 0.0 }
    }

    fn lfo_waveform_index(w: LfoWaveform) -> f32 {
        match w {
            LfoWaveform::Triangle => 0.0,
            LfoWaveform::Saw => 1.0,
            LfoWaveform::ReverseSaw => 2.0,
            LfoWaveform::Square => 3.0,
            LfoWaveform::SampleAndHold => 4.0,
        }
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
        f(ParamId::Osc1Shape, self.osc1.shape);
        f(ParamId::Osc1Level, self.osc1.level);
        f(ParamId::Osc1NoteReset, s(self.osc1.note_reset));
        f(ParamId::Osc1KeyboardOn, s(self.osc1.keyboard_on));
        f(ParamId::Osc1Glide, s(self.osc1.glide));

        f(ParamId::Osc2Waveform, self.osc2.waveform as f32);
        f(ParamId::Osc2Enabled, s(self.osc2.enabled));
        f(ParamId::Osc2Frequency, self.osc2.frequency);
        f(ParamId::Osc2FineTune, self.osc2.fine_tune);
        f(ParamId::Osc2Shape, self.osc2.shape);
        f(ParamId::Osc2Level, self.osc2.level);
        f(ParamId::Osc2NoteReset, s(self.osc2.note_reset));
        f(ParamId::Osc2KeyboardOn, s(self.osc2.keyboard_on));
        f(ParamId::Osc2Glide, s(self.osc2.glide));

        f(ParamId::OscMix, self.osc_mix);
        f(ParamId::SubOscLevel, self.sub_osc_level);
        f(ParamId::NoiseLevel, self.noise_level);
        f(ParamId::HardSync, s(self.hard_sync));
        f(ParamId::OscSlop, self.osc_slop);
        f(ParamId::GlideTime, self.glide_time);

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
            f(key[i], s(lfo.key_sync));
        }

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
}
