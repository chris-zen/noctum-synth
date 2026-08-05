use crate::{
    ParamId,
    math::WideF32,
    midi::prophet::{
        attack_decay_raw, attack_decay_seconds, release_raw, release_seconds,
    },
    patch::{LayerPatch, MOD_MATRIX_FREE_SLOT_COUNT, ModDestination},
};

pub(crate) const PARAM_ENVELOPE_TIME_RAW_DEPTH: f32 = 127.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParamDestination {
    FilterEnvAmount,
    AmpEnvAmount,
    AuxEnvAmount,
    EnvAllAmount,
    FilterAttack,
    AmpAttack,
    AuxAttack,
    FilterDecay,
    AmpDecay,
    AuxDecay,
    FilterRelease,
    AmpRelease,
    AuxRelease,
    EnvAllAttack,
    EnvAllDecay,
    EnvAllRelease,
    MatrixAmount(u8),
    OscSlop,
}

pub(crate) fn param_destination(destination: ModDestination) -> Option<ParamDestination> {
    match destination {
        ModDestination::LpFilterEnvAmount => Some(ParamDestination::FilterEnvAmount),
        ModDestination::AmpEnvAmount => Some(ParamDestination::AmpEnvAmount),
        ModDestination::Env3Amount => Some(ParamDestination::AuxEnvAmount),
        ModDestination::EnvAllAmount => Some(ParamDestination::EnvAllAmount),
        ModDestination::LpFilterAttack => Some(ParamDestination::FilterAttack),
        ModDestination::VcaAttack => Some(ParamDestination::AmpAttack),
        ModDestination::Env3Attack => Some(ParamDestination::AuxAttack),
        ModDestination::EnvAllAttack => Some(ParamDestination::EnvAllAttack),
        ModDestination::LpFilterDecay => Some(ParamDestination::FilterDecay),
        ModDestination::VcaDecay => Some(ParamDestination::AmpDecay),
        ModDestination::Env3Decay => Some(ParamDestination::AuxDecay),
        ModDestination::EnvAllDecay => Some(ParamDestination::EnvAllDecay),
        ModDestination::LpFilterRelease => Some(ParamDestination::FilterRelease),
        ModDestination::VcaRelease => Some(ParamDestination::AmpRelease),
        ModDestination::Env3Release => Some(ParamDestination::AuxRelease),
        ModDestination::EnvAllRelease => Some(ParamDestination::EnvAllRelease),
        ModDestination::Mod1Amount => Some(ParamDestination::MatrixAmount(0)),
        ModDestination::Mod2Amount => Some(ParamDestination::MatrixAmount(1)),
        ModDestination::Mod3Amount => Some(ParamDestination::MatrixAmount(2)),
        ModDestination::Mod4Amount => Some(ParamDestination::MatrixAmount(3)),
        ModDestination::Mod5Amount => Some(ParamDestination::MatrixAmount(4)),
        ModDestination::Mod6Amount => Some(ParamDestination::MatrixAmount(5)),
        ModDestination::Mod7Amount => Some(ParamDestination::MatrixAmount(6)),
        ModDestination::Mod8Amount => Some(ParamDestination::MatrixAmount(7)),
        ModDestination::OscSlop => Some(ParamDestination::OscSlop),
        _ => None,
    }
}

pub(crate) fn is_param_destination(destination: ModDestination) -> bool {
    param_destination(destination).is_some()
}

pub(crate) fn matrix_amount_slot(destination: ModDestination) -> Option<u8> {
    match param_destination(destination)? {
        ParamDestination::MatrixAmount(slot) => Some(slot),
        _ => None,
    }
}

pub(crate) fn is_modulatable_param(id: ParamId) -> bool {
    matches!(
        id,
        ParamId::FilterEnvAmount
            | ParamId::FilterEgAttack
            | ParamId::FilterEgDecay
            | ParamId::FilterEgRelease
            | ParamId::AmpEnvAmount
            | ParamId::AmpEgAttack
            | ParamId::AmpEgDecay
            | ParamId::AmpEgRelease
            | ParamId::AuxEgAttack
            | ParamId::AuxEgDecay
            | ParamId::AuxEgRelease
            | ParamId::OscSlop
            | ParamId::AnalogDrift
    )
}

#[derive(Clone, Copy)]
pub(crate) struct VoiceParamSnapshot {
    filter_env_amount: f32,
    amp_env_amount: f32,
    filter_attack_raw: u16,
    filter_decay_raw: u16,
    filter_release_raw: u16,
    amp_attack_raw: u16,
    amp_decay_raw: u16,
    amp_release_raw: u16,
    aux_attack_raw: u16,
    aux_decay_raw: u16,
    aux_release_raw: u16,
    osc_slop: f32,
}

impl VoiceParamSnapshot {
    pub fn from_patch(patch: &LayerPatch) -> Self {
        Self {
            filter_env_amount: patch.filter.env_amount,
            amp_env_amount: patch.amplifier.env_amount,
            filter_attack_raw: attack_decay_raw(patch.filter.eg_attack),
            filter_decay_raw: attack_decay_raw(patch.filter.eg_decay),
            filter_release_raw: release_raw(patch.filter.eg_release),
            amp_attack_raw: attack_decay_raw(patch.amplifier.eg_attack),
            amp_decay_raw: attack_decay_raw(patch.amplifier.eg_decay),
            amp_release_raw: release_raw(patch.amplifier.eg_release),
            aux_attack_raw: attack_decay_raw(patch.aux_envelope.attack),
            aux_decay_raw: attack_decay_raw(patch.aux_envelope.decay),
            aux_release_raw: release_raw(patch.aux_envelope.release),
            osc_slop: patch.osc_slop,
        }
    }

    pub fn mirror(&mut self, other: &Self) {
        *self = *other;
    }

    pub fn filter_env_amount(&self) -> f32 {
        self.filter_env_amount
    }

    pub fn set_filter_env_amount(&mut self, amount: f32) {
        self.filter_env_amount = amount;
    }

    pub fn amp_env_amount(&self) -> f32 {
        self.amp_env_amount
    }

    pub fn set_amp_env_amount(&mut self, amount: f32) {
        self.amp_env_amount = amount;
    }

    pub fn osc_slop(&self) -> f32 {
        self.osc_slop
    }

    pub fn set_osc_slop(&mut self, slop: f32) {
        self.osc_slop = slop;
    }

    pub fn set_filter_attack_seconds(&mut self, seconds: f32) {
        self.filter_attack_raw = attack_decay_raw(seconds);
    }

    pub fn set_filter_decay_seconds(&mut self, seconds: f32) {
        self.filter_decay_raw = attack_decay_raw(seconds);
    }

    pub fn set_filter_release_seconds(&mut self, seconds: f32) {
        self.filter_release_raw = release_raw(seconds);
    }

    pub fn set_amp_attack_seconds(&mut self, seconds: f32) {
        self.amp_attack_raw = attack_decay_raw(seconds);
    }

    pub fn set_amp_decay_seconds(&mut self, seconds: f32) {
        self.amp_decay_raw = attack_decay_raw(seconds);
    }

    pub fn set_amp_release_seconds(&mut self, seconds: f32) {
        self.amp_release_raw = release_raw(seconds);
    }

    pub fn set_aux_attack_seconds(&mut self, seconds: f32) {
        self.aux_attack_raw = attack_decay_raw(seconds);
    }

    pub fn set_aux_decay_seconds(&mut self, seconds: f32) {
        self.aux_decay_raw = attack_decay_raw(seconds);
    }

    pub fn set_aux_release_seconds(&mut self, seconds: f32) {
        self.aux_release_raw = release_raw(seconds);
    }

    pub fn aux_attack_raw(&self) -> u16 {
        self.aux_attack_raw
    }
}

impl Default for VoiceParamSnapshot {
    fn default() -> Self {
        Self::from_patch(&LayerPatch::default())
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PreviousParamSignals {
    pub filter_env: WideF32,
    pub amp_env: WideF32,
    pub aux_env: WideF32,
    pub aux_signal: WideF32,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ParamModulation {
    pub filter_env_amount: f32,
    pub amp_env_amount: f32,
    pub aux_env_amount: f32,
    pub env_all_amount: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_release: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_release: f32,
    pub aux_attack: f32,
    pub aux_decay: f32,
    pub aux_release: f32,
    pub env_all_attack: f32,
    pub env_all_decay: f32,
    pub env_all_release: f32,
    pub matrix_amounts: [f32; MOD_MATRIX_FREE_SLOT_COUNT],
    pub matrix_amount_mask: u8,
    pub osc_slop: f32,
}

impl ParamModulation {
    pub fn accumulate(&mut self, destination: ModDestination, signal: f32) {
        match param_destination(destination) {
            Some(ParamDestination::FilterEnvAmount) => self.filter_env_amount += signal,
            Some(ParamDestination::AmpEnvAmount) => self.amp_env_amount += signal,
            Some(ParamDestination::AuxEnvAmount) => self.aux_env_amount += signal,
            Some(ParamDestination::EnvAllAmount) => self.env_all_amount += signal,
            Some(ParamDestination::FilterAttack) => self.filter_attack += signal,
            Some(ParamDestination::AmpAttack) => self.amp_attack += signal,
            Some(ParamDestination::AuxAttack) => self.aux_attack += signal,
            Some(ParamDestination::EnvAllAttack) => self.env_all_attack += signal,
            Some(ParamDestination::FilterDecay) => self.filter_decay += signal,
            Some(ParamDestination::AmpDecay) => self.amp_decay += signal,
            Some(ParamDestination::AuxDecay) => self.aux_decay += signal,
            Some(ParamDestination::EnvAllDecay) => self.env_all_decay += signal,
            Some(ParamDestination::FilterRelease) => self.filter_release += signal,
            Some(ParamDestination::AmpRelease) => self.amp_release += signal,
            Some(ParamDestination::AuxRelease) => self.aux_release += signal,
            Some(ParamDestination::EnvAllRelease) => self.env_all_release += signal,
            Some(ParamDestination::MatrixAmount(slot)) => {
                self.accumulate_matrix_amount(slot as usize, signal)
            }
            Some(ParamDestination::OscSlop) => self.osc_slop += signal,
            None => {}
        }
    }

    fn accumulate_matrix_amount(&mut self, slot: usize, signal: f32) {
        self.matrix_amounts[slot] += signal;
        self.matrix_amount_mask |= 1 << slot;
    }
}

#[derive(Clone, Copy)]
pub(crate) enum EnvelopeTimeTarget {
    FilterAttack,
    FilterDecay,
    FilterRelease,
    AmpAttack,
    AmpDecay,
    AmpRelease,
    AuxAttack,
    AuxDecay,
    AuxRelease,
}

impl EnvelopeTimeTarget {
    const ALL: [Self; 9] = [
        Self::FilterAttack,
        Self::FilterDecay,
        Self::FilterRelease,
        Self::AmpAttack,
        Self::AmpDecay,
        Self::AmpRelease,
        Self::AuxAttack,
        Self::AuxDecay,
        Self::AuxRelease,
    ];

    fn base_raw(self, bases: &VoiceParamSnapshot) -> u16 {
        match self {
            Self::FilterAttack => bases.filter_attack_raw,
            Self::FilterDecay => bases.filter_decay_raw,
            Self::FilterRelease => bases.filter_release_raw,
            Self::AmpAttack => bases.amp_attack_raw,
            Self::AmpDecay => bases.amp_decay_raw,
            Self::AmpRelease => bases.amp_release_raw,
            Self::AuxAttack => bases.aux_attack_raw,
            Self::AuxDecay => bases.aux_decay_raw,
            Self::AuxRelease => bases.aux_release_raw,
        }
    }

    fn applied_raw_mut(self, applied: &mut VoiceParamSnapshot) -> &mut u16 {
        match self {
            Self::FilterAttack => &mut applied.filter_attack_raw,
            Self::FilterDecay => &mut applied.filter_decay_raw,
            Self::FilterRelease => &mut applied.filter_release_raw,
            Self::AmpAttack => &mut applied.amp_attack_raw,
            Self::AmpDecay => &mut applied.amp_decay_raw,
            Self::AmpRelease => &mut applied.amp_release_raw,
            Self::AuxAttack => &mut applied.aux_attack_raw,
            Self::AuxDecay => &mut applied.aux_decay_raw,
            Self::AuxRelease => &mut applied.aux_release_raw,
        }
    }

    fn deltas(self, param: &ParamModulation) -> (f32, f32) {
        match self {
            Self::FilterAttack => (param.filter_attack, param.env_all_attack),
            Self::FilterDecay => (param.filter_decay, param.env_all_decay),
            Self::FilterRelease => (param.filter_release, param.env_all_release),
            Self::AmpAttack => (param.amp_attack, param.env_all_attack),
            Self::AmpDecay => (param.amp_decay, param.env_all_decay),
            Self::AmpRelease => (param.amp_release, param.env_all_release),
            Self::AuxAttack => (param.aux_attack, param.env_all_attack),
            Self::AuxDecay => (param.aux_decay, param.env_all_decay),
            Self::AuxRelease => (param.aux_release, param.env_all_release),
        }
    }

    fn to_seconds(self, raw: u16) -> f32 {
        match self {
            Self::FilterRelease | Self::AmpRelease | Self::AuxRelease => release_seconds(raw),
            _ => attack_decay_seconds(raw),
        }
    }
}

fn modulated_time_raw(base_raw: u16, specific: f32, all: f32) -> u16 {
    (f32::from(base_raw) + (specific + all) * PARAM_ENVELOPE_TIME_RAW_DEPTH).clamp(0.0, 127.0)
        as u16
}

fn apply_modulated_time(
    base_raw: u16,
    specific: f32,
    all: f32,
    applied_raw: &mut u16,
    target: EnvelopeTimeTarget,
) -> Option<f32> {
    let raw = modulated_time_raw(base_raw, specific, all);
    if raw == *applied_raw {
        return None;
    }
    *applied_raw = raw;
    Some(target.to_seconds(raw))
}

pub(crate) fn apply_envelope_time_modulation(
    bases: &VoiceParamSnapshot,
    applied: &mut VoiceParamSnapshot,
    param: &ParamModulation,
    mut write: impl FnMut(EnvelopeTimeTarget, f32),
) -> u32 {
    let mut time_writes = 0u32;
    for target in EnvelopeTimeTarget::ALL {
        let (specific, all) = target.deltas(param);
        if let Some(seconds) = apply_modulated_time(
            target.base_raw(bases),
            specific,
            all,
            target.applied_raw_mut(applied),
            target,
        ) {
            write(target, seconds);
            time_writes += 1;
        }
    }
    time_writes
}
