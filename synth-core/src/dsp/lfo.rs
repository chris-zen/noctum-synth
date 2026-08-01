//! Low-frequency oscillators for periodic modulation.

use crate::{
    dsp::{parameter_smoother::WideParameterSmoother, rng::DspRng},
    math::WideF32,
};

/// Minimum LFO rate in Hz.
pub const MIN_LFO_RATE_HZ: f32 = 0.022;
/// Maximum LFO rate in Hz.
pub const MAX_LFO_RATE_HZ: f32 = 500.0;
/// Lowest internal rate required by 30 BPM, half-note clock, 32-step sync.
pub(crate) const MIN_SYNCED_LFO_RATE_HZ: f32 = 1.0 / 128.0;
const LFO_SMOOTHING_SECONDS: f32 = 0.005;

/// LFO waveform shape.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoWaveform {
    Triangle,
    Saw,
    ReverseSaw,
    Square,
    SampleAndHold,
}

impl LfoWaveform {
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Saw,
            2 => Self::ReverseSaw,
            3 => Self::Square,
            4 => Self::SampleAndHold,
            _ => Self::Triangle,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Triangle => 0,
            Self::Saw => 1,
            Self::ReverseSaw => 2,
            Self::Square => 3,
            Self::SampleAndHold => 4,
        }
    }
}

/// Four-lane LFO with configurable rate, depth, waveform, and key sync.
pub struct Lfo {
    phase: WideF32,
    phase_uniform: bool,
    phase_inc: WideF32,
    waveform: LfoWaveform,
    key_sync: bool,
    depth: f32,
    sample_and_hold: WideF32,
    rng: [DspRng; WideF32::LANES],
    sample_rate: f32,
    output: WideParameterSmoother,
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new(crate::DEFAULT_SAMPLE_RATE)
    }
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        let mut lfo = Self {
            phase: WideF32::ZERO,
            phase_uniform: true,
            phase_inc: WideF32::ZERO,
            waveform: LfoWaveform::Triangle,
            key_sync: true,
            depth: 0.0,
            sample_and_hold: WideF32::ZERO,
            rng: core::array::from_fn(|i| {
                let seeds = [
                    (0x4c46_4f01, 0x5148_0001),
                    (0x4c46_4f02, 0x5148_0002),
                    (0x4c46_4f03, 0x5148_0003),
                    (0x4c46_4f04, 0x5148_0004),
                    (0x4c46_4f05, 0x5148_0005),
                    (0x4c46_4f06, 0x5148_0006),
                    (0x4c46_4f07, 0x5148_0007),
                    (0x4c46_4f08, 0x5148_0008),
                ];
                DspRng::new(seeds[i].0, seeds[i].1)
            }),
            sample_rate,
            output: WideParameterSmoother::with_euler_coefficient(
                0.0,
                sample_rate,
                LFO_SMOOTHING_SECONDS,
            ),
        };
        lfo.set_rate_hz(MIN_LFO_RATE_HZ);
        lfo.refresh_sample_and_hold();
        lfo
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        let rate = self.phase_inc.to_array()[0] * self.sample_rate;
        self.set_rate_hz(rate);
        self.output.set_sample_rate(self.sample_rate);
    }

    pub fn set_rate_hz(&mut self, rate_hz: f32) {
        let rate_hz = rate_hz.clamp(MIN_SYNCED_LFO_RATE_HZ, MAX_LFO_RATE_HZ);
        self.phase_inc = WideF32::splat(rate_hz / self.sample_rate.max(1.0));
    }

    pub fn set_depth(&mut self, depth: f32) {
        self.depth = depth.clamp(0.0, 1.0);
    }

    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    pub fn set_key_sync(&mut self, key_sync: bool) {
        self.key_sync = key_sync;
    }

    pub fn key_sync(&self) -> bool {
        self.key_sync
    }

    #[inline(always)]
    pub(crate) fn output_is_uniform(&self) -> bool {
        self.phase_uniform && self.waveform != LfoWaveform::SampleAndHold
    }

    pub fn reset_all(&mut self) {
        self.phase = WideF32::ZERO;
        self.phase_uniform = true;
        self.output.snap_all(0.0);
        if self.waveform == LfoWaveform::SampleAndHold {
            self.refresh_sample_and_hold();
        }
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.phase = self.phase.replace_lane(lane, 0.0);
        self.output.snap_lane(lane, 0.0);
        let phases = self.phase.to_array();
        self.phase_uniform = phases
            .iter()
            .all(|phase| phase.to_bits() == phases[0].to_bits());

        if self.waveform == LfoWaveform::SampleAndHold {
            self.sample_and_hold = self
                .sample_and_hold
                .replace_lane(lane, bipolar_random(&mut self.rng[lane]));
        }
    }

    pub fn next(&mut self) -> WideF32 {
        let raw = if self.phase_uniform && self.waveform != LfoWaveform::SampleAndHold {
            WideF32::splat(self.uniform_raw_output() * self.depth)
        } else {
            self.raw_output() * WideF32::splat(self.depth)
        };
        let output = self.output.next_toward(raw);
        self.advance();
        output
    }

    /// Advances phase without evaluating the waveform. This preserves latent
    /// phase and sample-and-hold RNG state for LFOs whose output is not used.
    pub(crate) fn advance_silent(&mut self) {
        self.advance();
    }

    pub fn raw_output(&self) -> WideF32 {
        if self.phase_uniform && self.waveform != LfoWaveform::SampleAndHold {
            return WideF32::splat(self.uniform_raw_output());
        }
        match self.waveform {
            LfoWaveform::Triangle => triangle(self.phase),
            LfoWaveform::Saw => self.phase,
            LfoWaveform::ReverseSaw => WideF32::splat(1.0) - self.phase,
            LfoWaveform::Square => square(self.phase),
            LfoWaveform::SampleAndHold => self.sample_and_hold,
        }
    }

    fn uniform_raw_output(&self) -> f32 {
        let phase = self.phase.to_array()[0];
        match self.waveform {
            LfoWaveform::Triangle => {
                let scaled = phase * 4.0;
                if phase < 0.25 {
                    scaled
                } else if phase < 0.75 {
                    2.0 - scaled
                } else {
                    scaled - 4.0
                }
            }
            LfoWaveform::Saw => phase,
            LfoWaveform::ReverseSaw => 1.0 - phase,
            LfoWaveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            LfoWaveform::SampleAndHold => 0.0,
        }
    }

    fn advance(&mut self) {
        if self.phase_uniform {
            let next = self.phase.to_array()[0] + self.phase_inc.to_array()[0];
            self.phase = WideF32::splat(if next < 1.0 { next } else { next - 1.0 });
            if self.waveform == LfoWaveform::SampleAndHold && next >= 1.0 {
                self.refresh_sample_and_hold();
            }
            return;
        }

        let next = self.phase + self.phase_inc;
        // LFO phase and its clamped increment are non-negative and their sum
        // is below two, so wrapping needs only one exact subtraction. Avoid
        // the considerably more expensive four-lane floor operation on Daisy.
        self.phase = next
            .simd_lt(WideF32::splat(1.0))
            .blend(next, next - WideF32::splat(1.0));

        if self.waveform == LfoWaveform::SampleAndHold {
            let next_lanes = next.to_array();
            let mut sample_and_hold = self.sample_and_hold.to_array();
            for lane in 0..WideF32::LANES {
                if next_lanes[lane] >= 1.0 {
                    sample_and_hold[lane] = bipolar_random(&mut self.rng[lane]);
                }
            }
            self.sample_and_hold = WideF32::new(sample_and_hold);
        }
    }

    fn refresh_sample_and_hold(&mut self) {
        self.sample_and_hold = WideF32::new(core::array::from_fn(|lane| {
            bipolar_random(&mut self.rng[lane])
        }));
    }
}

fn triangle(phase: WideF32) -> WideF32 {
    let four = WideF32::splat(4.0);
    let scaled = phase * four;
    let rising = scaled;
    let falling = WideF32::splat(2.0) - scaled;
    let wrapping = scaled - four;
    phase.simd_lt(WideF32::splat(0.25)).blend(
        rising,
        phase.simd_lt(WideF32::splat(0.75)).blend(falling, wrapping),
    )
}

fn square(phase: WideF32) -> WideF32 {
    phase
        .simd_lt(WideF32::splat(0.5))
        .blend(WideF32::splat(1.0), WideF32::ZERO)
}

fn bipolar_random(rng: &mut DspRng) -> f32 {
    rng.f32() * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_lane(value: WideF32) -> f32 {
        value.to_array()[0]
    }

    #[test]
    fn uniform_phase_fast_path_matches_four_lane_path_bit_exactly() {
        for waveform in [
            LfoWaveform::Triangle,
            LfoWaveform::Saw,
            LfoWaveform::ReverseSaw,
            LfoWaveform::Square,
            LfoWaveform::SampleAndHold,
        ] {
            let mut fast = Lfo::new(48_000.0);
            let mut reference = Lfo::new(48_000.0);
            for lfo in [&mut fast, &mut reference] {
                lfo.set_rate_hz(499.0);
                lfo.set_depth(0.73);
                lfo.set_waveform(waveform);
            }
            reference.phase_uniform = false;

            for _ in 0..4096 {
                assert_eq!(
                    fast.next().to_array().map(f32::to_bits),
                    reference.next().to_array().map(f32::to_bits),
                    "uniform fast path changed {waveform:?} output"
                );
            }
        }
    }

    #[test]
    fn triangle_is_bipolar() {
        let mut lfo = Lfo::new(4.0);
        lfo.set_rate_hz(1.0);
        lfo.set_depth(1.0);
        lfo.set_waveform(LfoWaveform::Triangle);

        let values = [
            first_lane(lfo.next()),
            first_lane(lfo.next()),
            first_lane(lfo.next()),
            first_lane(lfo.next()),
        ];

        assert_eq!(values, [0.0, 1.0, 0.0, -1.0]);
    }

    #[test]
    fn saw_square_and_reverse_saw_are_positive_only() {
        for waveform in [
            LfoWaveform::Saw,
            LfoWaveform::ReverseSaw,
            LfoWaveform::Square,
        ] {
            let mut lfo = Lfo::new(16.0);
            lfo.set_rate_hz(1.0);
            lfo.set_depth(1.0);
            lfo.set_waveform(waveform);

            for _ in 0..32 {
                let value = first_lane(lfo.next());
                assert!(
                    (0.0..=1.0).contains(&value),
                    "{waveform:?} produced {value}"
                );
            }
        }
    }

    #[test]
    fn sample_and_hold_updates_once_per_cycle() {
        let mut lfo = Lfo::new(4.0);
        lfo.set_rate_hz(1.0);
        lfo.set_depth(1.0);
        lfo.set_waveform(LfoWaveform::SampleAndHold);

        let held = first_lane(lfo.next());
        for _ in 0..3 {
            assert_eq!(first_lane(lfo.next()), held);
        }
        assert_ne!(first_lane(lfo.next()), held);
    }

    #[test]
    fn max_audio_rate_remains_finite() {
        let mut lfo = Lfo::new(44_100.0);
        lfo.set_rate_hz(500.0);
        lfo.set_depth(1.0);

        for _ in 0..44_100 {
            let value = first_lane(lfo.next());
            assert!(value.is_finite());
            assert!((-1.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn discontinuous_waveforms_have_no_large_per_sample_jumps() {
        let sample_rate = 48_000.0;
        let rates = [1.0, 10.0, 50.0, 200.0, 500.0];

        for waveform in [
            LfoWaveform::Saw,
            LfoWaveform::ReverseSaw,
            LfoWaveform::Square,
        ] {
            for &rate in &rates {
                let mut lfo = Lfo::new(sample_rate);
                lfo.set_rate_hz(rate);
                lfo.set_depth(1.0);
                lfo.set_waveform(waveform);

                let mut prev = first_lane(lfo.next());
                let mut max_diff: f32 = 0.0;
                let cycles = (sample_rate / rate * 3.0) as usize;

                for _ in 1..cycles {
                    let value = first_lane(lfo.next());
                    let diff = (value - prev).abs();
                    max_diff = max_diff.max(diff);
                    prev = value;
                }

                // Normal slope for a saw at this rate is rate/sample_rate.
                // A smoothed discontinuity should not exceed a small absolute step.
                let normal_slope = rate / sample_rate;
                let abs_limit = (0.01f32).max(normal_slope * 5.0);

                assert!(
                    max_diff <= abs_limit,
                    "{waveform:?} at {rate} Hz: max per-sample jump {:.6} exceeds limit {:.6} (normal slope {:.6})",
                    max_diff,
                    abs_limit,
                    normal_slope,
                );
            }
        }
    }
}
