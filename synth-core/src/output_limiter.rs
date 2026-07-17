//! Low-cost final-output peak protection.

/// Fixed lookahead keeps the limiter independent of allocation and host block size.
const LOOKAHEAD_SAMPLES: usize = 64;
/// Leaves margin for rounding and downstream sample conversion.
const OUTPUT_CEILING: f32 = 0.95;
const RELEASE_SECONDS: f32 = 0.1;
/// Number of exponential time constants needed to recover 99.9% in the
/// configured release time.
const RELEASE_TIME_CONSTANTS: f32 = 6.907_755_4;

/// Stereo-linked lookahead limiter used only as a final clipping safeguard.
///
/// Normal signals pass unchanged apart from the fixed lookahead delay. When
/// either channel exceeds the ceiling, both channels receive the same gain so
/// the stereo image and relative note levels remain intact.
pub(crate) struct OutputLimiter {
    left_delay: [f32; LOOKAHEAD_SAMPLES],
    right_delay: [f32; LOOKAHEAD_SAMPLES],
    write_index: usize,
    gain: f32,
    attack_target: f32,
    attack_step: f32,
    hold_samples: usize,
    release_coefficient: f32,
}

impl OutputLimiter {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let release_samples = (sample_rate.max(1.0) * RELEASE_SECONDS).max(1.0);
        let release_coefficient = (RELEASE_TIME_CONSTANTS / release_samples).min(1.0);

        Self {
            left_delay: [0.0; LOOKAHEAD_SAMPLES],
            right_delay: [0.0; LOOKAHEAD_SAMPLES],
            write_index: 0,
            gain: 1.0,
            attack_target: 1.0,
            attack_step: 0.0,
            hold_samples: 0,
            release_coefficient,
        }
    }

    #[inline(always)]
    pub(crate) fn next(&mut self, left: f32, right: f32) -> (f32, f32) {
        let delayed_left = self.left_delay[self.write_index];
        let delayed_right = self.right_delay[self.write_index];
        self.left_delay[self.write_index] = left;
        self.right_delay[self.write_index] = right;
        self.write_index += 1;
        if self.write_index == LOOKAHEAD_SAMPLES {
            self.write_index = 0;
        }

        let peak = left.abs().max(right.abs());
        let over_ceiling = peak > OUTPUT_CEILING;
        let can_release = self.hold_samples == 0 && !over_ceiling;
        if over_ceiling {
            let target_gain = OUTPUT_CEILING / peak;
            self.hold_samples = LOOKAHEAD_SAMPLES;
            self.schedule_attack(target_gain);
        } else if self.hold_samples > 0 {
            self.hold_samples -= 1;
        }

        if self.attack_target < self.gain {
            self.gain = (self.gain + self.attack_step).max(self.attack_target);
            if self.gain <= self.attack_target {
                self.attack_target = 1.0;
                self.attack_step = 0.0;
            }
        } else if can_release && self.gain < 1.0 {
            self.gain += (1.0 - self.gain) * self.release_coefficient;
            if self.gain > 1.0 - f32::EPSILON {
                self.gain = 1.0;
            }
        }

        (delayed_left * self.gain, delayed_right * self.gain)
    }

    #[inline(always)]
    fn schedule_attack(&mut self, target_gain: f32) {
        if target_gain >= self.gain {
            return;
        }

        let required_step = (target_gain - self.gain) / LOOKAHEAD_SAMPLES as f32;
        if self.attack_target >= self.gain {
            self.attack_target = target_gain;
            self.attack_step = required_step;
            return;
        }

        // A new peak may steepen an in-flight ramp, but must never relax the
        // slope needed to meet an earlier peak's deadline.
        self.attack_target = self.attack_target.min(target_gain);
        self.attack_step = self.attack_step.min(required_step);
    }
}

#[cfg(test)]
mod tests {
    use super::{LOOKAHEAD_SAMPLES, OUTPUT_CEILING, OutputLimiter};

    const SAMPLE_RATE: f32 = 48_000.0;

    #[test]
    fn below_ceiling_is_unchanged_after_fixed_delay() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        let mut output = [(0.0, 0.0); LOOKAHEAD_SAMPLES + 4];
        let input = [(0.1, -0.2), (0.3, 0.4), (-0.5, 0.25), (0.9, -0.8)];

        for (index, frame) in output.iter_mut().enumerate() {
            *frame = limiter.next(
                if index < input.len() {
                    input[index].0
                } else {
                    0.0
                },
                if index < input.len() {
                    input[index].1
                } else {
                    0.0
                },
            );
        }

        assert!(
            output[..LOOKAHEAD_SAMPLES]
                .iter()
                .all(|frame| *frame == (0.0, 0.0))
        );
        assert_eq!(&output[LOOKAHEAD_SAMPLES..], &input);
        assert_eq!(limiter.gain, 1.0);
    }

    #[test]
    fn linked_gain_preserves_stereo_ratio_and_limits_peak() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        let input = (1.9, -0.475);
        limiter.next(input.0, input.1);

        let mut limited = (0.0, 0.0);
        for _ in 0..LOOKAHEAD_SAMPLES {
            limited = limiter.next(0.0, 0.0);
        }

        assert!((limited.0 - OUTPUT_CEILING).abs() < 1.0e-6);
        assert!((limited.1 / limited.0 - input.1 / input.0).abs() < 1.0e-6);
    }

    #[test]
    fn attack_is_spread_across_the_lookahead_window() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        let target_gain = 0.5;
        let input_peak = OUTPUT_CEILING / target_gain;
        let expected_step = (1.0 - target_gain) / LOOKAHEAD_SAMPLES as f32;
        let mut previous_gain = 1.0;

        for index in 0..LOOKAHEAD_SAMPLES {
            limiter.next(if index == 0 { input_peak } else { 0.0 }, 0.0);
            let gain_drop = previous_gain - limiter.gain;
            assert!(
                gain_drop <= expected_step + 1.0e-6,
                "attack jumped by {gain_drop} at sample {index}"
            );
            previous_gain = limiter.gain;
        }

        assert!((limiter.gain - target_gain).abs() < 1.0e-6);
        let (limited, _) = limiter.next(0.0, 0.0);
        assert!((limited - OUTPUT_CEILING).abs() < 1.0e-6);
    }

    #[test]
    fn later_milder_peak_does_not_relax_an_earlier_attack_deadline() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        limiter.next(OUTPUT_CEILING / 0.5, 0.0);
        let original_step = limiter.attack_step;

        for _ in 1..LOOKAHEAD_SAMPLES / 2 {
            limiter.next(0.0, 0.0);
        }
        limiter.next(OUTPUT_CEILING / 0.6, 0.0);

        assert!(
            limiter.attack_step <= original_step,
            "later peak relaxed attack from {original_step} to {}",
            limiter.attack_step
        );
        for _ in LOOKAHEAD_SAMPLES / 2..LOOKAHEAD_SAMPLES {
            limiter.next(0.0, 0.0);
        }

        assert!(limiter.gain <= 0.5 + 1.0e-6, "gain={}", limiter.gain);
    }

    #[test]
    fn smoothed_attack_suppresses_high_frequency_gain_sidebands() {
        const FFT_SIZE: usize = 4096;
        const ANALYSIS_BIN: usize = 512;
        const SOURCE_BINS: [usize; 6] = [6, 7, 8, 38, 48, 57];
        const SETTLE_SAMPLES: usize = SAMPLE_RATE as usize * 2;

        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        let mut real = 0.0f32;
        let mut imaginary = 0.0f32;
        let mut maximum = 0.0f32;

        for index in 0..SETTLE_SAMPLES + FFT_SIZE {
            let mut input = 0.0f32;
            for (source, bin) in SOURCE_BINS.iter().copied().enumerate() {
                let phase =
                    crate::TAU * bin as f32 * index as f32 / FFT_SIZE as f32 + source as f32 * 0.37;
                input += libm::sinf(phase);
            }
            input *= 0.28;

            let (output, _) = limiter.next(input, -input * 0.75);
            maximum = maximum.max(output.abs());
            if index >= SETTLE_SAMPLES {
                let analysis_index = index - SETTLE_SAMPLES;
                let phase =
                    crate::TAU * ANALYSIS_BIN as f32 * analysis_index as f32 / FFT_SIZE as f32;
                real += output * libm::cosf(phase);
                imaginary -= output * libm::sinf(phase);
            }
        }

        let magnitude = libm::sqrtf(real * real + imaginary * imaginary) / FFT_SIZE as f32;
        let sideband_db = 20.0 * libm::log10f(magnitude.max(1.0e-15));
        assert!(
            sideband_db < -108.0,
            "6 kHz limiter sideband remained audible in the analyzer: {sideband_db} dB"
        );
        assert!(maximum <= OUTPUT_CEILING + 1.0e-6, "peak={maximum}");
    }

    #[test]
    fn repeated_over_ceiling_samples_never_escape_during_release() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        let total = LOOKAHEAD_SAMPLES + SAMPLE_RATE as usize;
        let mut maximum = 0.0f32;

        for index in 0..total {
            let input = if index % 97 == 0 { 1.5 } else { 0.2 };
            let (left, right) = limiter.next(input, -input);
            maximum = maximum.max(left.abs()).max(right.abs());
        }

        assert!(
            maximum <= OUTPUT_CEILING + 1.0e-6,
            "peak escaped limiter: {maximum}"
        );
    }

    #[test]
    fn gain_recovers_after_release_window() {
        let mut limiter = OutputLimiter::new(SAMPLE_RATE);
        limiter.next(1.9, 0.0);
        assert!(limiter.gain < 1.0);

        for _ in 0..(LOOKAHEAD_SAMPLES + SAMPLE_RATE as usize / 10) {
            limiter.next(0.0, 0.0);
        }

        assert!(
            limiter.gain > 0.999,
            "gain did not recover: {}",
            limiter.gain
        );
    }
}
