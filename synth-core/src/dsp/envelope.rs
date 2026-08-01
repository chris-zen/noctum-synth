use crate::math::{F32, WideF32};

pub const MIN_TIME_SECONDS: f32 = 0.0005;
pub const MAX_TIME_SECONDS: f32 = 40.0;
pub const DEFAULT_ATTACK_SECONDS: f32 = 0.25;
pub const DEFAULT_DECAY_SECONDS: f32 = 1.0;
pub const DEFAULT_SUSTAIN_LEVEL: f32 = 0.707;
pub const DEFAULT_RELEASE_SECONDS: f32 = 3.0;
const IDLE_THRESHOLD: f32 = 1.0e-4;
/// Target-ratio constant for the analog-style attack curve: `exp(-1.5)`.
/// It lets the exponential curve cross `1.0` in finite configured time.
const ANALOG_ATTACK_TCO: f32 = 0.223_130_17;
/// Target-ratio constant for analog-style decay/release curves: `exp(-4.95)`.
/// It makes the curve discharge past the target, then snap at the threshold.
const ANALOG_DECAY_TCO: f32 = 0.007_083_409;

/// Four-lane delayed ADSR envelope with optional attack-decay looping.
pub struct DadsrEnvelope {
    stage: [EnvStage; WideF32::LANES],
    gate: [bool; WideF32::LANES],
    value: WideF32,
    delay_seconds: f32,
    delay_samples_remaining: [u32; WideF32::LANES],
    shutdown_start: [f32; WideF32::LANES],
    shutdown_samples_remaining: [u32; WideF32::LANES],
    shutdown_total_samples: [u32; WideF32::LANES],
    attack_seconds: f32,
    decay_seconds: f32,
    sustain_level: f32,
    release_seconds: f32,
    curve: EnvelopeCurve,
    loop_enabled: bool,
    sample_rate: f32,
}

impl DadsrEnvelope {
    /// Creates an envelope whose attack, decay, and release segments advance linearly.
    pub fn linear(sample_rate: f32) -> Self {
        Self {
            stage: [EnvStage::Idle; WideF32::LANES],
            gate: [false; WideF32::LANES],
            value: WideF32::ZERO,
            delay_seconds: 0.0,
            delay_samples_remaining: [0; WideF32::LANES],
            shutdown_start: [0.0; WideF32::LANES],
            shutdown_samples_remaining: [0; WideF32::LANES],
            shutdown_total_samples: [0; WideF32::LANES],
            attack_seconds: DEFAULT_ATTACK_SECONDS,
            decay_seconds: DEFAULT_DECAY_SECONDS,
            sustain_level: DEFAULT_SUSTAIN_LEVEL,
            release_seconds: DEFAULT_RELEASE_SECONDS,
            curve: EnvelopeCurve::linear(
                DEFAULT_ATTACK_SECONDS,
                DEFAULT_DECAY_SECONDS,
                DEFAULT_SUSTAIN_LEVEL,
                DEFAULT_RELEASE_SECONDS,
                sample_rate,
            ),
            loop_enabled: false,
            sample_rate,
        }
    }

    /// Creates an envelope whose attack, decay, and release segments use analog-style curves.
    pub fn analog(sample_rate: f32) -> Self {
        Self {
            stage: [EnvStage::Idle; WideF32::LANES],
            gate: [false; WideF32::LANES],
            value: WideF32::ZERO,
            delay_seconds: 0.0,
            delay_samples_remaining: [0; WideF32::LANES],
            shutdown_start: [0.0; WideF32::LANES],
            shutdown_samples_remaining: [0; WideF32::LANES],
            shutdown_total_samples: [0; WideF32::LANES],
            attack_seconds: DEFAULT_ATTACK_SECONDS,
            decay_seconds: DEFAULT_DECAY_SECONDS,
            sustain_level: DEFAULT_SUSTAIN_LEVEL,
            release_seconds: DEFAULT_RELEASE_SECONDS,
            curve: EnvelopeCurve::analog(
                DEFAULT_ATTACK_SECONDS,
                DEFAULT_DECAY_SECONDS,
                DEFAULT_SUSTAIN_LEVEL,
                DEFAULT_RELEASE_SECONDS,
                sample_rate,
            ),
            loop_enabled: false,
            sample_rate,
        }
    }

    /// Sets the delay time before the attack segment starts, in seconds.
    pub fn set_delay_seconds(&mut self, seconds: f32) {
        self.delay_seconds = seconds.clamp(0.0, MAX_TIME_SECONDS);
    }

    /// Sets the attack time, in seconds, and updates the active curve state.
    pub fn set_attack_seconds(&mut self, seconds: f32) {
        self.attack_seconds = seconds.clamp(MIN_TIME_SECONDS, MAX_TIME_SECONDS);
        self.curve.set_attack(self.attack_seconds, self.sample_rate);
    }

    /// Sets the decay time, in seconds, and updates the active curve state.
    pub fn set_decay_seconds(&mut self, seconds: f32) {
        self.decay_seconds = seconds.clamp(MIN_TIME_SECONDS, MAX_TIME_SECONDS);
        self.curve
            .set_decay(self.decay_seconds, self.sustain_level, self.sample_rate);
    }

    /// Sets the sustain level and updates any curve state that depends on the target level.
    pub fn set_sustain_level(&mut self, sustain: f32) {
        self.sustain_level = sustain.clamp(0.0, 1.0);
        self.curve
            .set_decay(self.decay_seconds, self.sustain_level, self.sample_rate);
    }

    /// Sets the release time, in seconds, and updates the active curve state.
    pub fn set_release_seconds(&mut self, seconds: f32) {
        self.release_seconds = seconds.clamp(MIN_TIME_SECONDS, MAX_TIME_SECONDS);
        self.curve
            .set_release(self.release_seconds, self.sample_rate);
    }

    /// Enables or disables attack-decay looping while a lane remains gated.
    pub fn set_loop_enabled(&mut self, enabled: bool) {
        self.loop_enabled = enabled;
    }

    /// Starts or retriggers the envelope for a single SIMD lane.
    pub fn trigger_lane(&mut self, lane: usize) {
        self.gate[lane] = true;
        self.start_delay_or_attack(lane);
    }

    /// Advances every lane by one sample and returns the current envelope values.
    pub fn next(&mut self) -> WideF32 {
        let mut values = self.value.to_array();
        let curve = self.curve;

        for (lane, value) in values.iter_mut().enumerate() {
            match self.stage[lane] {
                EnvStage::Idle => {
                    *value = 0.0;
                }
                EnvStage::Delay => {
                    *value = 0.0;
                    if self.delay_samples_remaining[lane] > 0 {
                        self.delay_samples_remaining[lane] -= 1;
                    }
                    if self.delay_samples_remaining[lane] == 0 {
                        self.stage[lane] = EnvStage::Attack;
                    }
                }
                EnvStage::Attack => {
                    *value = curve.attack(*value);
                    if *value >= 1.0 {
                        *value = 1.0;
                        self.stage[lane] = EnvStage::Decay;
                    }
                }
                EnvStage::Decay => {
                    *value = curve.decay(*value);
                    if *value <= self.sustain_level {
                        *value = self.sustain_level;
                        if self.loop_enabled && self.gate[lane] {
                            *value = 0.0;
                            self.start_delay_or_attack(lane);
                        } else {
                            self.stage[lane] = EnvStage::Sustain;
                        }
                    }
                }
                EnvStage::Sustain => {
                    *value = self.sustain_level;
                }
                EnvStage::Release => {
                    *value = curve.release(*value);
                    if *value <= IDLE_THRESHOLD {
                        *value = 0.0;
                        self.stage[lane] = EnvStage::Idle;
                    }
                }
                EnvStage::Shutdown => {
                    let remaining = self.shutdown_samples_remaining[lane].saturating_sub(1);
                    self.shutdown_samples_remaining[lane] = remaining;
                    if remaining == 0 {
                        *value = 0.0;
                        self.shutdown_start[lane] = 0.0;
                        self.shutdown_total_samples[lane] = 0;
                        self.stage[lane] = EnvStage::Idle;
                    } else {
                        let total = self.shutdown_total_samples[lane] as f32;
                        let progress = 1.0 - remaining as f32 / total;
                        // Smoothstep from one to zero. Its zero slope at both ends
                        // avoids turning the de-click ramp into a slope discontinuity.
                        let gain = 1.0 - progress * progress * (3.0 - 2.0 * progress);
                        *value = self.shutdown_start[lane] * gain;
                    }
                }
            }
        }

        self.value = WideF32::new(values);
        self.value
    }

    /// Releases a single SIMD lane, moving it into the release segment when audible.
    pub fn release_lane(&mut self, lane: usize) {
        self.gate[lane] = false;
        if self.stage[lane] == EnvStage::Shutdown {
            return;
        }
        if self.stage[lane] != EnvStage::Idle {
            self.delay_samples_remaining[lane] = 0;
            let current = self.value.to_array()[lane].max(0.0);
            if current <= IDLE_THRESHOLD {
                self.value = self.value.replace_lane(lane, 0.0);
                self.stage[lane] = EnvStage::Idle;
            } else {
                self.stage[lane] = EnvStage::Release;
            }
        }
    }

    /// Releases every lane.
    pub fn release_all(&mut self) {
        for lane in 0..WideF32::LANES {
            self.release_lane(lane);
        }
    }

    /// Returns true when a lane has reached the idle stage and is effectively silent.
    pub fn is_idle_lane(&self, lane: usize) -> bool {
        self.stage[lane] == EnvStage::Idle && self.value.to_array()[lane] <= IDLE_THRESHOLD
    }

    /// Smoothly silences a lane over a fixed interval for click-free voice reuse.
    pub(crate) fn shutdown_lane(&mut self, lane: usize, seconds: f32) {
        self.gate[lane] = false;
        self.delay_samples_remaining[lane] = 0;
        let current = self.value.to_array()[lane].max(0.0);
        if current <= IDLE_THRESHOLD {
            self.reset_lane(lane);
            return;
        }

        let samples = F32(seconds.max(MIN_TIME_SECONDS) * self.sample_rate)
            .round()
            .as_f32()
            .max(1.0) as u32;
        self.shutdown_start[lane] = current;
        self.shutdown_samples_remaining[lane] = samples;
        self.shutdown_total_samples[lane] = samples;
        self.stage[lane] = EnvStage::Shutdown;
    }

    /// Returns a lane to its inactive zero state without disturbing adjacent lanes.
    pub(crate) fn reset_lane(&mut self, lane: usize) {
        self.gate[lane] = false;
        self.delay_samples_remaining[lane] = 0;
        self.shutdown_start[lane] = 0.0;
        self.shutdown_samples_remaining[lane] = 0;
        self.shutdown_total_samples[lane] = 0;
        self.value = self.value.replace_lane(lane, 0.0);
        self.stage[lane] = EnvStage::Idle;
    }

    /// Moves a lane into delay or directly into attack based on the current delay time.
    fn start_delay_or_attack(&mut self, lane: usize) {
        if self.delay_seconds <= 0.0 {
            self.delay_samples_remaining[lane] = 0;
            self.stage[lane] = EnvStage::Attack;
        } else {
            self.delay_samples_remaining[lane] = F32(self.delay_seconds * self.sample_rate)
                .round()
                .as_f32()
                .max(1.0) as u32;
            self.stage[lane] = EnvStage::Delay;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Delay,
    Attack,
    Decay,
    Sustain,
    Release,
    Shutdown,
}

#[derive(Clone, Copy)]
enum EnvelopeCurve {
    Linear(LinearEnvelopeCurve),
    Analog(AnalogEnvelopeCurve),
}

impl EnvelopeCurve {
    /// Creates the linear curve variant.
    fn linear(
        attack_seconds: f32,
        decay_seconds: f32,
        _sustain_level: f32,
        release_seconds: f32,
        sample_rate: f32,
    ) -> Self {
        Self::Linear(LinearEnvelopeCurve::new(
            attack_seconds,
            decay_seconds,
            release_seconds,
            sample_rate,
        ))
    }

    /// Creates the analog curve variant.
    fn analog(
        attack_seconds: f32,
        decay_seconds: f32,
        sustain_level: f32,
        release_seconds: f32,
        sample_rate: f32,
    ) -> Self {
        Self::Analog(AnalogEnvelopeCurve::new(
            attack_seconds,
            decay_seconds,
            sustain_level,
            release_seconds,
            sample_rate,
        ))
    }

    /// Updates attack coefficients for the active curve variant.
    fn set_attack(&mut self, seconds: f32, sample_rate: f32) {
        match self {
            Self::Linear(curve) => curve.set_attack(seconds, sample_rate),
            Self::Analog(curve) => curve.set_attack(seconds, sample_rate),
        }
    }

    /// Updates decay coefficients for the active curve variant.
    fn set_decay(&mut self, seconds: f32, sustain_level: f32, sample_rate: f32) {
        match self {
            Self::Linear(curve) => curve.set_decay(seconds, sample_rate),
            Self::Analog(curve) => curve.set_decay(seconds, sustain_level, sample_rate),
        }
    }

    /// Updates release coefficients for the active curve variant.
    fn set_release(&mut self, seconds: f32, sample_rate: f32) {
        match self {
            Self::Linear(curve) => curve.set_release(seconds, sample_rate),
            Self::Analog(curve) => curve.set_release(seconds, sample_rate),
        }
    }

    /// Advances an attack sample for the active curve variant.
    fn attack(self, value: f32) -> f32 {
        match self {
            Self::Linear(curve) => curve.attack(value),
            Self::Analog(curve) => curve.attack(value),
        }
    }

    /// Advances a decay sample for the active curve variant.
    fn decay(self, value: f32) -> f32 {
        match self {
            Self::Linear(curve) => curve.decay(value),
            Self::Analog(curve) => curve.decay(value),
        }
    }

    /// Advances a release sample for the active curve variant.
    fn release(self, value: f32) -> f32 {
        match self {
            Self::Linear(curve) => curve.release(value),
            Self::Analog(curve) => curve.release(value),
        }
    }
}

#[derive(Clone, Copy)]
struct LinearEnvelopeCurve {
    attack_step: f32,
    decay_step: f32,
    release_step: f32,
}

impl LinearEnvelopeCurve {
    /// Creates per-sample increments for each linear segment.
    fn new(
        attack_seconds: f32,
        decay_seconds: f32,
        release_seconds: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            attack_step: linear_step(attack_seconds, sample_rate),
            decay_step: linear_step(decay_seconds, sample_rate),
            release_step: linear_step(release_seconds, sample_rate),
        }
    }

    /// Recalculates the linear attack increment.
    fn set_attack(&mut self, seconds: f32, sample_rate: f32) {
        self.attack_step = linear_step(seconds, sample_rate);
    }

    /// Recalculates the linear decay increment.
    fn set_decay(&mut self, seconds: f32, sample_rate: f32) {
        self.decay_step = linear_step(seconds, sample_rate);
    }

    /// Recalculates the linear release increment.
    fn set_release(&mut self, seconds: f32, sample_rate: f32) {
        self.release_step = linear_step(seconds, sample_rate);
    }

    /// Applies one linear attack step.
    fn attack(self, value: f32) -> f32 {
        value + self.attack_step
    }

    /// Applies one linear decay step.
    fn decay(self, value: f32) -> f32 {
        value - self.decay_step
    }

    /// Applies one linear release step.
    fn release(self, value: f32) -> f32 {
        value - self.release_step
    }
}

#[derive(Clone, Copy)]
struct AnalogEnvelopeCurve {
    attack_coeff: f32,
    attack_offset: f32,
    decay_coeff: f32,
    decay_offset: f32,
    release_coeff: f32,
    release_offset: f32,
}

impl AnalogEnvelopeCurve {
    /// Creates coefficient/offset pairs for each analog-style segment.
    fn new(
        attack_seconds: f32,
        decay_seconds: f32,
        sustain_level: f32,
        release_seconds: f32,
        sample_rate: f32,
    ) -> Self {
        let attack_coeff = analog_coeff(ANALOG_ATTACK_TCO, attack_seconds, sample_rate);
        let attack_offset = (1.0 + ANALOG_ATTACK_TCO) * (1.0 - attack_coeff);
        let decay_coeff = analog_coeff(ANALOG_DECAY_TCO, decay_seconds, sample_rate);
        let decay_offset = (sustain_level - ANALOG_DECAY_TCO) * (1.0 - decay_coeff);
        let release_coeff = analog_coeff(ANALOG_DECAY_TCO, release_seconds, sample_rate);
        let release_offset = -ANALOG_DECAY_TCO * (1.0 - release_coeff);
        Self {
            attack_coeff,
            attack_offset,
            decay_coeff,
            decay_offset,
            release_coeff,
            release_offset,
        }
    }

    /// Recalculates the analog attack coefficient and offset.
    fn set_attack(&mut self, seconds: f32, sample_rate: f32) {
        self.attack_coeff = analog_coeff(ANALOG_ATTACK_TCO, seconds, sample_rate);
        self.attack_offset = (1.0 + ANALOG_ATTACK_TCO) * (1.0 - self.attack_coeff);
    }

    /// Recalculates the analog decay coefficient and offset.
    fn set_decay(&mut self, seconds: f32, sustain_level: f32, sample_rate: f32) {
        self.decay_coeff = analog_coeff(ANALOG_DECAY_TCO, seconds, sample_rate);
        self.decay_offset = (sustain_level - ANALOG_DECAY_TCO) * (1.0 - self.decay_coeff);
    }

    /// Recalculates the analog release coefficient and offset.
    fn set_release(&mut self, seconds: f32, sample_rate: f32) {
        self.release_coeff = analog_coeff(ANALOG_DECAY_TCO, seconds, sample_rate);
        self.release_offset = -ANALOG_DECAY_TCO * (1.0 - self.release_coeff);
    }

    /// Applies one analog-style attack step.
    fn attack(self, value: f32) -> f32 {
        self.attack_offset + value * self.attack_coeff
    }

    /// Applies one analog-style decay step.
    fn decay(self, value: f32) -> f32 {
        self.decay_offset + value * self.decay_coeff
    }

    /// Applies one analog-style release step.
    fn release(self, value: f32) -> f32 {
        self.release_offset + value * self.release_coeff
    }
}

fn linear_step(seconds: f32, sample_rate: f32) -> f32 {
    1.0 / (seconds * sample_rate)
}

fn analog_coeff(tco: f32, seconds: f32, sample_rate: f32) -> f32 {
    let samples = seconds * sample_rate;
    (-F32((1.0 + tco) / tco).ln() / samples).exp().as_f32()
}

#[cfg(test)]
mod tests {
    use super::DadsrEnvelope;

    #[test]
    fn adsr_linear_decay_uses_max_to_zero_time_constant() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.set_decay_seconds(1.0);
        env.set_sustain_level(0.25);
        env.trigger_lane(0);

        assert_eq!(env.next().to_array()[0], 1.0);
        for _ in 0..749 {
            assert!(env.next().to_array()[0] > 0.25);
        }

        let sustain = env.next().to_array()[0];
        assert!((sustain - 0.25).abs() < 1.0e-5);
    }

    #[test]
    fn adsr_linear_release_uses_max_to_zero_time_constant() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.set_decay_seconds(0.001);
        env.set_sustain_level(0.25);
        env.set_release_seconds(1.0);
        env.trigger_lane(0);

        env.next();
        assert!((env.next().to_array()[0] - 0.25).abs() < 1.0e-6);
        env.release_lane(0);

        for _ in 0..249 {
            assert!(env.next().to_array()[0] > 0.0);
        }
        assert_eq!(env.next().to_array()[0], 0.0);
        assert!(env.is_idle_lane(0));
    }

    #[test]
    fn analog_curve_rises_and_decays_faster_than_linear_early() {
        let mut linear = DadsrEnvelope::linear(1000.0);
        linear.set_attack_seconds(1.0);
        linear.trigger_lane(0);

        let mut analog = DadsrEnvelope::analog(1000.0);
        analog.set_attack_seconds(1.0);
        analog.trigger_lane(0);

        for _ in 0..100 {
            linear.next();
            analog.next();
        }

        let linear_attack = linear.next().to_array()[0];
        let analog_attack = analog.next().to_array()[0];
        assert!(
            analog_attack > linear_attack,
            "analog attack should rise faster early, linear {linear_attack}, analog {analog_attack}"
        );

        let mut linear = DadsrEnvelope::linear(1000.0);
        linear.set_attack_seconds(0.001);
        linear.set_decay_seconds(1.0);
        linear.set_sustain_level(0.0);
        linear.trigger_lane(0);
        linear.next();

        let mut analog = DadsrEnvelope::analog(1000.0);
        analog.set_attack_seconds(0.001);
        analog.set_decay_seconds(1.0);
        analog.set_sustain_level(0.0);
        analog.trigger_lane(0);
        analog.next();

        for _ in 0..100 {
            linear.next();
            analog.next();
        }

        let linear_decay = linear.next().to_array()[0];
        let analog_decay = analog.next().to_array()[0];
        assert!(
            analog_decay < linear_decay,
            "analog decay should fall faster early, linear {linear_decay}, analog {analog_decay}"
        );
    }

    #[test]
    fn adsr_retrigger_continues_from_current_level() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(1.0);
        env.trigger_lane(0);

        let mut before_retrigger = 0.0;
        for _ in 0..250 {
            before_retrigger = env.next().to_array()[0];
        }

        env.trigger_lane(0);
        let after_retrigger = env.next().to_array()[0];

        assert!(before_retrigger > 0.2);
        assert!(
            after_retrigger > before_retrigger,
            "retrigger should continue upward from {before_retrigger}, got {after_retrigger}"
        );
    }

    #[test]
    fn shutdown_reaches_zero_in_the_requested_time() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.trigger_lane(0);
        assert_eq!(env.next().to_array()[0], 1.0);

        env.shutdown_lane(0, 0.002);
        assert!((env.next().to_array()[0] - 0.5).abs() < 1.0e-6);
        assert_eq!(env.next().to_array()[0], 0.0);
        assert!(env.is_idle_lane(0));
    }

    #[test]
    fn shutdown_has_gentle_slopes_at_both_ends() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.trigger_lane(0);
        assert_eq!(env.next().to_array()[0], 1.0);

        env.shutdown_lane(0, 0.1);
        let first = env.next().to_array()[0];
        let mut midpoint = first;
        for _ in 1..50 {
            midpoint = env.next().to_array()[0];
        }
        let mut penultimate = midpoint;
        for _ in 50..99 {
            penultimate = env.next().to_array()[0];
        }
        let final_value = env.next().to_array()[0];

        assert!(
            1.0 - first < 0.001,
            "shutdown started too abruptly: {first}"
        );
        assert!((midpoint - 0.5).abs() < 1.0e-6);
        assert!(
            penultimate < 0.001,
            "shutdown ended too abruptly: {penultimate}"
        );
        assert_eq!(final_value, 0.0);
        assert!(env.is_idle_lane(0));
    }

    #[cfg(not(feature = "wide-1"))]
    #[test]
    fn reset_lane_does_not_change_adjacent_lanes() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.004);
        env.trigger_lane(0);
        env.trigger_lane(1);
        env.next();

        env.reset_lane(0);
        let values = env.next().to_array();
        assert_eq!(values[0], 0.0);
        assert_eq!(values[1], 0.5);
        assert!(env.is_idle_lane(0));
        assert!(!env.is_idle_lane(1));
    }

    #[test]
    fn dadsr_delay_holds_output_before_attack() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_delay_seconds(0.01);
        env.set_attack_seconds(0.001);
        env.trigger_lane(0);

        for _ in 0..9 {
            assert_eq!(env.next().to_array()[0], 0.0);
        }
        assert_eq!(env.next().to_array()[0], 0.0);
        assert!(env.next().to_array()[0] > 0.0);
    }

    #[test]
    fn dadsr_reaches_sustain_and_releases_to_idle() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.set_decay_seconds(0.001);
        env.set_sustain_level(0.4);
        env.set_release_seconds(0.001);
        env.trigger_lane(0);

        env.next();
        let sustain = env.next().to_array()[0];
        assert!((sustain - 0.4).abs() < 1.0e-6);

        env.release_lane(0);
        assert_eq!(env.next().to_array()[0], 0.0);
        assert!(env.is_idle_lane(0));
    }

    #[test]
    fn dadsr_repeat_cycles_until_note_off() {
        let mut env = DadsrEnvelope::linear(1000.0);
        env.set_attack_seconds(0.001);
        env.set_decay_seconds(0.001);
        env.set_sustain_level(0.5);
        env.set_release_seconds(0.001);
        env.set_loop_enabled(true);
        env.trigger_lane(0);

        let first = env.next().to_array()[0];
        let loop_reset = env.next().to_array()[0];
        let second = env.next().to_array()[0];

        assert!(first > 0.9);
        assert_eq!(loop_reset, 0.0);
        assert!(second > 0.9);

        env.release_lane(0);
        assert_eq!(env.next().to_array()[0], 0.0);
        assert!(env.is_idle_lane(0));
    }
}
