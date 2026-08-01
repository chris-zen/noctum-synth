use crate::{
    ParamId,
    dsp::{DEFAULT_PARAMETER_SMOOTHING_SECONDS, parameter_smoother::ParameterSmoother},
    math::WideF32,
    patch::AmplifierParams,
};

pub struct Amplifier {
    envelope: crate::dsp::envelope::DadsrEnvelope,
    initial_level: ParameterSmoother,
    env_amount: ParameterSmoother,
    velocity_amount: ParameterSmoother,
}

impl Amplifier {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            envelope: crate::dsp::envelope::DadsrEnvelope::analog(sample_rate),
            initial_level: ParameterSmoother::new(
                0.0,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
            env_amount: ParameterSmoother::new(
                0.0,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
            velocity_amount: ParameterSmoother::new(
                0.0,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
        }
    }

    pub fn apply_params(&mut self, params: &AmplifierParams) {
        self.snap_initial_level(params.initial_level);
        self.snap_env_amount(params.env_amount);
        self.snap_velocity_amount(params.velocity);
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

    pub fn advance_smoothers(&mut self) {
        self.initial_level.next();
        self.env_amount.next();
        self.velocity_amount.next();
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
        self.initial_level.target()
    }

    pub fn set_initial_level(&mut self, level: f32) {
        self.initial_level.set_target(level.clamp(0.0, 1.0));
    }

    fn snap_initial_level(&mut self, level: f32) {
        self.initial_level.snap(level.clamp(0.0, 1.0));
    }

    pub fn set_env_amount(&mut self, amount: f32) {
        self.env_amount.set_target(amount.clamp(0.0, 1.0));
    }

    fn snap_env_amount(&mut self, amount: f32) {
        self.env_amount.snap(amount.clamp(0.0, 1.0));
    }

    pub fn shutdown_lane(&mut self, lane: usize, seconds: f32) {
        self.envelope.shutdown_lane(lane, seconds);
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

    /// Computes the Rev2 VCA control signal.
    ///
    /// `Velocity Amount` modulates (adds to) `VCA Envelope Amount`; it is not a
    /// second gain applied after the envelope. Modulation routed to `VCA` joins
    /// the same control sum. See the Amplifier Envelope section of the
    /// [Prophet Rev2 User's Guide](https://www.sequential.com/wp-content/uploads/2021/02/Prophet-Rev2-Users-Guide-1.2.4.pdf#page=35).
    pub fn gain(&self, amp_env: WideF32, velocities: WideF32, amp_modulation: WideF32) -> WideF32 {
        let velocity_amount = self.velocity_amount.value();
        let effective_env_amount = (WideF32::splat(self.env_amount.value())
            + velocities.clamp(WideF32::ZERO, WideF32::splat(1.0))
                * WideF32::splat(velocity_amount))
        .clamp(WideF32::ZERO, WideF32::splat(1.0));
        let initial_level = self.initial_level.value();
        (WideF32::splat(initial_level)
            + amp_env.clamp(WideF32::ZERO, WideF32::splat(1.0)) * effective_env_amount
            + amp_modulation)
            .clamp(WideF32::ZERO, WideF32::splat(1.0))
    }
}
