//! Corrected-tuning Huovilainen nonlinear ladder reference.

use crate::f32x4;

use crate::dsp::filter::{
    FilterAlgorithm, FilterFrame, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ, SELF_OSC_RESONANCE_START,
};

const TWO_POLE_MAX_RESONANCE: f32 = 1.5;
const FOUR_POLE_MAX_LINEAR_RESONANCE: f32 = 3.75;
const FOUR_POLE_SELF_OSC_START_RESONANCE: f32 = 4.07;
const FOUR_POLE_SELF_OSC_MAX_RESONANCE: f32 = 4.57;
const FOUR_POLE_MAX_EFFECTIVE_RESONANCE: f32 = 4.85;
const RESONANCE_BASS_COMP: f32 = 1.22;
const THERMAL_DRIVE: f32 = 0.238;
const SELF_OSC_MAKEUP_CORRECTION_SCALE: f32 = 4.0;
const SELF_OSC_MAX_OUTPUT_MAKEUP: f32 = 1.25;
const SELF_OSC_PITCH_TUNING_CENTS: f32 = 64.0;
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
const RATIONAL_TANH_INPUT_LIMIT: f32 = 3.2;

#[derive(Clone, Copy, Debug, Default)]
struct StaticCoefficientCache {
    key: [u32; 2],
    tune: f32,
    amplitude_correction: f32,
}

#[derive(Clone, Copy)]
struct Coefficients {
    tune: f32x4,
    amplitude_correction: f32x4,
}

/// Four Euler-integrated nonlinear ladder stages with a half-sample feedback delay.
pub(super) struct HuovilainenLadder {
    self_osc_pitch_tuning_cents: f32,
    static_coefficient_cache: StaticCoefficientCache,
    stage: [f32x4; 4],
    saturated_stage: [f32x4; 3],
    half_sample_feedback: f32x4,
}

impl Default for HuovilainenLadder {
    fn default() -> Self {
        Self {
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            static_coefficient_cache: StaticCoefficientCache::default(),
            stage: [f32x4::splat(0.0); 4],
            saturated_stage: [f32x4::splat(0.0); 3],
            half_sample_feedback: f32x4::splat(0.0),
        }
    }
}

impl HuovilainenLadder {
    fn reset(&mut self) {
        self.stage = [f32x4::splat(0.0); 4];
        self.saturated_stage = [f32x4::splat(0.0); 3];
        self.half_sample_feedback = f32x4::splat(0.0);
    }

    fn reset_lane(&mut self, lane: usize) {
        for state in self
            .stage
            .iter_mut()
            .chain(self.saturated_stage.iter_mut())
            .chain(core::iter::once(&mut self.half_sample_feedback))
        {
            let mut values = state.to_array();
            values[lane] = 0.0;
            *state = f32x4::new(values);
        }
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        // The paper recommends at least 2x, but the experiment's quality
        // setting is global: Off remains 1x and never changes with resonance.
        let factor = frame.oversampling.factor(frame.sample_rate);
        let processing_rate = frame.sample_rate * factor as f32;
        let coefficients = self.coefficients(frame, processing_rate);
        let mut output = f32x4::splat(0.0);
        for _ in 0..factor {
            output = self.process_subsample(frame, coefficients);
        }
        output
    }

    fn process_subsample(&mut self, frame: FilterFrame, coefficients: Coefficients) -> f32x4 {
        let amount = if frame.poles == 4 {
            self_oscillation_amount(frame.resonance_control)
        } else {
            f32x4::splat(0.0)
        };
        let feedback = if frame.poles == 2 {
            frame.shaped_resonance * f32x4::splat(TWO_POLE_MAX_RESONANCE)
        } else {
            self_oscillation_feedback(
                frame.shaped_resonance * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE),
                amount,
            )
        } * coefficients.amplitude_correction;
        let feedback = if frame.poles == 4 {
            // The Euler ladder develops a strong sub-Nyquist limit cycle when
            // the published amplitude correction is combined with the
            // experiment's extended resonance range at high 1x cutoffs.
            feedback.min(f32x4::splat(FOUR_POLE_MAX_EFFECTIVE_RESONANCE))
        } else {
            feedback
        };
        let compensated_input = if frame.poles == 4 {
            frame.input
                * (f32x4::splat(1.0)
                    + frame.shaped_resonance
                        * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            frame.input
        };
        let input = compensated_input - feedback * self.half_sample_feedback
            + self_oscillation_excitation(amount);
        let previous_tap = if frame.poles == 2 {
            self.stage[1]
        } else {
            self.stage[3]
        };

        // Huovilainen's serial cascade uses five saturation evaluations: the
        // differential input, the three new intermediate stages, and the old
        // fourth stage. Saturated intermediate states are cached between steps.
        let saturated_input = normalized_rational_tanh(input);
        let y0 = self.stage[0] + coefficients.tune * (saturated_input - self.saturated_stage[0]);
        let saturated_y0 = normalized_rational_tanh(y0);
        let y1 = self.stage[1] + coefficients.tune * (saturated_y0 - self.saturated_stage[1]);
        let saturated_y1 = normalized_rational_tanh(y1);
        let y2 = self.stage[2] + coefficients.tune * (saturated_y1 - self.saturated_stage[2]);
        let saturated_y2 = normalized_rational_tanh(y2);
        let y3 = self.stage[3]
            + coefficients.tune * (saturated_y2 - normalized_rational_tanh(self.stage[3]));

        self.stage = [y0, y1, y2, y3];
        self.saturated_stage = [saturated_y0, saturated_y1, saturated_y2];
        let output = if frame.poles == 2 { y1 } else { y3 };
        self.half_sample_feedback = (output + previous_tap) * f32x4::splat(0.5);
        self.half_sample_feedback
            * self_oscillation_output_makeup(amount, coefficients.amplitude_correction)
    }

    fn coefficients(&mut self, frame: FilterFrame, processing_rate: f32) -> Coefficients {
        if frame.static_cutoff {
            let coefficients = self.static_coefficients(frame, processing_rate);
            return Coefficients {
                tune: f32x4::splat(coefficients.0),
                amplitude_correction: f32x4::splat(coefficients.1),
            };
        }

        let max_cutoff = (processing_rate * 0.45).min(MAX_CUTOFF_HZ);
        let pitch_semitones = if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control))
                * f32x4::splat(self.self_osc_pitch_tuning_cents / 100.0)
        } else {
            f32x4::splat(0.0)
        };
        let scale =
            ((frame.cutoff_mod_semitones + pitch_semitones) * f32x4::splat(1.0 / 12.0)).exp2();
        coefficients_from_cutoff(
            (f32x4::splat(frame.cutoff_hz) * scale)
                .clamp(f32x4::splat(MIN_CUTOFF_HZ), f32x4::splat(max_cutoff)),
            processing_rate,
        )
    }

    fn static_coefficients(&mut self, frame: FilterFrame, processing_rate: f32) -> (f32, f32) {
        let pitch_cents = if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control)).to_array()[0]
                * self.self_osc_pitch_tuning_cents
        } else {
            0.0
        };
        let key = [processing_rate.to_bits(), pitch_cents.to_bits()];
        if self.static_coefficient_cache.key == key {
            return (
                self.static_coefficient_cache.tune,
                self.static_coefficient_cache.amplitude_correction,
            );
        }

        let max_cutoff = (processing_rate * 0.45).min(MAX_CUTOFF_HZ);
        let cutoff = (frame.cutoff_hz * crate::math::exp2(pitch_cents / 1200.0))
            .clamp(MIN_CUTOFF_HZ, max_cutoff);
        // The published correction polynomials use cutoff / sample rate.
        // Only the exponential contains the full 2*pi factor.
        let normalized = cutoff / processing_rate;
        let frequency_correction = frequency_correction_scalar(normalized);
        let tune = 1.0
            - crate::math::powf(
                2.0,
                -core::f32::consts::TAU
                    * normalized
                    * frequency_correction
                    * core::f32::consts::LOG2_E,
            );
        let amplitude_correction = amplitude_correction_scalar(normalized);
        self.static_coefficient_cache = StaticCoefficientCache {
            key,
            tune,
            amplitude_correction,
        };
        (tune, amplitude_correction)
    }
}

impl FilterAlgorithm for HuovilainenLadder {
    fn reset(&mut self) {
        HuovilainenLadder::reset(self);
    }

    fn reset_lane(&mut self, lane: usize) {
        HuovilainenLadder::reset_lane(self, lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn clear_oversampling_state(&mut self) {}

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        HuovilainenLadder::process(self, frame)
    }
}

fn coefficients_from_cutoff(cutoff: f32x4, processing_rate: f32) -> Coefficients {
    let normalized = cutoff * f32x4::splat(1.0 / processing_rate);
    let normalized2 = normalized * normalized;
    let frequency_correction = f32x4::splat(1.873) * normalized2 * normalized
        + f32x4::splat(0.4955) * normalized2
        - f32x4::splat(0.6490) * normalized
        + f32x4::splat(0.9988);
    let exponent = -f32x4::splat(core::f32::consts::TAU * core::f32::consts::LOG2_E)
        * normalized
        * frequency_correction;
    let tune = f32x4::splat(1.0) - exponent.exp2();
    let amplitude_correction = -f32x4::splat(3.9364) * normalized2
        + f32x4::splat(1.8409) * normalized
        + f32x4::splat(0.9968);
    Coefficients {
        tune,
        amplitude_correction,
    }
}

fn frequency_correction_scalar(normalized: f32) -> f32 {
    let normalized2 = normalized * normalized;
    1.873 * normalized2 * normalized + 0.4955 * normalized2 - 0.6490 * normalized + 0.9988
}

fn amplitude_correction_scalar(normalized: f32) -> f32 {
    -3.9364 * normalized * normalized + 1.8409 * normalized + 0.9968
}

fn self_oscillation_amount(resonance_control: f32x4) -> f32x4 {
    ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
        / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
    .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
}

fn self_oscillation_feedback(linear: f32x4, amount: f32x4) -> f32x4 {
    let transition = smoothstep(amount);
    let target = f32x4::splat(FOUR_POLE_SELF_OSC_START_RESONANCE)
        + transition
            * f32x4::splat(FOUR_POLE_SELF_OSC_MAX_RESONANCE - FOUR_POLE_SELF_OSC_START_RESONANCE);
    linear + (target - linear) * transition
}

fn self_oscillation_output_makeup(amount: f32x4, amplitude_correction: f32x4) -> f32x4 {
    let maximum_makeup = f32x4::splat(1.0)
        + ((amplitude_correction - f32x4::splat(0.9968))
            * f32x4::splat(SELF_OSC_MAKEUP_CORRECTION_SCALE))
        .clamp(
            f32x4::splat(0.0),
            f32x4::splat(SELF_OSC_MAX_OUTPUT_MAKEUP - 1.0),
        );
    f32x4::splat(1.0) + smoothstep(amount) * (maximum_makeup - f32x4::splat(1.0))
}

fn self_oscillation_excitation(amount: f32x4) -> f32x4 {
    amount * amount * f32x4::splat(SELF_OSC_EXCITATION) * f32x4::new([1.0, -0.75, 0.5, -0.25])
}

fn normalized_rational_tanh(value: f32x4) -> f32x4 {
    let x = (value * f32x4::splat(THERMAL_DRIVE)).clamp(
        f32x4::splat(-RATIONAL_TANH_INPUT_LIMIT),
        f32x4::splat(RATIONAL_TANH_INPUT_LIMIT),
    );
    let x2 = x * x;
    let numerator =
        f32x4::splat(135_135.0) + x2 * (f32x4::splat(17_325.0) + x2 * (f32x4::splat(378.0) + x2));
    let denominator = f32x4::splat(135_135.0)
        + x2 * (f32x4::splat(62_370.0) + x2 * (f32x4::splat(3_150.0) + x2 * f32x4::splat(28.0)));
    x * (numerator / denominator) / f32x4::splat(THERMAL_DRIVE)
}

fn smoothstep(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    value * value * (f32x4::splat(3.0) - f32x4::splat(2.0) * value)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn published_correction_polynomials_match_reference_points() {
        assert!((frequency_correction_scalar(0.0) - 0.9988).abs() < 1.0e-7);
        assert!((amplitude_correction_scalar(0.0) - 0.9968).abs() < 1.0e-7);
        assert!((frequency_correction_scalar(0.25) - 0.896_784_4).abs() < 1.0e-6);
        assert!((amplitude_correction_scalar(0.25) - 1.211).abs() < 1.0e-6);
        assert!(amplitude_correction_scalar(0.45) > 1.0);
    }

    #[test]
    fn coefficient_mapping_uses_cutoff_over_processing_rate() {
        let coefficients = coefficients_from_cutoff(f32x4::splat(12_000.0), 48_000.0);
        let expected_tune = 1.0 - libm::expf(-core::f32::consts::TAU * 0.25 * 0.896_784_4);
        assert!((coefficients.tune.to_array()[0] - expected_tune).abs() < 1.0e-6);
        assert!((coefficients.amplitude_correction.to_array()[0] - 1.211).abs() < 1.0e-6);
    }

    #[test]
    fn rational_tanh_is_odd_bounded_and_monotonic() {
        let input = f32x4::new([-3.2, -0.5, 0.5, 3.2]);
        let output = normalized_rational_tanh(input).to_array();
        assert!(output.windows(2).all(|pair| pair[1] > pair[0]));
        assert!((output[0] + output[3]).abs() < 1.0e-6);
        assert!((output[1] + output[2]).abs() < 1.0e-6);
        assert!(
            output
                .iter()
                .all(|value| value.abs() <= 1.0 / THERMAL_DRIVE)
        );
    }

    #[test]
    fn rational_tanh_matches_normalized_libm_reference() {
        let mut maximum_error = 0.0f32;
        for index in 0..=4096 {
            let fraction = index as f32 / 4096.0;
            let input = (-RATIONAL_TANH_INPUT_LIMIT + 2.0 * RATIONAL_TANH_INPUT_LIMIT * fraction)
                / THERMAL_DRIVE;
            let actual = normalized_rational_tanh(f32x4::splat(input)).to_array()[0];
            let expected = libm::tanhf(input * THERMAL_DRIVE) / THERMAL_DRIVE;
            maximum_error = maximum_error.max((actual - expected).abs());
        }
        assert!(maximum_error < 1.0e-3, "maximum error={maximum_error}");
    }

    use crate::dsp::filter::{Filter, FilterOversampling, FilterType};
    use crate::f32x4;

    extern crate std;
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 48_000.0;
    const CUTOFF_HZ: f32 = 440.0;

    fn filter(
        filter_type: FilterType,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
    ) -> Filter {
        let mut filter = Filter::new(filter_type);
        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(poles);
        filter.set_oversampling(oversampling);
        filter
    }

    fn process(filter: &mut Filter, input: f32x4, note: f32x4, sample_rate: f32) -> f32x4 {
        filter.process(
            input,
            note,
            f32x4::splat(0.0),
            f32x4::splat(1.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            f32x4::splat(0.0),
            sample_rate,
        )
    }

    fn sine_gain(
        filter_type: FilterType,
        sample_rate: f32,
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
        amplitude: f32,
    ) -> f32 {
        let mut filter = filter(filter_type, cutoff, resonance, poles, oversampling);
        let step = core::f32::consts::TAU * frequency / sample_rate;
        let frames = (sample_rate * 0.1) as usize;
        let mut phase = 0.0f32;
        for _ in 0..frames {
            let _ = process(
                &mut filter,
                f32x4::splat(phase.sin() * amplitude),
                f32x4::splat(69.0),
                sample_rate,
            );
            phase += step;
        }
        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for _ in 0..frames {
            let sine = phase.sin();
            let output = process(
                &mut filter,
                f32x4::splat(sine * amplitude),
                f32x4::splat(69.0),
                sample_rate,
            )
            .to_array()[0];
            sin_sum += output * sine;
            cos_sum += output * phase.cos();
            phase += step;
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
    }

    fn tail(
        filter_type: FilterType,
        cutoff: f32,
        resonance: f32,
        oversampling: FilterOversampling,
        kick: bool,
    ) -> Vec<f32> {
        let mut filter = filter(filter_type, cutoff, resonance, 4, oversampling);
        if kick {
            for _ in 0..128 {
                let _ = process(
                    &mut filter,
                    f32x4::splat(0.1),
                    f32x4::splat(69.0),
                    SAMPLE_RATE,
                );
            }
        }
        let mut samples = Vec::with_capacity(48_000);
        for _ in 0..48_000 {
            samples.push(
                process(
                    &mut filter,
                    f32x4::splat(0.0),
                    f32x4::splat(69.0),
                    SAMPLE_RATE,
                )
                .to_array()[0],
            );
        }
        samples
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn pitch(samples: &[f32]) -> f32 {
        let mut crossings = 0usize;
        let mut first = None;
        let mut last = None;
        for (index, pair) in samples.windows(2).enumerate() {
            if pair[0] <= 0.0 && pair[1] > 0.0 {
                crossings += 1;
                first.get_or_insert(index);
                last = Some(index);
            }
        }
        match (first, last) {
            (Some(first), Some(last)) if crossings > 1 => {
                (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
            }
            _ => 0.0,
        }
    }

    fn projected_amplitude(samples: &[f32], frequency: f32) -> f32 {
        let step = core::f32::consts::TAU * frequency / SAMPLE_RATE;
        let mut phase = 0.0f32;
        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for &sample in samples {
            sin_sum += sample * phase.sin();
            cos_sum += sample * phase.cos();
            phase += step;
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len() as f32
    }

    #[test]
    fn huovilainen_is_available_with_corrected_response_and_slopes() {
        assert!(FilterType::HuovilainenLadder.is_implemented());
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for (poles, expected) in [(2, 11.0..=12.5), (4, 22.0..=24.5)] {
                let lower = sine_gain(
                    FilterType::HuovilainenLadder,
                    sample_rate,
                    CUTOFF_HZ * 4.0,
                    CUTOFF_HZ,
                    0.0,
                    poles,
                    FilterOversampling::Off,
                    1.0e-4,
                );
                let upper = sine_gain(
                    FilterType::HuovilainenLadder,
                    sample_rate,
                    CUTOFF_HZ * 8.0,
                    CUTOFF_HZ,
                    0.0,
                    poles,
                    FilterOversampling::Off,
                    1.0e-4,
                );
                let slope = 20.0 * (lower / upper).log10();
                assert!(
                    expected.contains(&slope),
                    "sr={sample_rate} poles={poles} slope={slope}"
                );
            }

            for frequency in [CUTOFF_HZ * 0.5, CUTOFF_HZ, CUTOFF_HZ * 2.0] {
                let gain = |filter_type| {
                    sine_gain(
                        filter_type,
                        sample_rate,
                        frequency,
                        CUTOFF_HZ,
                        if frequency == CUTOFF_HZ { 0.65 } else { 0.0 },
                        4,
                        FilterOversampling::Off,
                        1.0e-4,
                    )
                };
                let ratio = gain(FilterType::HuovilainenLadder)
                    / gain(FilterType::DistributedNewtonTpt).max(1.0e-9);
                assert!(
                    (0.93..=1.05).contains(&ratio),
                    "sr={sample_rate} frequency={frequency} ratio={ratio}"
                );
            }
        }
    }

    #[test]
    fn huovilainen_self_oscillation_is_tuned_and_harmonically_bounded() {
        for cutoff in [110.0, 220.0, 440.0, 880.0, 1760.0] {
            let baseline = tail(
                FilterType::DistributedNewtonTpt,
                cutoff,
                1.0,
                FilterOversampling::Off,
                true,
            );
            let candidate = tail(
                FilterType::HuovilainenLadder,
                cutoff,
                1.0,
                FilterOversampling::Off,
                true,
            );
            let baseline = &baseline[24_000..];
            let candidate = &candidate[24_000..];
            let baseline_rms = rms(baseline);
            let candidate_rms = rms(candidate);
            let baseline_pitch = pitch(baseline);
            let candidate_pitch = pitch(candidate);
            assert!(
                (candidate_rms / baseline_rms - 1.0).abs() < 0.08,
                "cutoff={cutoff} baseline={baseline_rms} candidate={candidate_rms}"
            );
            assert!(
                (candidate_pitch / baseline_pitch - 1.0).abs() < 0.02,
                "cutoff={cutoff} baseline={baseline_pitch} candidate={candidate_pitch}"
            );
            for harmonic in 2..=5 {
                let amplitude = projected_amplitude(candidate, candidate_pitch * harmonic as f32);
                assert!(
                    amplitude < 0.005,
                    "cutoff={cutoff} harmonic={harmonic} amplitude={amplitude}"
                );
            }
        }
    }

    #[test]
    fn huovilainen_resonance_onset_and_global_oversampling_are_smooth() {
        let gains = [0.70, 0.71, 0.72, 0.74, 0.75, 0.76, 0.80].map(|resonance| {
            sine_gain(
                FilterType::HuovilainenLadder,
                SAMPLE_RATE,
                CUTOFF_HZ,
                CUTOFF_HZ,
                resonance,
                4,
                FilterOversampling::Auto,
                0.1,
            )
        });
        assert!(
            gains.windows(2).all(|pair| pair[1] > pair[0]),
            "gains={gains:?}"
        );
        let threshold_db = 20.0 * (gains[2] / gains[1]).log10();
        let reported_db = 20.0 * (gains[4] / gains[3]).log10();
        assert!(threshold_db < 0.75, "gains={gains:?} step={threshold_db}");
        assert!(reported_db < 0.8, "gains={gains:?} step={reported_db}");

        for (resonance, range) in [
            (0.85, 0.0..0.001),
            (0.90, 0.08..0.18),
            (0.95, 0.38..0.47),
            (1.00, 0.44..0.52),
        ] {
            let samples = tail(
                FilterType::HuovilainenLadder,
                CUTOFF_HZ,
                resonance,
                FilterOversampling::Off,
                true,
            );
            let level = rms(&samples[36_000..]);
            assert!(range.contains(&level), "resonance={resonance} rms={level}");
        }

        for mode in [
            FilterOversampling::Off,
            FilterOversampling::Auto,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            let samples = tail(FilterType::HuovilainenLadder, CUTOFF_HZ, 1.0, mode, false);
            let level = rms(&samples[24_000..]);
            assert!((0.44..0.52).contains(&level), "mode={mode:?} rms={level}");
            assert!(
                samples
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() < 1.0)
            );
        }
    }

    #[test]
    fn huovilainen_two_pole_decays_and_control_grid_stays_finite() {
        let mut two_pole = filter(
            FilterType::HuovilainenLadder,
            CUTOFF_HZ,
            1.0,
            2,
            FilterOversampling::X4,
        );
        for _ in 0..128 {
            let _ = process(
                &mut two_pole,
                f32x4::splat(0.1),
                f32x4::splat(69.0),
                SAMPLE_RATE,
            );
        }
        let mut first = 0.0;
        let mut last = 0.0;
        for frame in 0..24_000 {
            let output = process(
                &mut two_pole,
                f32x4::splat(0.0),
                f32x4::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
            if frame < 2_000 {
                first += output * output;
            } else if frame >= 22_000 {
                last += output * output;
            }
        }
        assert!(last < first * 1.0e-4, "first={first} last={last}");

        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for mode in [
                FilterOversampling::Off,
                FilterOversampling::X2,
                FilterOversampling::X4,
            ] {
                for poles in [2, 4] {
                    for resonance in [0.0, 0.71, 0.9, 1.0] {
                        let mut filter = filter(
                            FilterType::HuovilainenLadder,
                            CUTOFF_HZ,
                            resonance,
                            poles,
                            mode,
                        );
                        filter.set_key_track(1.0);
                        filter.set_env_amount(1.0);
                        filter.set_audio_mod(1.0);
                        for frame in 0..256 {
                            let phase = frame as f32;
                            let output = filter.process(
                                f32x4::new([0.8, -0.8, 0.25, -0.25]),
                                f32x4::new([24.0, 60.0, 96.0, 120.0]),
                                f32x4::new([0.0, 0.33, 0.66, 1.0]),
                                f32x4::new([0.0, 0.33, 0.66, 1.0]),
                                f32x4::new([phase.sin(), phase.cos(), -phase.sin(), -phase.cos()]),
                                f32x4::new([-48.0, -12.0, 12.0, 48.0]),
                                f32x4::new([-0.2, -0.05, 0.05, 0.2]),
                                f32x4::new([-0.25, 0.0, 0.25, 0.5]),
                                sample_rate,
                            );
                            assert!(
                                output
                                    .to_array()
                                    .iter()
                                    .all(|value| value.is_finite() && value.abs() < 10.0)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn huovilainen_reset_key_tracking_and_lanes_are_independent() {
        let make = || {
            filter(
                FilterType::HuovilainenLadder,
                CUTOFF_HZ,
                0.95,
                4,
                FilterOversampling::X2,
            )
        };
        let mut mixed = make();
        let mut low = make();
        let mut high = make();
        for frame in 0..512 {
            let input = f32x4::splat((frame as f32 * 0.037).sin() * 0.1);
            let render = |filter: &mut Filter, resonance_mod| {
                filter.process(
                    input,
                    f32x4::splat(69.0),
                    f32x4::splat(0.0),
                    f32x4::splat(1.0),
                    f32x4::splat(0.0),
                    f32x4::splat(0.0),
                    resonance_mod,
                    f32x4::splat(0.0),
                    SAMPLE_RATE,
                )
            };
            let mixed_output =
                render(&mut mixed, f32x4::new([-0.25, 0.05, -0.25, 0.05])).to_array();
            let low_output = render(&mut low, f32x4::splat(-0.25)).to_array();
            let high_output = render(&mut high, f32x4::splat(0.05)).to_array();
            assert!((mixed_output[0] - low_output[0]).abs() < 1.0e-12);
            assert!((mixed_output[1] - high_output[1]).abs() < 1.0e-12);
        }
        mixed.reset_lane(2);
        let mut fresh = make();
        let reset = process(
            &mut mixed,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array();
        let fresh_output = process(
            &mut fresh,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array();
        assert_eq!(reset[2], fresh_output[2]);
        mixed.reset();
        fresh.reset();
        assert_eq!(
            process(
                &mut mixed,
                f32x4::splat(0.1),
                f32x4::splat(69.0),
                SAMPLE_RATE
            ),
            process(
                &mut fresh,
                f32x4::splat(0.1),
                f32x4::splat(69.0),
                SAMPLE_RATE
            )
        );

        for note in [36.0, 48.0, 60.0, 72.0, 84.0] {
            let mut tracked = filter(
                FilterType::HuovilainenLadder,
                110.0,
                1.0,
                4,
                FilterOversampling::Off,
            );
            tracked.set_key_track(1.0);
            let pitch_trim = tracked.self_osc_pitch_tuning_cents();
            let mut samples = Vec::with_capacity(24_000);
            for frame in 0..48_000 {
                let output = process(
                    &mut tracked,
                    f32x4::splat(0.0),
                    f32x4::splat(note),
                    SAMPLE_RATE,
                )
                .to_array()[0];
                assert!(output.is_finite() && output.abs() < 1.0);
                if frame >= 24_000 {
                    samples.push(output);
                }
            }
            let expected = 110.0 * 2.0f32.powf((note - 36.0) / 12.0 + pitch_trim / 1200.0);
            let measured = pitch(&samples);
            assert!(
                (measured / expected - 1.0).abs() < 0.05,
                "note={note} expected={expected} measured={measured}"
            );
        }
    }

    #[test]
    fn huovilainen_long_running_self_oscillation_stays_bounded() {
        let mut filter = filter(
            FilterType::HuovilainenLadder,
            CUTOFF_HZ,
            1.0,
            4,
            FilterOversampling::Off,
        );
        let mut energy = 0.0;
        for frame in 0..192_000 {
            let input = if frame < 24_000 { 0.1 } else { 0.0 };
            let output = process(
                &mut filter,
                f32x4::splat(input),
                f32x4::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
            assert!(output.is_finite() && output.abs() < 1.0);
            if frame >= 168_000 {
                energy += output * output;
            }
        }
        assert!((energy / 24_000.0).sqrt() > 0.44);
    }

    #[test]
    fn huovilainen_high_cutoff_feedback_stays_bounded() {
        for cutoff in [6_600.0, 12_000.0, 18_000.0, 20_000.0] {
            let samples = tail(
                FilterType::HuovilainenLadder,
                cutoff,
                1.0,
                FilterOversampling::Off,
                true,
            );
            let level = rms(&samples[36_000..]);
            assert!(
                samples
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() < 1.7),
                "cutoff={cutoff} level={level}"
            );
            assert!(
                (0.05..1.3).contains(&level),
                "cutoff={cutoff} level={level}"
            );
        }
    }
}
