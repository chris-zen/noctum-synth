use crate::math::{F32, WideF32};

pub const DEFAULT_PARAMETER_SMOOTHING_SECONDS: f32 = 0.005;

const SETTLE_EPSILON: f32 = 1e-5;

pub fn smoothing_coefficient(sample_rate: f32, time_seconds: f32) -> f32 {
    let tau_samples =
        (sample_rate.max(1.0) * time_seconds.max(1.0 / sample_rate.max(1.0))).max(1.0);
    1.0 - F32(-1.0 / tau_samples).exp().as_f32()
}

pub fn smoothing_coefficient_euler_approx(sample_rate: f32, time_seconds: f32) -> f32 {
    (1.0 / (sample_rate.max(1.0) * time_seconds.max(1.0 / sample_rate.max(1.0)))).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug)]
pub struct ParameterSmoother {
    current: f32,
    target: f32,
    coefficient: f32,
    time_seconds: f32,
}

impl ParameterSmoother {
    pub fn new(initial: f32, sample_rate: f32, time_seconds: f32) -> Self {
        let mut smoother = Self {
            current: initial,
            target: initial,
            coefficient: 0.0,
            time_seconds,
        };
        smoother.set_sample_rate(sample_rate);
        smoother
    }

    pub fn with_coefficient(initial: f32, coefficient: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            coefficient: coefficient.clamp(0.0, 1.0),
            time_seconds: DEFAULT_PARAMETER_SMOOTHING_SECONDS,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.coefficient = smoothing_coefficient(sample_rate, self.time_seconds);
    }

    pub fn set_coefficient(&mut self, coefficient: f32) {
        self.coefficient = coefficient.clamp(0.0, 1.0);
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }

    pub fn value(&self) -> f32 {
        self.current
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn next(&mut self) -> f32 {
        self.advance_toward(self.target);
        self.current
    }

    pub fn next_toward(&mut self, target: f32) -> f32 {
        self.target = target;
        self.next()
    }

    fn advance_toward(&mut self, target: f32) {
        let delta = target - self.current;
        if delta.abs() < SETTLE_EPSILON {
            self.current = target;
            return;
        }
        self.current += delta * self.coefficient;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WideParameterSmoother {
    current: WideF32,
    target: WideF32,
    coefficient: WideF32,
    time_seconds: f32,
    use_euler_coeff: bool,
}

impl WideParameterSmoother {
    pub fn new(initial: f32, sample_rate: f32, time_seconds: f32) -> Self {
        let mut smoother = Self {
            current: WideF32::splat(initial),
            target: WideF32::splat(initial),
            coefficient: WideF32::ZERO,
            time_seconds,
            use_euler_coeff: false,
        };
        smoother.set_sample_rate(sample_rate);
        smoother
    }

    pub fn with_euler_coefficient(initial: f32, sample_rate: f32, time_seconds: f32) -> Self {
        let coeff = smoothing_coefficient_euler_approx(sample_rate, time_seconds);
        Self {
            current: WideF32::splat(initial),
            target: WideF32::splat(initial),
            coefficient: WideF32::splat(coeff),
            time_seconds,
            use_euler_coeff: true,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let coeff = if self.use_euler_coeff {
            smoothing_coefficient_euler_approx(sample_rate, self.time_seconds)
        } else {
            smoothing_coefficient(sample_rate, self.time_seconds)
        };
        self.coefficient = WideF32::splat(coeff);
    }

    pub fn set_target_lane(&mut self, lane: usize, target: f32) {
        self.target = self.target.replace_lane(lane, target);
    }

    pub fn snap_lane(&mut self, lane: usize, value: f32) {
        self.current = self.current.replace_lane(lane, value);
        self.target = self.target.replace_lane(lane, value);
    }

    pub fn snap_all(&mut self, value: f32) {
        let value = WideF32::splat(value);
        self.current = value;
        self.target = value;
    }

    pub fn value(&self) -> WideF32 {
        self.current
    }

    pub fn target(&self) -> WideF32 {
        self.target
    }

    pub fn next(&mut self) -> WideF32 {
        self.advance_toward(self.target);
        self.current
    }

    pub fn next_toward(&mut self, target: WideF32) -> WideF32 {
        self.target = target;
        self.next()
    }

    fn advance_toward(&mut self, target: WideF32) {
        let delta = target - self.current;
        let settled = delta.abs().simd_lt(WideF32::splat(SETTLE_EPSILON));
        self.current = settled.blend(target, self.current + delta * self.coefficient);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_is_immediate() {
        let mut smoother = ParameterSmoother::new(0.0, 44_100.0, 0.005);
        smoother.snap(0.75);
        assert_eq!(smoother.next(), 0.75);
    }

    #[test]
    fn ramp_approaches_target_and_settles_exactly() {
        let mut smoother = ParameterSmoother::new(0.0, 44_100.0, 0.005);
        smoother.set_target(1.0);
        for _ in 0..10_000 {
            smoother.next();
        }
        assert_eq!(smoother.value(), 1.0);
    }

    #[test]
    fn epsilon_settle_reaches_exact_zero() {
        let mut smoother = ParameterSmoother::new(1.0, 44_100.0, 0.005);
        smoother.set_target(0.0);
        for _ in 0..10_000 {
            smoother.next();
        }
        assert_eq!(smoother.value(), 0.0);
    }

    #[test]
    fn coefficient_clamps_at_extreme_sample_rate() {
        let coeff = smoothing_coefficient(1.0, 0.005);
        assert!((0.0..=1.0).contains(&coeff));
    }

    #[test]
    fn wide_lane_independence() {
        if WideF32::LANES < 2 {
            return;
        }
        let mut smoother = WideParameterSmoother::new(0.0, 44_100.0, 0.005);
        smoother.snap_lane(0, 1.0);
        smoother.set_target_lane(1, 0.5);
        let output = smoother.next();
        assert!((output.to_array()[0] - 1.0).abs() < 1e-6);
        assert!(output.to_array()[1] > 0.0);
        assert!(output.to_array()[1] < 0.5);
    }

    #[test]
    fn wide_snap_lane_leaves_other_lanes() {
        let mut smoother = WideParameterSmoother::new(0.25, 44_100.0, 0.005);
        smoother.snap_lane(2, 0.9);
        assert_eq!(smoother.value().to_array()[2], 0.9);
        assert_eq!(smoother.target().to_array()[2], 0.9);
        assert_eq!(smoother.value().to_array()[0], 0.25);
    }
}
