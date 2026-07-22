//! Four-stage TPT cascade with one scalar nonlinear feedback solve.

use crate::{LANES, f32x4};

use crate::dsp::filter::{
    FilterAlgorithm, FilterFrame, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ, SELF_OSC_RESONANCE_START,
};

const TWO_POLE_MAX_RESONANCE: f32 = 1.9;
const FOUR_POLE_MAX_LINEAR_RESONANCE: f32 = 3.75;
const FOUR_POLE_SELF_OSC_START_RESONANCE: f32 = 4.05;
const FOUR_POLE_SELF_OSC_MAX_RESONANCE: f32 = 5.25;
const RESONANCE_BASS_COMP: f32 = 1.22;
const SELF_OSC_LIMITER_DRIVE: f32 = 2.1;
const SELF_OSC_OUTPUT_MAKEUP: f32 = 1.2;
const SELF_OSC_PITCH_TUNING_CENTS: f32 = 51.0;
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
const NONLINEAR_STATE_LIMIT: f32 = 8.0;
const RATIONAL_TANH_INPUT_LIMIT: f32 = 3.2;
const NONLINEAR_NEWTON_STEPS: usize = 2;
const FEEDBACK_TRANSITION_SCALE: f32 = 1.0;
const OVERSAMPLE_DECIMATOR_POLES: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
struct StaticCoefficientCache {
    /// Sample-rate and effective pitch-trim bits. Cutoff changes explicitly
    /// invalidate the cache through the runtime wrapper.
    key: [u32; 2],
    value: f32,
}

/// Rev2-inspired candidate whose only nonlinear unknown is the cascade input.
pub(super) struct ScalarFeedbackTpt {
    self_osc_pitch_tuning_cents: f32,
    static_coefficient_cache: StaticCoefficientCache,
    z: [f32x4; 4],
    oversample_decimator_z: [f32x4; OVERSAMPLE_DECIMATOR_POLES],
    excitation_seed: [u32; LANES],
}

impl Default for ScalarFeedbackTpt {
    fn default() -> Self {
        Self {
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            static_coefficient_cache: StaticCoefficientCache::default(),
            z: [f32x4::splat(0.0); 4],
            oversample_decimator_z: [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES],
            excitation_seed: [0x1234_5678, 0x8765_4321, 0x9e37_79b9, 0x7f4a_7c15],
        }
    }
}

impl ScalarFeedbackTpt {
    fn reset(&mut self) {
        self.z = [f32x4::splat(0.0); 4];
        self.clear_oversampling_state();
    }

    fn reset_lane(&mut self, lane: usize) {
        for stage in self
            .z
            .iter_mut()
            .chain(self.oversample_decimator_z.iter_mut())
        {
            let mut values = stage.to_array();
            values[lane] = 0.0;
            *stage = f32x4::new(values);
        }
    }

    fn clear_oversampling_state(&mut self) {
        self.oversample_decimator_z = [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES];
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        // The oversampling factor is fixed by the global setting and host
        // sample rate. It must not change when resonance crosses the
        // nonlinear threshold.
        let factor = frame.oversampling.factor(frame.sample_rate);

        if factor == 1 {
            let g = self.coefficients(frame, frame.sample_rate);
            return self.process_subsample(frame, g);
        }

        let oversampled_rate = frame.sample_rate * factor as f32;
        let g = self.coefficients(frame, oversampled_rate);
        let mut output = f32x4::splat(0.0);
        for _ in 0..factor {
            output = self.process_subsample(frame, g);
            output = self.decimate(output, frame.sample_rate, oversampled_rate);
        }
        output
    }

    fn process_subsample(&mut self, frame: FilterFrame, g: f32x4) -> f32x4 {
        let amount = if frame.poles == 4 {
            self_oscillation_amount(frame.resonance_control)
        } else {
            f32x4::splat(0.0)
        };
        let linear_feedback = if frame.poles == 2 {
            frame.shaped_resonance * f32x4::splat(TWO_POLE_MAX_RESONANCE)
        } else {
            frame.shaped_resonance * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE)
        };
        let feedback = if frame.poles == 4 {
            self_oscillation_feedback(linear_feedback, amount)
        } else {
            linear_feedback
        };
        let solver_amount = amount;
        let drive = if frame.poles == 4 {
            self_oscillation_drive(solver_amount)
        } else {
            f32x4::splat(0.0)
        };
        let compensated_input = if frame.poles == 4 {
            frame.input
                * (f32x4::splat(1.0)
                    + frame.shaped_resonance
                        * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            frame.input
        };
        let input = compensated_input + self.self_oscillation_excitation(solver_amount);

        let (a, b) = self.output_affine_form(g, frame.poles);
        let mut u = (input - feedback * b) / (f32x4::splat(1.0) + feedback * a);

        if frame.poles == 4 && solver_amount.simd_gt(f32x4::splat(0.0)).any() {
            for _ in 0..NONLINEAR_NEWTON_STEPS {
                let y4 = a * u + b;
                let (saturated, derivative) = rational_tanh_with_derivative(y4, drive);
                let function = u - input + feedback * saturated;
                let slope = f32x4::splat(1.0) + feedback * a * derivative;
                u = clamp_nonlinear_state(u - function / slope);
            }
        }

        let y0 = g * u + stage_offset(self.z[0], g);
        let y1 = g * y0 + stage_offset(self.z[1], g);
        let y2 = g * y1 + stage_offset(self.z[2], g);
        let y3 = g * y2 + stage_offset(self.z[3], g);
        commit_tpt_output(&mut self.z[0], y0);
        commit_tpt_output(&mut self.z[1], y1);
        commit_tpt_output(&mut self.z[2], y2);
        commit_tpt_output(&mut self.z[3], y3);

        if frame.poles == 2 {
            y1
        } else {
            y3 * self_oscillation_output_makeup(amount)
        }
    }

    /// Returns `A` and `B` for the selected cascade tap, `y = A*u + B`.
    fn output_affine_form(&self, g: f32x4, poles: u8) -> (f32x4, f32x4) {
        let s0 = stage_offset(self.z[0], g);
        let s1 = stage_offset(self.z[1], g);
        let g2 = g * g;
        if poles == 2 {
            return (g2, g * s0 + s1);
        }

        let s2 = stage_offset(self.z[2], g);
        let s3 = stage_offset(self.z[3], g);
        let g3 = g2 * g;
        let g4 = g2 * g2;
        (g4, g3 * s0 + g2 * s1 + g * s2 + s3)
    }

    fn self_oscillation_excitation(&mut self, amount: f32x4) -> f32x4 {
        let gains = (amount * amount * f32x4::splat(SELF_OSC_EXCITATION)).to_array();
        let mut output = [0.0; LANES];
        for (lane, sample) in output.iter_mut().enumerate() {
            if gains[lane] == 0.0 {
                continue;
            }
            let seed = self.excitation_seed[lane]
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            self.excitation_seed[lane] = seed;
            let normalized = ((seed >> 8) as f32) * (1.0 / 16_777_216.0);
            *sample = (normalized * 2.0 - 1.0) * gains[lane];
        }
        f32x4::new(output)
    }

    fn decimate(&mut self, output: f32x4, sample_rate: f32, oversampled_rate: f32) -> f32x4 {
        // Prime a cleared decimator from the current cascade output so a new
        // run or explicit global oversampling change does not begin with an
        // artificial drop to zero.
        let output_values = output.to_array();
        let first_state = self.oversample_decimator_z[0].to_array();
        let second_state = self.oversample_decimator_z[1].to_array();
        for lane in 0..LANES {
            if first_state[lane] == 0.0 && second_state[lane] == 0.0 {
                for state in &mut self.oversample_decimator_z {
                    let mut values = state.to_array();
                    values[lane] = output_values[lane];
                    *state = f32x4::new(values);
                }
            }
        }

        let cutoff = sample_rate * 0.45;
        let raw = crate::math::tan(core::f32::consts::PI * cutoff / oversampled_rate);
        let g = f32x4::splat(raw / (1.0 + raw));
        let mut filtered = output;
        for z in &mut self.oversample_decimator_z {
            filtered = tpt_one_pole(filtered, z, g);
        }
        filtered
    }

    fn coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> f32x4 {
        if frame.static_cutoff {
            return f32x4::splat(self.static_coefficient(frame, sample_rate));
        }

        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
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
            max_cutoff,
            sample_rate,
        )
    }

    fn static_coefficient(&mut self, frame: FilterFrame, sample_rate: f32) -> f32 {
        let pitch_cents = if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control)).to_array()[0]
                * self.self_osc_pitch_tuning_cents
        } else {
            0.0
        };
        let key = [sample_rate.to_bits(), pitch_cents.to_bits()];
        if self.static_coefficient_cache.key == key {
            return self.static_coefficient_cache.value;
        }

        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let cutoff = (frame.cutoff_hz * crate::math::exp2(pitch_cents / 1200.0))
            .clamp(MIN_CUTOFF_HZ, max_cutoff);
        let raw = crate::math::tan(core::f32::consts::PI * cutoff / sample_rate);
        let value = raw / (1.0 + raw);
        self.static_coefficient_cache = StaticCoefficientCache { key, value };
        value
    }
}

impl FilterAlgorithm for ScalarFeedbackTpt {
    fn reset(&mut self) {
        ScalarFeedbackTpt::reset(self);
    }

    fn reset_lane(&mut self, lane: usize) {
        ScalarFeedbackTpt::reset_lane(self, lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn clear_oversampling_state(&mut self) {
        ScalarFeedbackTpt::clear_oversampling_state(self);
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        ScalarFeedbackTpt::process(self, frame)
    }
}

fn self_oscillation_amount(resonance_control: f32x4) -> f32x4 {
    ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
        / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
    .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
}

fn self_oscillation_feedback(linear: f32x4, amount: f32x4) -> f32x4 {
    let target = f32x4::splat(FOUR_POLE_SELF_OSC_START_RESONANCE)
        + smoothstep(amount)
            * f32x4::splat(FOUR_POLE_SELF_OSC_MAX_RESONANCE - FOUR_POLE_SELF_OSC_START_RESONANCE);
    let transition = smoothstep(
        (amount * f32x4::splat(FEEDBACK_TRANSITION_SCALE))
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0)),
    );
    linear + (target - linear) * transition
}

fn self_oscillation_drive(amount: f32x4) -> f32x4 {
    smoothstep(amount) * f32x4::splat(SELF_OSC_LIMITER_DRIVE)
}

fn self_oscillation_output_makeup(amount: f32x4) -> f32x4 {
    f32x4::splat(1.0) + smoothstep(amount) * f32x4::splat(SELF_OSC_OUTPUT_MAKEUP - 1.0)
}

fn smoothstep(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    value * value * (f32x4::splat(3.0) - f32x4::splat(2.0) * value)
}

/// Padé tanh and its normalized derivative for `tanh(drive*y) / drive`.
///
/// Cancelling `drive` algebraically keeps the value exactly linear at zero
/// drive and avoids a branch or division near the shared threshold.
fn rational_tanh_with_derivative(value: f32x4, drive: f32x4) -> (f32x4, f32x4) {
    let safe_drive = drive.clamp(f32x4::splat(1.0e-6), f32x4::splat(SELF_OSC_LIMITER_DRIVE));
    let value_limit = f32x4::splat(RATIONAL_TANH_INPUT_LIMIT) / safe_drive;
    let unclipped_value = clamp_nonlinear_state(value);
    let inside_rational_range = unclipped_value.abs().simd_lt(value_limit);
    let value = unclipped_value.clamp(-value_limit, value_limit);
    let x = value * drive;
    let x2 = x * x;
    let numerator_scale =
        f32x4::splat(135_135.0) + x2 * (f32x4::splat(17_325.0) + x2 * (f32x4::splat(378.0) + x2));
    let denominator = f32x4::splat(135_135.0)
        + x2 * (f32x4::splat(62_370.0) + x2 * (f32x4::splat(3_150.0) + x2 * f32x4::splat(28.0)));
    let scale = numerator_scale / denominator;
    let tanh = x * scale;
    let derivative =
        inside_rational_range.blend(f32x4::splat(1.0) - tanh * tanh, f32x4::splat(0.0));
    (value * scale, derivative)
}

fn clamp_nonlinear_state(value: f32x4) -> f32x4 {
    value.clamp(
        f32x4::splat(-NONLINEAR_STATE_LIMIT),
        f32x4::splat(NONLINEAR_STATE_LIMIT),
    )
}

fn stage_offset(z: f32x4, g: f32x4) -> f32x4 {
    z * (f32x4::splat(1.0) - g)
}

fn commit_tpt_output(z: &mut f32x4, y: f32x4) {
    *z = y + (y - *z);
}

fn tpt_one_pole(input: f32x4, z: &mut f32x4, g: f32x4) -> f32x4 {
    let v = (input - *z) * g;
    let output = v + *z;
    *z = output + v;
    output
}

fn coefficients_from_cutoff(cutoff: f32x4, max_cutoff: f32, sample_rate: f32) -> f32x4 {
    let mut values = cutoff.to_array();
    for value in &mut values {
        let hz = value.clamp(MIN_CUTOFF_HZ, max_cutoff);
        *value = core::f32::consts::PI * hz / sample_rate;
    }
    let raw = f32x4::new(values).tan();
    raw / (f32x4::splat(1.0) + raw)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn rational_tanh_is_linear_at_zero_drive() {
        let input = f32x4::new([-8.0, -0.25, 0.25, 8.0]);
        let (output, derivative) = rational_tanh_with_derivative(input, f32x4::splat(0.0));
        assert_eq!(output, input);
        assert_eq!(derivative, f32x4::splat(1.0));
    }

    #[test]
    fn rational_tanh_matches_value_and_derivative_over_solver_range() {
        const SAMPLES: usize = 65_536;
        let mut maximum_value_error = 0.0f32;
        let mut maximum_derivative_error = 0.0f32;

        for start in (0..=SAMPLES).step_by(LANES) {
            let values: [f32; LANES] = core::array::from_fn(|lane| {
                let fraction = (start + lane).min(SAMPLES) as f32 / SAMPLES as f32;
                -NONLINEAR_STATE_LIMIT + 2.0 * NONLINEAR_STATE_LIMIT * fraction
            });
            for drive in [0.0, 0.05, 0.2, SELF_OSC_LIMITER_DRIVE] {
                let (actual, derivative) =
                    rational_tanh_with_derivative(f32x4::new(values), f32x4::splat(drive));
                let actual = actual.to_array();
                let derivative = derivative.to_array();
                for lane in 0..LANES {
                    let rational_input = (values[lane] * drive)
                        .clamp(-RATIONAL_TANH_INPUT_LIMIT, RATIONAL_TANH_INPUT_LIMIT);
                    let expected = if drive == 0.0 {
                        values[lane]
                    } else {
                        libm::tanhf(rational_input) / drive
                    };
                    let expected_derivative =
                        if (values[lane] * drive).abs() < RATIONAL_TANH_INPUT_LIMIT {
                            1.0 - libm::tanhf(rational_input).powi(2)
                        } else {
                            0.0
                        };
                    maximum_value_error = maximum_value_error.max((actual[lane] - expected).abs());
                    maximum_derivative_error = maximum_derivative_error
                        .max((derivative[lane] - expected_derivative).abs());
                }
            }
        }

        assert!(
            maximum_value_error <= 2.0e-4,
            "value error {maximum_value_error}"
        );
        assert!(
            maximum_derivative_error <= 2.0e-5,
            "derivative error {maximum_derivative_error}"
        );
    }

    #[test]
    fn nonlinear_solver_uses_exactly_two_newton_steps() {
        assert_eq!(NONLINEAR_NEWTON_STEPS, 2);
    }

    #[test]
    fn drive_ramp_is_gradual_across_074_to_075() {
        let controls = f32x4::new([0.73, 0.74, 0.75, 0.76]);
        let drive = self_oscillation_drive(self_oscillation_amount(controls)).to_array();
        assert!(drive.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(
            drive[2] - drive[1] < 0.1,
            "drive changes too quickly across 0.74->0.75: {drive:?}"
        );
    }

    #[test]
    fn global_oversampling_runs_below_nonlinear_threshold() {
        let mut filter = ScalarFeedbackTpt::default();
        let frame = FilterFrame {
            input: f32x4::splat(0.1),
            cutoff_hz: 440.0,
            cutoff_mod_semitones: f32x4::splat(0.0),
            cutoff_mod_uniform_semitones: Some(0.0),
            resonance_control: f32x4::splat(0.5),
            shaped_resonance: f32x4::splat(0.3),
            poles: 4,
            oversampling: crate::dsp::FilterOversampling::X4,
            sample_rate: 48_000.0,
            static_cutoff: true,
            self_oscillation_color_enabled: true,
        };
        for _ in 0..8 {
            let _ = filter.process(frame);
        }
        assert!(
            filter
                .oversample_decimator_z
                .iter()
                .any(|state| state.abs().simd_gt(f32x4::splat(0.0)).any())
        );
    }

    use crate::dsp::filter::{Filter, FilterOversampling, FilterType};
    use crate::f32x4;

    extern crate std;
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 48_000.0;
    const CUTOFF_HZ: f32 = 440.0;

    fn configured_filter(
        filter_type: FilterType,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
    ) -> Filter {
        let mut filter = Filter::new(filter_type);
        filter.set_cutoff(CUTOFF_HZ);
        filter.set_resonance(resonance);
        filter.set_poles(poles);
        filter.set_oversampling(oversampling);
        filter
    }

    fn process(filter: &mut Filter, input: f32x4, sample_rate: f32) -> f32x4 {
        filter.process(
            input,
            f32x4::splat(69.0),
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
        resonance: f32,
        poles: u8,
    ) -> f32 {
        sine_gain_with_oversampling(
            filter_type,
            sample_rate,
            frequency,
            resonance,
            poles,
            FilterOversampling::Off,
        )
    }

    fn sine_gain_with_oversampling(
        filter_type: FilterType,
        sample_rate: f32,
        frequency: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
    ) -> f32 {
        sine_gain_at_level(
            filter_type,
            sample_rate,
            frequency,
            resonance,
            poles,
            oversampling,
            1.0e-4,
        )
    }

    fn sine_gain_at_level(
        filter_type: FilterType,
        sample_rate: f32,
        frequency: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
        amplitude: f32,
    ) -> f32 {
        let mut filter = configured_filter(filter_type, resonance, poles, oversampling);
        let phase_step = core::f32::consts::TAU * frequency / sample_rate;
        let frames = (sample_rate * 0.1) as usize;
        let mut phase = 0.0f32;
        for _ in 0..frames {
            let _ = process(
                &mut filter,
                f32x4::splat(phase.sin() * amplitude),
                sample_rate,
            );
            phase += phase_step;
        }

        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for _ in 0..frames {
            let sine = phase.sin();
            let output =
                process(&mut filter, f32x4::splat(sine * amplitude), sample_rate).to_array()[0];
            sin_sum += output * sine;
            cos_sum += output * phase.cos();
            phase += phase_step;
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
    }

    fn tail_samples(
        filter_type: FilterType,
        resonance: f32,
        oversampling: FilterOversampling,
    ) -> Vec<f32> {
        let mut filter = configured_filter(filter_type, resonance, 4, oversampling);
        for _ in 0..128 {
            let _ = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
        }
        let mut samples = Vec::with_capacity(48_000);
        for _ in 0..48_000 {
            samples.push(process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0]);
        }
        samples
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn positive_crossing_pitch(samples: &[f32]) -> f32 {
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
            (Some(first), Some(last)) if crossings > 1 && last > first => {
                (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
            }
            _ => 0.0,
        }
    }

    fn projected_amplitude(samples: &[f32], frequency: f32) -> f32 {
        let phase_step = core::f32::consts::TAU * frequency / SAMPLE_RATE;
        let mut phase = 0.0f32;
        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for &sample in samples {
            sin_sum += sample * phase.sin();
            cos_sum += sample * phase.cos();
            phase += phase_step;
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len() as f32
    }

    #[test]
    fn scalar_feedback_tpt_is_available_and_has_expected_slopes() {
        assert!(FilterType::ScalarFeedbackTpt.is_implemented());
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let two_pole_lower = sine_gain(
                FilterType::ScalarFeedbackTpt,
                sample_rate,
                CUTOFF_HZ * 4.0,
                0.0,
                2,
            );
            let two_pole_upper = sine_gain(
                FilterType::ScalarFeedbackTpt,
                sample_rate,
                CUTOFF_HZ * 8.0,
                0.0,
                2,
            );
            let four_pole_lower = sine_gain(
                FilterType::ScalarFeedbackTpt,
                sample_rate,
                CUTOFF_HZ * 4.0,
                0.0,
                4,
            );
            let four_pole_upper = sine_gain(
                FilterType::ScalarFeedbackTpt,
                sample_rate,
                CUTOFF_HZ * 8.0,
                0.0,
                4,
            );
            let two_pole_db = 20.0 * (two_pole_lower / two_pole_upper).log10();
            let four_pole_db = 20.0 * (four_pole_lower / four_pole_upper).log10();
            assert!(
                (11.0..=12.5).contains(&two_pole_db),
                "sr={sample_rate} slope={two_pole_db}"
            );
            assert!(
                (22.0..=24.5).contains(&four_pole_db),
                "sr={sample_rate} slope={four_pole_db}"
            );
        }
    }

    #[test]
    fn scalar_feedback_tpt_linear_response_matches_baseline() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for poles in [2, 4] {
                for (frequency, resonance) in [
                    (CUTOFF_HZ * 0.5, 0.0),
                    (CUTOFF_HZ, 0.65),
                    (CUTOFF_HZ * 2.0, 0.0),
                ] {
                    let baseline = sine_gain(
                        FilterType::DistributedNewtonTpt,
                        sample_rate,
                        frequency,
                        resonance,
                        poles,
                    );
                    let candidate = sine_gain(
                        FilterType::ScalarFeedbackTpt,
                        sample_rate,
                        frequency,
                        resonance,
                        poles,
                    );
                    let relative_error = (candidate - baseline).abs() / baseline.max(1.0e-9);
                    assert!(
                        relative_error < 2.0e-4,
                        "sr={sample_rate} poles={poles} frequency={frequency} baseline={baseline} candidate={candidate}"
                    );
                }
            }
        }
    }

    #[test]
    fn scalar_feedback_tpt_self_oscillation_is_calibrated_to_baseline() {
        let baseline = tail_samples(
            FilterType::DistributedNewtonTpt,
            1.0,
            FilterOversampling::Off,
        );
        let candidate = tail_samples(FilterType::ScalarFeedbackTpt, 1.0, FilterOversampling::Off);
        let baseline = &baseline[24_000..];
        let candidate = &candidate[24_000..];
        let baseline_pitch = positive_crossing_pitch(baseline);
        let candidate_pitch = positive_crossing_pitch(candidate);
        let baseline_rms = rms(baseline);
        let candidate_rms = rms(candidate);
        let baseline_peak = baseline
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        let candidate_peak = candidate
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));

        assert!(
            (candidate_pitch / baseline_pitch - 1.0).abs() < 0.01,
            "baseline={baseline_pitch} candidate={candidate_pitch}"
        );
        assert!(
            (candidate_rms / baseline_rms - 1.0).abs() < 0.06,
            "baseline={baseline_rms} candidate={candidate_rms}"
        );
        assert!(
            (candidate_peak / baseline_peak - 1.0).abs() < 0.06,
            "baseline={baseline_peak} candidate={candidate_peak}"
        );

        for harmonic in 2..=5 {
            let baseline_harmonic = projected_amplitude(baseline, baseline_pitch * harmonic as f32);
            let candidate_harmonic = projected_amplitude(candidate, candidate_pitch * harmonic as f32);
            assert!(
                candidate_harmonic < 0.005,
                "harmonic={harmonic} baseline={baseline_harmonic} candidate={candidate_harmonic}"
            );
        }
    }

    #[test]
    fn scalar_feedback_tpt_self_oscillation_onset_tracks_baseline() {
        for resonance in [0.85, 0.9, 0.95, 1.0] {
            let baseline = tail_samples(
                FilterType::DistributedNewtonTpt,
                resonance,
                FilterOversampling::Off,
            );
            let candidate = tail_samples(
                FilterType::ScalarFeedbackTpt,
                resonance,
                FilterOversampling::Off,
            );
            let baseline_rms = rms(&baseline[36_000..]);
            let candidate_rms = rms(&candidate[36_000..]);
            if resonance == 0.85 {
                assert!(baseline_rms < 1.0e-3 && candidate_rms < 1.0e-3);
            } else {
                let ratio = candidate_rms / baseline_rms.max(1.0e-9);
                assert!(
                    (0.65..=1.25).contains(&ratio),
                    "resonance={resonance} baseline={baseline_rms} candidate={candidate_rms}"
                );
            }
        }
    }

    #[test]
    fn scalar_feedback_tpt_resonance_boosts_cutoff_smoothly() {
        let gains = [0.0, 0.5, 0.65, 0.7, 0.71, 0.72, 0.8].map(|resonance| {
            sine_gain_with_oversampling(
                FilterType::ScalarFeedbackTpt,
                SAMPLE_RATE,
                CUTOFF_HZ,
                resonance,
                4,
                FilterOversampling::Auto,
            )
        });
        assert!(
            gains.windows(2).all(|pair| pair[1] > pair[0]),
            "gains={gains:?}"
        );
        assert!(gains[4] > gains[0] * 7.0, "gains={gains:?}");
        let threshold_step_db = 20.0 * (gains[5] / gains[4]).log10();
        assert!(
            threshold_step_db < 0.75,
            "gains={gains:?} step={threshold_step_db}dB"
        );

        let musical = [0.73, 0.74, 0.75, 0.76].map(|resonance| {
            sine_gain_at_level(
                FilterType::ScalarFeedbackTpt,
                SAMPLE_RATE,
                CUTOFF_HZ,
                resonance,
                4,
                FilterOversampling::Auto,
                0.1,
            )
        });
        let musical_step_db = 20.0 * (musical[2] / musical[1]).log10();
        assert!(musical.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(
            musical_step_db < 0.8,
            "musical gains={musical:?} step={musical_step_db}dB"
        );
    }

    #[test]
    fn scalar_feedback_tpt_global_oversampling_does_not_switch_at_threshold() {
        let mut filter = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.7,
            4,
            FilterOversampling::Auto,
        );
        let phase_step = core::f32::consts::TAU * CUTOFF_HZ / SAMPLE_RATE;
        let mut phase = 0.0f32;
        let mut previous = 0.0f32;
        for _ in 0..24_000 {
            previous = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
            phase += phase_step;
        }
        let mut found_peak = previous.abs() >= 0.12;
        for _ in 0..24_000 {
            if found_peak {
                break;
            }
            previous = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
            phase += phase_step;
            found_peak = previous.abs() >= 0.12;
        }
        assert!(
            found_peak,
            "resonant signal never reached the expected level"
        );

        filter.set_resonance(0.72);
        let crossed = process(&mut filter, f32x4::splat(phase.sin() * 0.1), SAMPLE_RATE).to_array()[0];
        assert!(
            (crossed - previous).abs() < 0.04,
            "threshold crossing dropped or jumped: before={previous} after={crossed}"
        );
    }

    #[test]
    fn scalar_feedback_tpt_auto_self_oscillates_from_silence() {
        let mut filter = configured_filter(
            FilterType::ScalarFeedbackTpt,
            1.0,
            4,
            FilterOversampling::Auto,
        );
        let mut energy = 0.0;
        let mut peak = 0.0f32;
        for frame in 0..96_000 {
            let output = process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0];
            assert!(output.is_finite());
            if frame >= 72_000 {
                energy += output * output;
                peak = peak.max(output.abs());
            }
        }
        let tail_rms = (energy / 24_000.0).sqrt();
        assert!(tail_rms > 0.4, "rms={tail_rms} peak={peak}");
        assert!(peak > 0.6 && peak < 1.0, "rms={tail_rms} peak={peak}");
    }

    #[test]
    fn scalar_feedback_tpt_oversampling_modes_are_stable() {
        for oversampling in [
            FilterOversampling::Off,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            let samples = tail_samples(FilterType::ScalarFeedbackTpt, 1.0, oversampling);
            let tail = &samples[24_000..];
            let tail_rms = rms(tail);
            let peak = tail
                .iter()
                .fold(0.0f32, |peak, value| peak.max(value.abs()));
            assert!(tail.iter().all(|value| value.is_finite()));
            assert!(
                (0.1..1.5).contains(&tail_rms),
                "mode={oversampling:?} rms={tail_rms}"
            );
            assert!(peak < 2.0, "mode={oversampling:?} peak={peak}");
        }
    }

    #[test]
    fn scalar_feedback_tpt_two_pole_resonance_decays() {
        let mut filter = configured_filter(
            FilterType::ScalarFeedbackTpt,
            1.0,
            2,
            FilterOversampling::X4,
        );
        for _ in 0..128 {
            let _ = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
        }
        let mut first_energy = 0.0;
        let mut last_energy = 0.0;
        for frame in 0..24_000 {
            let output = process(&mut filter, f32x4::splat(0.0), SAMPLE_RATE).to_array()[0];
            if frame < 2_000 {
                first_energy += output * output;
            } else if frame >= 22_000 {
                last_energy += output * output;
            }
        }
        assert!(
            last_energy < first_energy * 1.0e-4,
            "first={first_energy} last={last_energy}"
        );
    }

    #[test]
    fn scalar_feedback_tpt_remains_finite_across_control_grid() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for oversampling in [
                FilterOversampling::Off,
                FilterOversampling::X2,
                FilterOversampling::X4,
            ] {
                for poles in [2, 4] {
                    for resonance in [0.0, 0.71, 0.9, 1.0] {
                        let mut filter = configured_filter(
                            FilterType::ScalarFeedbackTpt,
                            resonance,
                            poles,
                            oversampling,
                        );
                        filter.set_key_track(1.0);
                        filter.set_env_amount(1.0);
                        filter.set_env_velocity_amount(1.0);
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
                            for value in output.to_array() {
                                assert!(
                                    value.is_finite() && value.abs() < 10.0,
                                    "sr={sample_rate} mode={oversampling:?} poles={poles} resonance={resonance} frame={frame} output={value}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scalar_feedback_tpt_reset_and_simd_lanes_are_independent() {
        let mut filter = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.6,
            4,
            FilterOversampling::Off,
        );
        let mut fresh = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.6,
            4,
            FilterOversampling::Off,
        );
        for _ in 0..128 {
            let _ = process(&mut filter, f32x4::new([0.2, -0.1, 0.4, -0.3]), SAMPLE_RATE);
        }
        filter.reset_lane(2);
        let reset_lane = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE).to_array();
        let fresh_lane = process(&mut fresh, f32x4::splat(0.1), SAMPLE_RATE).to_array();
        assert_eq!(reset_lane[2], fresh_lane[2]);
        assert_ne!(reset_lane[0], fresh_lane[0]);

        filter.reset();
        fresh.reset();
        let reset = process(&mut filter, f32x4::splat(0.1), SAMPLE_RATE);
        let fresh = process(&mut fresh, f32x4::splat(0.1), SAMPLE_RATE);
        assert_eq!(reset, fresh);
        let lanes = reset.to_array();
        assert!(lanes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn scalar_feedback_tpt_mixed_lane_oversampling_is_independent() {
        let mut mixed = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.6,
            4,
            FilterOversampling::X4,
        );
        let mut linear = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.6,
            4,
            FilterOversampling::X4,
        );
        let mut nonlinear = configured_filter(
            FilterType::ScalarFeedbackTpt,
            0.6,
            4,
            FilterOversampling::X4,
        );

        for frame in 0..512 {
            let input = f32x4::splat((frame as f32 * 0.037).sin() * 0.1);
            let render = |filter: &mut Filter, resonance_mod: f32x4| {
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
            let mixed_output = render(&mut mixed, f32x4::new([0.0, 0.4, 0.0, 0.4])).to_array();
            let linear_output = render(&mut linear, f32x4::splat(0.0)).to_array();
            let nonlinear_output = render(&mut nonlinear, f32x4::splat(0.4)).to_array();
            assert!(
                (mixed_output[0] - linear_output[0]).abs() < 1.0e-6,
                "linear lane frame={frame} mixed={} reference={}",
                mixed_output[0],
                linear_output[0]
            );
            assert!(
                (mixed_output[2] - linear_output[2]).abs() < 1.0e-6,
                "linear lane frame={frame} mixed={} reference={}",
                mixed_output[2],
                linear_output[2]
            );
            assert_eq!(
                mixed_output[1], nonlinear_output[1],
                "nonlinear lane frame={frame}"
            );
            assert_eq!(
                mixed_output[3], nonlinear_output[3],
                "nonlinear lane frame={frame}"
            );
        }
    }
}
