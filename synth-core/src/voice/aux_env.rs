use crate::{
    ParamId,
    dsp::{DEFAULT_PARAMETER_SMOOTHING_SECONDS, parameter_smoother::ParameterSmoother},
    math::WideF32,
    patch::AuxEnvelopeParams,
};

pub struct AuxEnv {
    envelope: crate::dsp::envelope::DadsrEnvelope,
    velocity_amount: ParameterSmoother,
}

impl AuxEnv {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            envelope: crate::dsp::envelope::DadsrEnvelope::analog(sample_rate),
            velocity_amount: ParameterSmoother::new(
                0.0,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
        }
    }

    pub fn apply_params(&mut self, params: &AuxEnvelopeParams) {
        self.snap_velocity_amount(params.velocity);
        self.set_delay_seconds(params.delay);
        self.set_attack_seconds(params.attack);
        self.set_decay_seconds(params.decay);
        self.set_sustain_level(params.sustain);
        self.set_release_seconds(params.release);
        self.set_repeat(params.repeat);
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) -> bool {
        match id {
            ParamId::AuxEgVelocity => self.set_velocity_amount(value),
            ParamId::AuxEgDelay => self.set_delay_seconds(value),
            ParamId::AuxEgAttack => self.set_attack_seconds(value),
            ParamId::AuxEgDecay => self.set_decay_seconds(value),
            ParamId::AuxEgSustain => self.set_sustain_level(value),
            ParamId::AuxEgRelease => self.set_release_seconds(value),
            ParamId::AuxEgLoop => self.set_repeat(value >= 0.5),
            _ => return false,
        }
        true
    }

    pub fn advance_smoothers(&mut self) {
        self.velocity_amount.next();
    }

    pub fn next_signal(&mut self, velocities: WideF32, aux_amount: f32) -> (WideF32, WideF32) {
        let aux_env = self.envelope.next();
        let velocity_amount = self.velocity_amount.value();
        let aux_velocity_scale =
            WideF32::splat(1.0 - velocity_amount) + velocities * WideF32::splat(velocity_amount);
        let aux_signal = aux_env * WideF32::splat(aux_amount) * aux_velocity_scale;
        (aux_env, aux_signal)
    }

    pub fn trigger_lane(&mut self, lane: usize) {
        self.envelope.trigger_lane(lane);
    }

    pub fn release_lane(&mut self, lane: usize) {
        self.envelope.release_lane(lane);
    }

    pub fn release_all(&mut self) {
        self.envelope.release_all();
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.envelope.reset_lane(lane);
    }

    pub fn set_velocity_amount(&mut self, amount: f32) {
        self.velocity_amount.set_target(amount.clamp(0.0, 1.0));
    }

    fn snap_velocity_amount(&mut self, amount: f32) {
        self.velocity_amount.snap(amount.clamp(0.0, 1.0));
    }

    pub fn set_delay_seconds(&mut self, seconds: f32) {
        self.envelope.set_delay_seconds(seconds);
    }

    pub fn set_attack_seconds(&mut self, seconds: f32) {
        self.envelope.set_attack_seconds(seconds);
    }

    pub fn set_decay_seconds(&mut self, seconds: f32) {
        self.envelope.set_decay_seconds(seconds);
    }

    pub fn set_sustain_level(&mut self, sustain: f32) {
        self.envelope.set_sustain_level(sustain);
    }

    pub fn set_release_seconds(&mut self, seconds: f32) {
        self.envelope.set_release_seconds(seconds);
    }

    pub fn set_repeat(&mut self, repeat: bool) {
        self.envelope.set_loop_enabled(repeat);
    }

    #[cfg(test)]
    pub(crate) fn envelope_next(&mut self) -> WideF32 {
        self.envelope.next()
    }

    #[cfg(test)]
    pub(crate) fn envelope_is_idle_lane(&self, lane: usize) -> bool {
        self.envelope.is_idle_lane(lane)
    }
}
