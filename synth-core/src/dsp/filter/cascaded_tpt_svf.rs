//! Cascaded trapezoidal state-variable filter reference.

use crate::{LANES, f32x4};

use crate::dsp::filter::{
    FilterAlgorithm, FilterFrame, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ, SELF_OSC_RESONANCE_START,
};

const TWO_POLE_DAMPING: f32 = core::f32::consts::SQRT_2;
const FOUR_POLE_FIRST_DAMPING: f32 = 1.847_759_1;
const FOUR_POLE_SECOND_DAMPING: f32 = 0.765_366_85;
const TWO_POLE_MAX_FEEDBACK: f32 = 1.5;
const FOUR_POLE_MAX_LINEAR_FEEDBACK: f32 = 1.25;
const FOUR_POLE_SELF_OSC_START_FEEDBACK: f32 = 1.42;
const FOUR_POLE_SELF_OSC_MAX_FEEDBACK: f32 = 2.4;
const SELF_OSC_LIMITER_DRIVE: f32 = 2.1;
const SELF_OSC_OUTPUT_MAKEUP: f32 = 0.765;
const SELF_OSC_PITCH_TUNING_CENTS: f32 = 51.0;
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
const NONLINEAR_STATE_LIMIT: f32 = 8.0;
const RATIONAL_TANH_INPUT_LIMIT: f32 = 3.2;
const NONLINEAR_NEWTON_STEPS: usize = 2;
const OVERSAMPLE_DECIMATOR_POLES: usize = 2;

#[derive(Clone, Copy)]
struct SectionCoefficients {
    a1: f32x4,
    a2: f32x4,
    a3: f32x4,
}

impl SectionCoefficients {
    fn splat(values: [f32; 3]) -> Self {
        Self {
            a1: f32x4::splat(values[0]),
            a2: f32x4::splat(values[1]),
            a3: f32x4::splat(values[2]),
        }
    }
}

#[derive(Clone, Copy)]
struct Coefficients {
    two_pole: SectionCoefficients,
    four_pole: [SectionCoefficients; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct StaticCoefficientCache {
    /// Sample-rate and effective pitch-trim bits. Cutoff changes explicitly
    /// invalidate the cache through the runtime wrapper.
    key: [u32; 2],
    two_pole: [f32; 3],
    four_pole_first: [f32; 3],
    four_pole_second: [f32; 3],
}

#[derive(Clone, Copy, Default)]
struct SvfState {
    integrator_1: f32x4,
    integrator_2: f32x4,
}

/// One SVF for 2-pole mode and two Butterworth-damped SVFs for 4-pole mode.
pub(super) struct CascadedTptSvf {
    self_osc_pitch_tuning_cents: f32,
    static_coefficient_cache: StaticCoefficientCache,
    section: [SvfState; 2],
    oversample_decimator_z: [f32x4; OVERSAMPLE_DECIMATOR_POLES],
    excitation_seed: [u32; LANES],
}

impl Default for CascadedTptSvf {
    fn default() -> Self {
        Self {
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            static_coefficient_cache: StaticCoefficientCache::default(),
            section: [SvfState::default(); 2],
            oversample_decimator_z: [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES],
            excitation_seed: [0x1234_5678, 0x8765_4321, 0x9e37_79b9, 0x7f4a_7c15],
        }
    }
}

impl CascadedTptSvf {
    fn reset(&mut self) {
        self.section = [SvfState::default(); 2];
        self.clear_oversampling_state();
    }

    fn reset_lane(&mut self, lane: usize) {
        for state in &mut self.section {
            reset_vector_lane(&mut state.integrator_1, lane);
            reset_vector_lane(&mut state.integrator_2, lane);
        }
        for state in &mut self.oversample_decimator_z {
            reset_vector_lane(state, lane);
        }
    }

    fn clear_oversampling_state(&mut self) {
        self.oversample_decimator_z = [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES];
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        // The global quality setting fixes this factor for the complete run;
        // resonance never changes the selected path.
        let factor = frame.oversampling.factor(frame.sample_rate);
        if factor == 1 {
            let coefficients = self.coefficients(frame, frame.sample_rate);
            return self.process_subsample(frame, coefficients);
        }

        let oversampled_rate = frame.sample_rate * factor as f32;
        let coefficients = self.coefficients(frame, oversampled_rate);
        let mut output = f32x4::splat(0.0);
        for _ in 0..factor {
            output = self.process_subsample(frame, coefficients);
            output = self.decimate(output, frame.sample_rate, oversampled_rate);
        }
        output
    }

    fn process_subsample(&mut self, frame: FilterFrame, coefficients: Coefficients) -> f32x4 {
        let amount = if frame.poles == 4 {
            self_oscillation_amount(frame.resonance_control)
        } else {
            f32x4::splat(0.0)
        };
        let linear_feedback = if frame.poles == 2 {
            frame.shaped_resonance * f32x4::splat(TWO_POLE_MAX_FEEDBACK)
        } else {
            frame.shaped_resonance * f32x4::splat(FOUR_POLE_MAX_LINEAR_FEEDBACK)
        };
        let feedback = if frame.poles == 4 {
            self_oscillation_feedback(linear_feedback, amount)
        } else {
            linear_feedback
        };
        let drive = if frame.poles == 4 {
            smoothstep(amount) * f32x4::splat(SELF_OSC_LIMITER_DRIVE)
        } else {
            f32x4::splat(0.0)
        };

        // Every low-pass section has unity DC gain. Multiplying the external
        // input by 1 + feedback exactly restores unity DC gain in the linear
        // outer loop without changing the affine solve.
        let input =
            frame.input * (f32x4::splat(1.0) + feedback) + self.self_oscillation_excitation(amount);
        let (a, b) = self.output_affine_form(frame.poles, coefficients);
        let mut u = (input - feedback * b) / (f32x4::splat(1.0) + feedback * a);

        if frame.poles == 4 && amount.simd_gt(f32x4::splat(0.0)).any() {
            for _ in 0..NONLINEAR_NEWTON_STEPS {
                let output = a * u + b;
                let (saturated, derivative) = rational_tanh_with_derivative(output, drive);
                let function = u - input + feedback * saturated;
                let slope = f32x4::splat(1.0) + feedback * a * derivative;
                u = clamp_nonlinear_state(u - function / slope);
            }
        }

        let first = process_section(
            &mut self.section[0],
            u,
            if frame.poles == 2 {
                coefficients.two_pole
            } else {
                coefficients.four_pole[0]
            },
        );
        if frame.poles == 2 {
            return first;
        }
        process_section(&mut self.section[1], first, coefficients.four_pole[1])
            * self_oscillation_output_makeup(amount)
    }

    fn output_affine_form(&self, poles: u8, coefficients: Coefficients) -> (f32x4, f32x4) {
        if poles == 2 {
            return section_affine(self.section[0], coefficients.two_pole);
        }

        let (first_a, first_b) = section_affine(self.section[0], coefficients.four_pole[0]);
        let (second_a, second_b) = section_affine(self.section[1], coefficients.four_pole[1]);
        (second_a * first_a, second_a * first_b + second_b)
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
        for state in &mut self.oversample_decimator_z {
            filtered = tpt_one_pole(filtered, state, g);
        }
        filtered
    }

    fn coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> Coefficients {
        if frame.static_cutoff {
            return self.static_coefficients(frame, sample_rate);
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
        let cutoff = (f32x4::splat(frame.cutoff_hz) * scale)
            .clamp(f32x4::splat(MIN_CUTOFF_HZ), f32x4::splat(max_cutoff));
        coefficients_from_cutoff(cutoff, max_cutoff, sample_rate)
    }

    fn static_coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> Coefficients {
        let pitch_cents = if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control)).to_array()[0]
                * self.self_osc_pitch_tuning_cents
        } else {
            0.0
        };
        let key = [sample_rate.to_bits(), pitch_cents.to_bits()];
        if self.static_coefficient_cache.key != key {
            let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
            let cutoff = (frame.cutoff_hz * crate::math::exp2(pitch_cents / 1200.0))
                .clamp(MIN_CUTOFF_HZ, max_cutoff);
            let g = crate::math::tan(core::f32::consts::PI * cutoff / sample_rate);
            self.static_coefficient_cache = StaticCoefficientCache {
                key,
                two_pole: section_coefficients_scalar(g, TWO_POLE_DAMPING),
                four_pole_first: section_coefficients_scalar(g, FOUR_POLE_FIRST_DAMPING),
                four_pole_second: section_coefficients_scalar(g, FOUR_POLE_SECOND_DAMPING),
            };
        }

        Coefficients {
            two_pole: SectionCoefficients::splat(self.static_coefficient_cache.two_pole),
            four_pole: [
                SectionCoefficients::splat(self.static_coefficient_cache.four_pole_first),
                SectionCoefficients::splat(self.static_coefficient_cache.four_pole_second),
            ],
        }
    }
}

impl FilterAlgorithm for CascadedTptSvf {
    fn reset(&mut self) {
        CascadedTptSvf::reset(self);
    }

    fn reset_lane(&mut self, lane: usize) {
        CascadedTptSvf::reset_lane(self, lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn clear_oversampling_state(&mut self) {
        CascadedTptSvf::clear_oversampling_state(self);
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        CascadedTptSvf::process(self, frame)
    }
}

fn section_coefficients(g: f32x4, damping: f32) -> SectionCoefficients {
    let a1 = f32x4::splat(1.0) / (f32x4::splat(1.0) + g * (g + f32x4::splat(damping)));
    let a2 = g * a1;
    SectionCoefficients { a1, a2, a3: g * a2 }
}

fn section_coefficients_scalar(g: f32, damping: f32) -> [f32; 3] {
    let a1 = 1.0 / (1.0 + g * (g + damping));
    let a2 = g * a1;
    [a1, a2, g * a2]
}

fn coefficients_from_cutoff(cutoff: f32x4, max_cutoff: f32, sample_rate: f32) -> Coefficients {
    let mut values = cutoff.to_array();
    for value in &mut values {
        *value = core::f32::consts::PI * value.clamp(MIN_CUTOFF_HZ, max_cutoff) / sample_rate;
    }
    let g = f32x4::new(values).tan();
    Coefficients {
        two_pole: section_coefficients(g, TWO_POLE_DAMPING),
        four_pole: [
            section_coefficients(g, FOUR_POLE_FIRST_DAMPING),
            section_coefficients(g, FOUR_POLE_SECOND_DAMPING),
        ],
    }
}

fn section_affine(state: SvfState, coefficients: SectionCoefficients) -> (f32x4, f32x4) {
    let a = coefficients.a3;
    let b = coefficients.a2 * state.integrator_1
        + (f32x4::splat(1.0) - coefficients.a3) * state.integrator_2;
    (a, b)
}

fn process_section(state: &mut SvfState, input: f32x4, coefficients: SectionCoefficients) -> f32x4 {
    let v3 = input - state.integrator_2;
    let v1 = coefficients.a1 * state.integrator_1 + coefficients.a2 * v3;
    let v2 = state.integrator_2 + coefficients.a2 * state.integrator_1 + coefficients.a3 * v3;
    state.integrator_1 = f32x4::splat(2.0) * v1 - state.integrator_1;
    state.integrator_2 = f32x4::splat(2.0) * v2 - state.integrator_2;
    v2
}

fn self_oscillation_amount(resonance_control: f32x4) -> f32x4 {
    ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
        / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
    .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
}

fn self_oscillation_feedback(linear: f32x4, amount: f32x4) -> f32x4 {
    let shape = smoothstep(amount);
    let transition = shape * shape;
    let target = f32x4::splat(FOUR_POLE_SELF_OSC_START_FEEDBACK)
        + shape * f32x4::splat(FOUR_POLE_SELF_OSC_MAX_FEEDBACK - FOUR_POLE_SELF_OSC_START_FEEDBACK);
    linear + (target - linear) * transition
}

fn self_oscillation_output_makeup(amount: f32x4) -> f32x4 {
    f32x4::splat(1.0) + smoothstep(amount) * f32x4::splat(SELF_OSC_OUTPUT_MAKEUP - 1.0)
}

fn smoothstep(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    value * value * (f32x4::splat(3.0) - f32x4::splat(2.0) * value)
}

/// Padé tanh and its normalized derivative for `tanh(drive*y) / drive`.
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

fn reset_vector_lane(value: &mut f32x4, lane: usize) {
    let mut values = value.to_array();
    values[lane] = 0.0;
    *value = f32x4::new(values);
}

fn tpt_one_pole(input: f32x4, state: &mut f32x4, g: f32x4) -> f32x4 {
    let v = (input - *state) * g;
    let output = v + *state;
    *state = output + v;
    output
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn butterworth_damping_constants_are_complementary() {
        assert!(
            (FOUR_POLE_FIRST_DAMPING * FOUR_POLE_SECOND_DAMPING - core::f32::consts::SQRT_2).abs()
                < 1.0e-6
        );
    }

    #[test]
    fn section_affine_form_matches_direct_processing() {
        let state = SvfState {
            integrator_1: f32x4::new([0.1, -0.2, 0.3, -0.4]),
            integrator_2: f32x4::new([-0.2, 0.3, -0.4, 0.5]),
        };
        let coefficients = section_coefficients(f32x4::splat(0.17), TWO_POLE_DAMPING);
        let input = f32x4::new([-0.7, -0.1, 0.2, 0.9]);
        let (a, b) = section_affine(state, coefficients);
        let mut direct_state = state;
        let direct = process_section(&mut direct_state, input, coefficients);
        for (affine, direct) in (a * input + b)
            .to_array()
            .into_iter()
            .zip(direct.to_array())
        {
            assert!((affine - direct).abs() < 1.0e-7);
        }
    }

    #[test]
    fn nonlinear_solver_uses_exactly_two_newton_steps() {
        assert_eq!(NONLINEAR_NEWTON_STEPS, 2);
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
    fn cascaded_tpt_svf_is_available_with_butterworth_response_and_slopes() {
        assert!(FilterType::CascadedTptSvf.is_implemented());
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for (poles, expected) in [(2, 11.5..=12.6), (4, 22.5..=25.0)] {
                let lower = sine_gain(
                    FilterType::CascadedTptSvf,
                    sample_rate,
                    CUTOFF_HZ * 4.0,
                    CUTOFF_HZ,
                    0.0,
                    poles,
                    FilterOversampling::Off,
                    1.0e-4,
                );
                let upper = sine_gain(
                    FilterType::CascadedTptSvf,
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

            let low = sine_gain(
                FilterType::CascadedTptSvf,
                sample_rate,
                CUTOFF_HZ * 0.5,
                CUTOFF_HZ,
                0.0,
                4,
                FilterOversampling::Off,
                1.0e-4,
            );
            let cutoff = sine_gain(
                FilterType::CascadedTptSvf,
                sample_rate,
                CUTOFF_HZ,
                CUTOFF_HZ,
                0.0,
                4,
                FilterOversampling::Off,
                1.0e-4,
            );
            let high = sine_gain(
                FilterType::CascadedTptSvf,
                sample_rate,
                CUTOFF_HZ * 2.0,
                CUTOFF_HZ,
                0.0,
                4,
                FilterOversampling::Off,
                1.0e-4,
            );
            let two_pole_cutoff = sine_gain(
                FilterType::CascadedTptSvf,
                sample_rate,
                CUTOFF_HZ,
                CUTOFF_HZ,
                0.0,
                2,
                FilterOversampling::Off,
                1.0e-4,
            );
            assert!((0.98..1.02).contains(&low), "sr={sample_rate} low={low}");
            assert!(
                (0.69..0.72).contains(&cutoff),
                "sr={sample_rate} cutoff={cutoff}"
            );
            assert!(
                (0.69..0.72).contains(&two_pole_cutoff),
                "sr={sample_rate} two_pole_cutoff={two_pole_cutoff}"
            );
            assert!(
                (0.058..0.066).contains(&high),
                "sr={sample_rate} high={high}"
            );
        }
    }

    #[test]
    fn cascaded_tpt_svf_self_oscillation_is_tuned_and_harmonically_bounded() {
        for cutoff in [110.0, 220.0, 440.0, 880.0, 1760.0] {
            let baseline = tail(
                FilterType::DistributedNewtonTpt,
                cutoff,
                1.0,
                FilterOversampling::Off,
                true,
            );
            let candidate = tail(
                FilterType::CascadedTptSvf,
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
    fn cascaded_tpt_svf_resonance_onset_and_global_oversampling_are_smooth() {
        let gains = [0.70, 0.71, 0.72, 0.74, 0.75, 0.76, 0.80].map(|resonance| {
            sine_gain(
                FilterType::CascadedTptSvf,
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
            (0.90, 0.24..0.36),
            (0.95, 0.38..0.47),
            (1.00, 0.44..0.52),
        ] {
            let samples = tail(
                FilterType::CascadedTptSvf,
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
            let samples = tail(FilterType::CascadedTptSvf, CUTOFF_HZ, 1.0, mode, false);
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
    fn cascaded_tpt_svf_two_pole_decays_and_control_grid_stays_finite() {
        let mut two_pole = filter(
            FilterType::CascadedTptSvf,
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
                            FilterType::CascadedTptSvf,
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
    fn cascaded_tpt_svf_reset_key_tracking_and_lanes_are_independent() {
        let make = || {
            filter(
                FilterType::CascadedTptSvf,
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
            assert!(
                (mixed_output[0] - low_output[0]).abs() < 1.0e-6,
                "frame={frame} mixed={} low={} delta={}",
                mixed_output[0],
                low_output[0],
                mixed_output[0] - low_output[0]
            );
            assert!(
                (mixed_output[1] - high_output[1]).abs() < 1.0e-6,
                "frame={frame} mixed={} high={} delta={}",
                mixed_output[1],
                high_output[1],
                mixed_output[1] - high_output[1]
            );
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
        let reset = process(
            &mut mixed,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        );
        let fresh_output = process(
            &mut fresh,
            f32x4::splat(0.1),
            f32x4::splat(69.0),
            SAMPLE_RATE,
        );
        for (reset, fresh) in reset.to_array().into_iter().zip(fresh_output.to_array()) {
            assert!((reset - fresh).abs() < 1.0e-12);
        }

        for note in [36.0, 48.0, 60.0, 72.0, 84.0] {
            let mut tracked = filter(
                FilterType::CascadedTptSvf,
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
    fn cascaded_tpt_svf_long_running_self_oscillation_stays_bounded() {
        let mut filter = filter(
            FilterType::CascadedTptSvf,
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
    fn cascaded_tpt_svf_high_cutoff_feedback_stays_bounded() {
        for cutoff in [6_600.0, 12_000.0, 18_000.0, 20_000.0] {
            let samples = tail(
                FilterType::CascadedTptSvf,
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
