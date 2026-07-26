use crate::ParamId;
use crate::math::WideF32;
use crate::patch::AmplifierParams;

pub struct Amplifier {
    envelope: crate::dsp::DadsrEnvelope,
    initial_level: f32,
    env_amount: f32,
    velocity_amount: f32,
}

impl Amplifier {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            envelope: crate::dsp::DadsrEnvelope::analog(sample_rate),
            initial_level: 0.0,
            env_amount: 0.0,
            velocity_amount: 0.0,
        }
    }

    pub fn apply_params(&mut self, params: &AmplifierParams) {
        self.set_initial_level(params.initial_level);
        self.set_env_amount(params.env_amount);
        self.set_velocity_amount(params.velocity);
        self.set_delay_seconds(params.eg_delay);
        self.set_attack_seconds(params.eg_attack);
        self.set_decay_seconds(params.eg_decay);
        self.set_sustain_level(params.eg_sustain);
        self.set_release_seconds(params.eg_release);
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) -> bool {
        match id {
            ParamId::VcaInitialLevel => self.set_initial_level(value),
            ParamId::AmpEnvAmount => self.set_env_amount(value),
            ParamId::AmpVelocity => self.set_velocity_amount(value),
            ParamId::AmpEgDelay => self.set_delay_seconds(value),
            ParamId::AmpEgAttack => self.set_attack_seconds(value),
            ParamId::AmpEgDecay => self.set_decay_seconds(value),
            ParamId::AmpEgSustain => self.set_sustain_level(value),
            ParamId::AmpEgRelease => self.set_release_seconds(value),
            _ => return false,
        }
        true
    }

    pub fn next_envelope(&mut self) -> WideF32 {
        self.envelope.next()
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

    pub fn is_idle_lane(&self, lane: usize) -> bool {
        self.envelope.is_idle_lane(lane)
    }

    pub fn initial_level(&self) -> f32 {
        self.initial_level
    }

    pub fn set_initial_level(&mut self, level: f32) {
        self.initial_level = level.clamp(0.0, 1.0);
    }

    pub fn set_env_amount(&mut self, amount: f32) {
        self.env_amount = amount.clamp(0.0, 1.0);
    }

    pub fn shutdown_lane(&mut self, lane: usize, seconds: f32) {
        self.envelope.shutdown_lane(lane, seconds);
    }

    pub fn set_velocity_amount(&mut self, amount: f32) {
        self.velocity_amount = amount.clamp(0.0, 1.0);
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

    pub fn gain(&self, amp_env: WideF32, velocities: WideF32, amp_lfo_gain: WideF32) -> WideF32 {
        let velocity_gain = WideF32::splat(1.0 - self.velocity_amount)
            + velocities * WideF32::splat(self.velocity_amount);
        let env_gain = WideF32::splat(self.initial_level)
            + (WideF32::splat(1.0 - self.initial_level) * amp_env * self.env_amount);
        velocity_gain * env_gain * amp_lfo_gain
    }
}
