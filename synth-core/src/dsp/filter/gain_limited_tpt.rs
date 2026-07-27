//! Four-stage TPT cascade with an analytic, gain-limited feedback loop.

use crate::math::{F32, WideF32};

use crate::dsp::filter::{
    FilterAlgorithm, FilterFrame, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ, SELF_OSC_RESONANCE_START,
};

const TWO_POLE_MAX_RESONANCE: f32 = 1.9;
const FOUR_POLE_MAX_LINEAR_RESONANCE: f32 = 3.75;
const FOUR_POLE_SELF_OSC_START_RESONANCE: f32 = 4.05;
const FOUR_POLE_SELF_OSC_MAX_RESONANCE: f32 = 5.25;
const RESONANCE_BASS_COMP: f32 = 0.80;
const LIMITER_DRIVE: f32 = 1.0;
const LIMITER_SMOOTHING_SCALE: f32 = 0.73;
const SELF_OSC_OUTPUT_MAKEUP: f32 = 0.84;
// Fixed four-pole input headroom; never varies with resonance or voice count.
const AUDIO_INPUT_GAIN: f32 = 0.40;
const SELF_OSC_PITCH_TUNING_CENTS: f32 = -18.0;
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
const SELF_OSC_COLOR_REFERENCE_LEVEL: f32 = 0.69;
const SELF_OSC_COLOR_REFERENCE_LEVEL_INV: f32 = 1.0 / SELF_OSC_COLOR_REFERENCE_LEVEL;
const SELF_OSC_COLOR_B0: f32 = -0.007564;
const SELF_OSC_COLOR_B1: f32 = 1.003405;
const SELF_OSC_COLOR_B2: f32 = 0.014912;
const SELF_OSC_COLOR_B3: f32 = -0.00434;
const SELF_OSC_COLOR_B4: f32 = 0.000288;
const SELF_OSC_COLOR_B5: f32 = -0.00024;

#[inline(always)]
fn uniform_lane_value(value: WideF32) -> Option<f32> {
    let lanes = value.to_array();
    lanes[1..]
        .iter()
        .all(|lane| lane.to_bits() == lanes[0].to_bits())
        .then_some(lanes[0])
}

#[derive(Clone, Copy, Debug, Default)]
struct StaticCoefficientCache {
    key: [u32; 2],
    value: f32,
}

/// CPU-floor candidate using the previous output to limit loop gain.
pub(super) struct GainLimitedTpt {
    self_osc_pitch_tuning_cents: f32,
    static_coefficient_cache: StaticCoefficientCache,
    z: [WideF32; 4],
    oversample_decimator_z: WideF32,
    previous_output: WideF32,
    smoothed_feedback: WideF32,
}

impl Default for GainLimitedTpt {
    fn default() -> Self {
        Self {
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            static_coefficient_cache: StaticCoefficientCache::default(),
            z: [WideF32::ZERO; 4],
            oversample_decimator_z: WideF32::ZERO,
            previous_output: WideF32::ZERO,
            smoothed_feedback: WideF32::ZERO,
        }
    }
}

impl GainLimitedTpt {
    fn reset(&mut self) {
        self.z = [WideF32::ZERO; 4];
        self.previous_output = WideF32::ZERO;
        self.smoothed_feedback = WideF32::ZERO;
        self.clear_oversampling_state();
    }

    fn reset_lane(&mut self, lane: usize) {
        for state in self.z.iter_mut().chain(
            [
                &mut self.oversample_decimator_z,
                &mut self.previous_output,
                &mut self.smoothed_feedback,
            ]
            .into_iter(),
        ) {
            let mut values = state.to_array();
            values[lane] = 0.0;
            *state = WideF32::new(values);
        }
    }

    fn clear_oversampling_state(&mut self) {
        self.oversample_decimator_z = WideF32::ZERO;
    }

    fn process(&mut self, frame: FilterFrame) -> WideF32 {
        // Oversampling is a global run setting, independent of resonance.
        let factor = frame.oversampling.factor(frame.sample_rate);
        if factor == 1 {
            let g = self.coefficients(frame, frame.sample_rate);
            return self.process_subsample(frame, g);
        }

        let oversampled_rate = frame.sample_rate * factor as f32;
        let g = self.coefficients(frame, oversampled_rate);
        let mut output = WideF32::ZERO;
        for _ in 0..factor {
            output = self.process_subsample(frame, g);
            output = self.decimate(output, factor);
        }
        output
    }

    fn process_subsample(&mut self, frame: FilterFrame, g: WideF32) -> WideF32 {
        let amount = if frame.poles == 4 {
            self_oscillation_amount(frame.resonance_control)
        } else {
            WideF32::ZERO
        };
        let transition = smoothstep(amount);
        let linear_feedback = if frame.poles == 2 {
            frame.shaped_resonance * WideF32::splat(TWO_POLE_MAX_RESONANCE)
        } else {
            frame.shaped_resonance * WideF32::splat(FOUR_POLE_MAX_LINEAR_RESONANCE)
        };
        let requested_feedback = if frame.poles == 4 {
            self_oscillation_feedback(linear_feedback, transition)
        } else {
            linear_feedback
        };
        let nonlinear = amount.simd_gt(WideF32::ZERO);
        let effective_feedback = if nonlinear.any() {
            let limiter_drive = transition * WideF32::splat(LIMITER_DRIVE);
            let output_power = self.previous_output * self.previous_output;
            let limited_feedback =
                requested_feedback / (WideF32::splat(1.0) + limiter_drive * output_power);
            // This keeps the smoothing time at a stable fraction of the cutoff
            // period across pitch, sample rate, and global oversampling factors.
            let smoothing = (g * WideF32::splat(LIMITER_SMOOTHING_SCALE))
                .clamp(WideF32::ZERO, WideF32::splat(1.0));
            self.smoothed_feedback += (limited_feedback - self.smoothed_feedback) * smoothing;
            self.smoothed_feedback = nonlinear.blend(self.smoothed_feedback, linear_feedback);
            self.smoothed_feedback
        } else {
            self.smoothed_feedback = linear_feedback;
            linear_feedback
        };

        let audio_input = if frame.poles == 4 {
            frame.input * WideF32::splat(AUDIO_INPUT_GAIN)
        } else {
            frame.input
        };
        let compensated_input = if frame.poles == 4 {
            audio_input
                * (WideF32::splat(1.0)
                    + frame.shaped_resonance
                        * WideF32::splat(FOUR_POLE_MAX_LINEAR_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            audio_input
        };
        let input = compensated_input + self_oscillation_excitation(amount);
        let (a, b) = self.output_affine_form(g, frame.poles);
        let u = (input - effective_feedback * b) / (WideF32::splat(1.0) + effective_feedback * a);

        let y0 = g * u + stage_offset(self.z[0], g);
        let y1 = g * y0 + stage_offset(self.z[1], g);
        let y2 = g * y1 + stage_offset(self.z[2], g);
        let y3 = g * y2 + stage_offset(self.z[3], g);
        commit_tpt_output(&mut self.z[0], y0);
        commit_tpt_output(&mut self.z[1], y1);
        commit_tpt_output(&mut self.z[2], y2);
        commit_tpt_output(&mut self.z[3], y3);
        self.previous_output = y3;

        if frame.poles == 2 {
            y1
        } else {
            let makeup = self_oscillation_output_makeup(transition);
            let output = y3 * makeup;
            if frame.self_oscillation_color_enabled && nonlinear.any() {
                // Adjacent low-pass stages cancel away from cutoff, providing a
                // state-free band-pass proxy for the resonant component. Color
                // only this band so oscillator content does not turn the whole
                // voice into a distortion stage.
                let resonance_band = (y2 - y3) * makeup;
                let colored =
                    output + self_oscillation_color(resonance_band, amount) - resonance_band;
                nonlinear.blend(colored, output)
            } else {
                output
            }
        }
    }

    fn output_affine_form(&self, g: WideF32, poles: u8) -> (WideF32, WideF32) {
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

    fn decimate(&mut self, output: WideF32, factor: usize) -> WideF32 {
        let mut state = self.oversample_decimator_z.to_array();
        let output_values = output.to_array();
        for lane in 0..state.len() {
            if state[lane] == 0.0 {
                state[lane] = output_values[lane];
            }
        }
        self.oversample_decimator_z = WideF32::new(state);
        let g = WideF32::splat(decimator_coefficient(factor));
        tpt_one_pole(output, &mut self.oversample_decimator_z, g)
    }

    fn coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> WideF32 {
        if frame.static_cutoff {
            return WideF32::splat(self.static_coefficient(frame, sample_rate));
        }

        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let uniform_pitch_semitones = frame.cutoff_mod_uniform_semitones.and_then(|_| {
            if frame.poles == 4 {
                uniform_lane_value(frame.resonance_control).map(|resonance| {
                    smoothstep_scalar(self_oscillation_amount_scalar(resonance))
                        * (self.self_osc_pitch_tuning_cents / 100.0)
                })
            } else {
                Some(0.0)
            }
        });
        let pitch_semitones = if let Some(pitch) = uniform_pitch_semitones {
            WideF32::splat(pitch)
        } else if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control))
                * WideF32::splat(self.self_osc_pitch_tuning_cents / 100.0)
        } else {
            WideF32::ZERO
        };
        let cutoff_modulation = super::coefficient_math::PreparedCutoffModulation::new(
            frame.cutoff_mod_semitones + pitch_semitones,
            frame
                .cutoff_mod_uniform_semitones
                .zip(uniform_pitch_semitones)
                .map(|(cutoff, pitch)| cutoff + pitch),
        );
        super::coefficient_math::modulated_tpt_coefficient(
            frame.cutoff_hz,
            cutoff_modulation,
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
        let cutoff = (frame.cutoff_hz * F32(pitch_cents / 1200.0).exp2().as_f32())
            .clamp(MIN_CUTOFF_HZ, max_cutoff);
        let raw = F32(core::f32::consts::PI * cutoff / sample_rate)
            .tan()
            .as_f32();
        let value = raw / (1.0 + raw);
        self.static_coefficient_cache = StaticCoefficientCache { key, value };
        value
    }
}

#[inline(always)]
fn decimator_coefficient(factor: usize) -> f32 {
    decimator_math::coefficient(factor)
}

mod decimator_math {
    use crate::math::F32;
    #[cfg(feature = "fast-math")]
    #[inline(always)]
    pub(super) fn coefficient(factor: usize) -> f32 {
        match factor {
            2 => 0.460_649_16,
            4 => 0.269_496_83,
            _ => 1.0,
        }
    }

    #[cfg(not(feature = "fast-math"))]
    #[inline(always)]
    pub(super) fn coefficient(factor: usize) -> f32 {
        let angle = core::f32::consts::PI * 0.45 / factor as f32;
        let raw = F32(angle).tan().as_f32();
        raw / (1.0 + raw)
    }
}

impl FilterAlgorithm for GainLimitedTpt {
    fn reset(&mut self) {
        GainLimitedTpt::reset(self);
    }

    fn reset_lane(&mut self, lane: usize) {
        GainLimitedTpt::reset_lane(self, lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn clear_oversampling_state(&mut self) {
        GainLimitedTpt::clear_oversampling_state(self);
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    fn process(&mut self, frame: FilterFrame) -> WideF32 {
        GainLimitedTpt::process(self, frame)
    }
}

fn self_oscillation_amount(resonance_control: WideF32) -> WideF32 {
    ((resonance_control - WideF32::splat(SELF_OSC_RESONANCE_START))
        / WideF32::splat(1.0 - SELF_OSC_RESONANCE_START))
    .clamp(WideF32::ZERO, WideF32::splat(1.0))
}

#[inline(always)]
fn self_oscillation_amount_scalar(resonance_control: f32) -> f32 {
    let amount = (resonance_control - SELF_OSC_RESONANCE_START) / (1.0 - SELF_OSC_RESONANCE_START);
    if amount <= 0.0 {
        0.0
    } else if amount >= 1.0 {
        1.0
    } else {
        amount
    }
}

fn self_oscillation_feedback(linear: WideF32, transition: WideF32) -> WideF32 {
    let target = WideF32::splat(FOUR_POLE_SELF_OSC_START_RESONANCE)
        + transition
            * WideF32::splat(FOUR_POLE_SELF_OSC_MAX_RESONANCE - FOUR_POLE_SELF_OSC_START_RESONANCE);
    linear + (target - linear) * transition
}

fn self_oscillation_output_makeup(transition: WideF32) -> WideF32 {
    WideF32::splat(1.0) + transition * WideF32::splat(SELF_OSC_OUTPUT_MAKEUP - 1.0)
}

/// Shapes the autonomous self-oscillation with a finite Chebyshev polynomial.
/// The second and third partials use relative coefficients 0.0076 and -0.00116;
/// tiny fourth/fifth-partial terms (0.000036/-0.000015) suppress residual upper
/// color without adding another polynomial order or an unlimited harmonic tail.
fn self_oscillation_color(output: WideF32, amount: WideF32) -> WideF32 {
    let ramp = ((amount - WideF32::splat(0.8)) * WideF32::splat(5.0))
        .clamp(WideF32::ZERO, WideF32::splat(1.0));
    let normalized = (output * WideF32::splat(SELF_OSC_COLOR_REFERENCE_LEVEL_INV))
        .clamp(WideF32::splat(-1.0), WideF32::splat(1.0));
    let polynomial =
        WideF32::splat(SELF_OSC_COLOR_B4) + normalized * WideF32::splat(SELF_OSC_COLOR_B5);
    let polynomial = WideF32::splat(SELF_OSC_COLOR_B3) + normalized * polynomial;
    let polynomial = WideF32::splat(SELF_OSC_COLOR_B2) + normalized * polynomial;
    let polynomial = WideF32::splat(SELF_OSC_COLOR_B1) + normalized * polynomial;
    let polynomial = WideF32::splat(SELF_OSC_COLOR_B0) + normalized * polynomial;
    let colored = polynomial * WideF32::splat(SELF_OSC_COLOR_REFERENCE_LEVEL);
    output + (colored - output) * ramp
}

fn self_oscillation_excitation(amount: WideF32) -> WideF32 {
    let gain = amount * amount * WideF32::splat(SELF_OSC_EXCITATION);
    gain * WideF32::new(core::array::from_fn(|i| [1.0, -0.75, 0.5, -0.25][i % 4]))
}

fn smoothstep(value: WideF32) -> WideF32 {
    let value = value.clamp(WideF32::ZERO, WideF32::splat(1.0));
    value * value * (WideF32::splat(3.0) - WideF32::splat(2.0) * value)
}

#[inline(always)]
fn smoothstep_scalar(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn stage_offset(z: WideF32, g: WideF32) -> WideF32 {
    z * (WideF32::splat(1.0) - g)
}

fn commit_tpt_output(z: &mut WideF32, y: WideF32) {
    *z = y + (y - *z);
}

fn tpt_one_pole(input: WideF32, z: &mut WideF32, g: WideF32) -> WideF32 {
    let v = (input - *z) * g;
    let output = v + *z;
    *z = output + v;
    output
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    #[cfg(feature = "wide-4")]
    fn state_stays_within_runtime_size_budget() {
        assert!(core::mem::size_of::<GainLimitedTpt>() <= 128);
    }

    #[test]
    fn embedded_uniform_coefficients_keep_filter_output_within_error_bounds() {
        for resonance in [0.0f32, 0.65, 0.9] {
            let shaped_resonance = F32(resonance).powf(F32(1.75)).as_f32();
            let resonance_control = WideF32::splat(resonance);
            let amount = self_oscillation_amount(resonance_control);
            let pitch_semitones =
                smoothstep(amount) * WideF32::splat(SELF_OSC_PITCH_TUNING_CENTS / 100.0);
            let mut vector_filter = GainLimitedTpt::default();
            let mut embedded_filter = GainLimitedTpt::default();
            let mut squared_error = 0.0f32;
            let mut maximum_error = 0.0f32;

            for sample in 0..4096 {
                let phase = (sample % 192) as f32 / 192.0;
                let triangle = if phase < 0.25 {
                    phase * 4.0
                } else if phase < 0.75 {
                    2.0 - phase * 4.0
                } else {
                    phase * 4.0 - 4.0
                };
                let cutoff_mod = WideF32::splat(triangle * 36.0);
                let input_phase = (sample % 97) as f32 / 97.0;
                let input = WideF32::new(core::array::from_fn(|i| {
                    [
                        input_phase * 2.0 - 1.0,
                        0.75 - input_phase,
                        input_phase - 0.25,
                        0.5 - input_phase * 0.5,
                    ][i % 4]
                }));
                let frame = FilterFrame {
                    input,
                    cutoff_hz: 1_200.0,
                    cutoff_mod_semitones: cutoff_mod,
                    cutoff_mod_uniform_semitones: Some(triangle * 36.0),
                    resonance_control,
                    shaped_resonance: WideF32::splat(shaped_resonance),
                    poles: 4,
                    oversampling: crate::dsp::FilterOversampling::Off,
                    sample_rate: 48_000.0,
                    static_cutoff: false,
                    self_oscillation_color_enabled: true,
                };
                let total_modulation = cutoff_mod + pitch_semitones;
                let vector_g = super::super::coefficient_math::vector_coefficient(
                    frame.cutoff_hz,
                    total_modulation,
                    18_000.0,
                    frame.sample_rate,
                );
                let embedded_g = super::super::coefficient_math::embedded_coefficient(
                    frame.cutoff_hz,
                    super::super::coefficient_math::PreparedCutoffModulation::new(
                        total_modulation,
                        uniform_lane_value(total_modulation),
                    ),
                    18_000.0,
                    frame.sample_rate,
                );
                let vector = vector_filter.process_subsample(frame, vector_g);
                let embedded = embedded_filter.process_subsample(frame, embedded_g);
                for (actual, expected) in embedded.to_array().into_iter().zip(vector.to_array()) {
                    let error = (actual - expected).abs();
                    maximum_error = maximum_error.max(error);
                    squared_error += error * error;
                }
            }

            let rms_error = (squared_error / (4096 * 4) as f32).sqrt();
            assert!(
                rms_error <= 5.0e-5,
                "resonance {resonance}: RMS error {rms_error}"
            );
            assert!(
                maximum_error <= 2.0e-4,
                "resonance {resonance}: maximum error {maximum_error}"
            );
        }
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn limiter_reduces_feedback_as_output_grows() {
        let requested = WideF32::splat(5.25);
        let drive = WideF32::splat(LIMITER_DRIVE);
        let previous = WideF32::new(core::array::from_fn(|i| [0.0, 0.25, 0.5, 1.0][i % 4]));
        let limited = requested / (WideF32::splat(1.0) + drive * previous * previous);
        assert!(limited.to_array().windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn harmonic_color_does_not_inflate_the_self_oscillation_crest() {
        let mut filter = GainLimitedTpt::default();
        let frame = FilterFrame {
            input: WideF32::ZERO,
            cutoff_hz: 739.99,
            cutoff_mod_semitones: WideF32::ZERO,
            cutoff_mod_uniform_semitones: Some(0.0),
            resonance_control: WideF32::splat(1.0),
            shaped_resonance: WideF32::splat(1.0),
            poles: 4,
            oversampling: crate::dsp::FilterOversampling::Off,
            sample_rate: 48_000.0,
            static_cutoff: true,
            self_oscillation_color_enabled: true,
        };
        let mut peak = 0.0f32;
        for index in 0..100_096 {
            let output = filter.process(frame).to_array()[0];
            if index >= 96_000 {
                peak = peak.max(output.abs());
            }
        }
        assert!(
            peak <= 0.72,
            "harmonics inflated the autonomous crest: {peak}"
        );
        assert!(
            peak >= 0.63,
            "harmonics reduced the autonomous crest: {peak}"
        );
    }

    use crate::dsp::filter::{Filter, FilterOversampling, FilterType};
    use crate::math::WideF32;
    use crate::{ParamId, SynthEngine};

    extern crate std;
    use std::vec::Vec;

    const SAMPLE_RATE: f32 = 48_000.0;
    const CUTOFF_HZ: f32 = 440.0;

    fn configured_filter(
        filter_type: FilterType,
        cutoff_hz: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
    ) -> Filter {
        let mut filter = Filter::new(filter_type);
        filter.set_cutoff(cutoff_hz);
        filter.set_resonance(resonance);
        filter.set_poles(poles);
        filter.set_oversampling(oversampling);
        filter
    }

    fn process(filter: &mut Filter, input: WideF32, note: WideF32, sample_rate: f32) -> WideF32 {
        filter.process(
            input,
            note,
            WideF32::ZERO,
            WideF32::splat(1.0),
            WideF32::ZERO,
            WideF32::ZERO,
            WideF32::ZERO,
            WideF32::ZERO,
            sample_rate,
        )
    }

    fn sine_gain(
        filter_type: FilterType,
        sample_rate: f32,
        frequency: f32,
        cutoff_hz: f32,
        resonance: f32,
        poles: u8,
        oversampling: FilterOversampling,
        amplitude: f32,
    ) -> f32 {
        let mut filter = configured_filter(filter_type, cutoff_hz, resonance, poles, oversampling);
        let phase_step = core::f32::consts::TAU * frequency / sample_rate;
        let frames = (sample_rate * 0.1) as usize;
        let mut phase = 0.0f32;
        for _ in 0..frames {
            let _ = process(
                &mut filter,
                WideF32::splat(phase.sin() * amplitude),
                WideF32::splat(69.0),
                sample_rate,
            );
            phase += phase_step;
        }

        let mut sin_sum = 0.0;
        let mut cos_sum = 0.0;
        for _ in 0..frames {
            let sine = phase.sin();
            let output = process(
                &mut filter,
                WideF32::splat(sine * amplitude),
                WideF32::splat(69.0),
                sample_rate,
            )
            .to_array()[0];
            sin_sum += output * sine;
            cos_sum += output * phase.cos();
            phase += phase_step;
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / frames as f32 / amplitude
    }

    fn self_oscillation_tail(
        filter_type: FilterType,
        cutoff_hz: f32,
        resonance: f32,
        oversampling: FilterOversampling,
    ) -> Vec<f32> {
        let mut filter = configured_filter(filter_type, cutoff_hz, resonance, 4, oversampling);
        for _ in 0..128 {
            let _ = process(
                &mut filter,
                WideF32::splat(0.1),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            );
        }
        let mut samples = Vec::with_capacity(48_000);
        for _ in 0..48_000 {
            samples.push(
                process(
                    &mut filter,
                    WideF32::ZERO,
                    WideF32::splat(69.0),
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

    fn positive_crossing_pitch(samples: &[f32]) -> f32 {
        let mut crossings = 0usize;
        let mut first = None;
        let mut last = None;
        for (index, pair) in samples.windows(2).enumerate() {
            if pair[0] <= 0.0 && pair[1] > 0.0 {
                let crossing = index as f32 + (-pair[0] / (pair[1] - pair[0])).clamp(0.0, 1.0);
                crossings += 1;
                first.get_or_insert(crossing);
                last = Some(crossing);
            }
        }
        match (first, last) {
            (Some(first), Some(last)) if crossings > 1 && last > first => {
                (crossings - 1) as f32 * SAMPLE_RATE / (last - first) as f32
            }
            _ => 0.0,
        }
    }

    fn analyzer_peak_near(samples: &[f32], center_bin: usize, radius: usize) -> (usize, f32) {
        let fft_size = samples.len();
        let mut peak_bin = center_bin;
        let mut peak = 0.0f32;
        for bin in
            center_bin.saturating_sub(radius).max(1)..=(center_bin + radius).min(fft_size / 2 - 1)
        {
            let step = core::f32::consts::TAU * bin as f32 / fft_size as f32;
            let mut sin_sum = 0.0;
            let mut cos_sum = 0.0;
            for (index, sample) in samples.iter().copied().enumerate() {
                let window = 0.5
                    * (1.0 - (core::f32::consts::TAU * index as f32 / (fft_size - 1) as f32).cos());
                let phase = step * index as f32;
                sin_sum += sample * window * phase.sin();
                cos_sum += sample * window * phase.cos();
            }
            let magnitude = (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / fft_size as f32;
            if magnitude > peak {
                peak = magnitude;
                peak_bin = bin;
            }
        }
        (peak_bin, 20.0 * peak.max(1.0e-10).log10())
    }

    fn live_self_oscillation_analyzer_harmonics_db(
        cutoff_hz: f32,
        resonance: f32,
        velocity: f32,
        amp_velocity: f32,
        oscillator_active: bool,
        note: u8,
    ) -> [f32; 15] {
        const FFT_SIZE: usize = 4096;
        let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
        engine.set_filter_type(FilterType::GainLimitedTpt);
        engine.set_filter_oversampling(FilterOversampling::Off);
        // Mirror the UI's self-oscillation setup. For the silent case oscillator 1
        // stays enabled but the mixer points at disabled oscillator 2.
        engine.set_param(ParamId::Osc1Enabled, 1.0);
        engine.set_param(ParamId::Osc2Enabled, 0.0);
        engine.set_param(ParamId::OscMix, if oscillator_active { 0.0 } else { 1.0 });
        engine.set_param(ParamId::Osc1Waveform, 2.0);
        engine.set_param(ParamId::NoiseLevel, 0.0);
        engine.set_param(ParamId::SubOscLevel, 0.0);
        engine.set_param(ParamId::FilterCutoff, cutoff_hz);
        engine.set_param(ParamId::FilterResonance, resonance);
        engine.set_param(ParamId::FilterPoles, 1.0);
        engine.set_param(ParamId::AmpEgAttack, 0.0005);
        engine.set_param(ParamId::AmpEgDecay, 0.0005);
        engine.set_param(ParamId::AmpEgSustain, 1.0);
        engine.set_param(ParamId::AmpVelocity, amp_velocity);
        engine.set_param(ParamId::MasterVolume, 1.0);
        engine.note_on(note, velocity);

        let mut settle = std::vec![0.0; 48_000 * 2];
        engine.process_interleaved(&mut settle, 2);
        let mut interleaved = std::vec![0.0; FFT_SIZE * 2];
        engine.process_interleaved(&mut interleaved, 2);
        let samples: Vec<f32> = interleaved.chunks_exact(2).map(|frame| frame[0]).collect();

        let expected_bin = (cutoff_hz * FFT_SIZE as f32 / SAMPLE_RATE).round() as usize;
        let (fundamental_bin, fundamental_db) = analyzer_peak_near(&samples, expected_bin, 4);
        core::array::from_fn(|index| {
            let harmonic = index + 1;
            if harmonic == 1 {
                fundamental_db
            } else {
                analyzer_peak_near(&samples, fundamental_bin * harmonic, 2).1
            }
        })
    }

    fn live_key_tracked_fundamental_db(notes: &[u8], measured_hz: f32) -> f32 {
        const FFT_SIZE: usize = 4096;
        let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
        engine.set_filter_type(FilterType::GainLimitedTpt);
        engine.set_filter_oversampling(FilterOversampling::Off);
        engine.set_param(ParamId::Osc1Enabled, 0.0);
        engine.set_param(ParamId::Osc2Enabled, 0.0);
        engine.set_param(ParamId::NoiseLevel, 0.0);
        engine.set_param(ParamId::SubOscLevel, 0.0);
        engine.set_param(ParamId::FilterCutoff, measured_hz);
        engine.set_param(ParamId::FilterResonance, 1.0);
        engine.set_param(ParamId::FilterPoles, 1.0);
        engine.set_param(ParamId::FilterKeyTrack, 1.0);
        engine.set_param(ParamId::AmpEgAttack, 0.0005);
        engine.set_param(ParamId::AmpEgDecay, 0.0005);
        engine.set_param(ParamId::AmpEgSustain, 1.0);
        engine.set_param(ParamId::AmpVelocity, 0.0);
        engine.set_param(ParamId::MasterVolume, 1.0);
        for &note in notes {
            engine.note_on(note, 1.0);
        }

        let mut settle = std::vec![0.0; 48_000 * 2];
        engine.process_interleaved(&mut settle, 2);
        let mut interleaved = std::vec![0.0; FFT_SIZE * 2];
        engine.process_interleaved(&mut interleaved, 2);
        let samples: Vec<f32> = interleaved.chunks_exact(2).map(|frame| frame[0]).collect();
        let expected_bin = (measured_hz * FFT_SIZE as f32 / SAMPLE_RATE).round() as usize;
        analyzer_peak_near(&samples, expected_bin, 4).1
    }

    fn live_driven_chord_stats(notes: &[u8], resonance: f32) -> (f32, f32, usize) {
        let mut engine = SynthEngine::<1, 48_000>::new(SAMPLE_RATE);
        engine.set_filter_type(FilterType::GainLimitedTpt);
        engine.set_filter_oversampling(FilterOversampling::Off);
        engine.set_param(ParamId::Osc1Enabled, 1.0);
        engine.set_param(ParamId::Osc2Enabled, 0.0);
        engine.set_param(ParamId::NoiseLevel, 0.0);
        engine.set_param(ParamId::SubOscLevel, 0.0);
        engine.set_param(ParamId::FilterCutoff, 440.0);
        engine.set_param(ParamId::FilterResonance, resonance);
        engine.set_param(ParamId::AmpVelocity, 0.0);
        engine.set_param(ParamId::AmpEgAttack, 0.0005);
        engine.set_param(ParamId::AmpEgDecay, 0.0005);
        engine.set_param(ParamId::AmpEgSustain, 1.0);
        engine.set_param(ParamId::MasterVolume, 1.0);
        for &note in notes {
            engine.note_on(note, 1.0);
        }
        let mut settle = std::vec![0.0; 48_000 * 2];
        engine.process_interleaved(&mut settle, 2);
        let mut output = std::vec![0.0; 4096 * 2];
        engine.process_interleaved(&mut output, 2);
        let left = output.chunks_exact(2).map(|frame| frame[0]);
        let mut peak = 0.0f32;
        let mut energy = 0.0f32;
        let mut clipped = 0usize;
        for sample in left {
            peak = peak.max(sample.abs());
            energy += sample * sample;
            clipped += usize::from(sample.abs() >= 0.999_999);
        }
        (peak, (energy / 4096.0).sqrt(), clipped)
    }

    #[test]
    #[cfg(not(feature = "wide-1"))]
    fn gain_limited_driven_chords_retain_output_headroom() {
        let mut previous_peak = 0.0;
        let mut previous_rms = 0.0;
        for notes in [&[60][..], &[60, 64][..], &[60, 64, 67, 72][..]] {
            let (peak, rms, clipped) = live_driven_chord_stats(notes, 1.0);
            assert_eq!(
                clipped,
                0,
                "{}-note chord reached the final clamp",
                notes.len()
            );
            assert!(
                peak > previous_peak,
                "chord peak should grow with voice count"
            );
            assert!(
                rms > previous_rms,
                "chord intensity should grow with voice count"
            );
            assert!(
                peak < 0.98,
                "{}-note peak left too little headroom: {peak}",
                notes.len()
            );
            assert!(
                rms < 0.45,
                "{}-note RMS is unexpectedly high: {rms}",
                notes.len()
            );
            previous_peak = peak;
            previous_rms = rms;
        }
    }

    #[test]
    fn gain_limited_driven_level_rises_through_max_resonance() {
        let mut previous_rms = 0.0;
        for resonance in [0.90, 0.92, 0.94, 0.95, 0.96, 0.97, 0.98, 0.99, 1.0] {
            let (_, rms, clipped) = live_driven_chord_stats(&[60], resonance);
            assert_eq!(clipped, 0);
            assert!(
                rms >= previous_rms * 0.98,
                "driven level fell at resonance {resonance:.2}: {previous_rms} -> {rms}",
            );
            previous_rms = rms;
        }
    }

    #[test]
    #[cfg(not(feature = "wide-1"))]
    fn adding_a_second_key_does_not_raise_the_first_fundamental() {
        let one_key = live_key_tracked_fundamental_db(&[36], CUTOFF_HZ);
        let two_keys = live_key_tracked_fundamental_db(&[36, 43], CUTOFF_HZ);
        assert!(
            (one_key - two_keys).abs() <= 0.2,
            "first fundamental changed with voice count: one={one_key:.3}dB two={two_keys:.3}dB"
        );
    }

    #[test]
    #[cfg(not(feature = "downsampling"))]
    fn gain_limited_live_analyzer_level_and_harmonics_match_target() {
        for cutoff in [410.0, 110.0, 220.0, 440.0, 739.99, 880.0, 1760.0] {
            let harmonics =
                live_self_oscillation_analyzer_harmonics_db(cutoff, 1.0, 1.0, 1.0, false, 36);
            assert!(
                (-26.0..=-23.0).contains(&harmonics[0]),
                "cutoff={cutoff} harmonics={harmonics:?}"
            );
            for (index, expected) in [
                (1, -69.0..=-63.0),
                (2, -86.0..=-78.0),
                (3, -130.0..=-115.0),
                (4, -141.0..=-123.0),
                (5, -155.0..=-129.0),
            ] {
                assert!(
                    expected.contains(&harmonics[index]),
                    "cutoff={cutoff} harmonic={} harmonics={harmonics:?}",
                    index + 1
                );
            }
            assert!(
                harmonics[0] > harmonics[1]
                    && harmonics[1] > harmonics[2]
                    && harmonics[2] > harmonics[3]
                    && harmonics[3] > harmonics[4]
                    && harmonics[4] > harmonics[5],
                "cutoff={cutoff} harmonics={harmonics:?}"
            );
            if cutoff == 410.0 {
                assert!(
                    (-66.9..=-64.9).contains(&harmonics[1])
                        && (-82.5..=-80.5).contains(&harmonics[2])
                        && harmonics[3] < -120.0
                        && harmonics[4] < -128.0
                        && harmonics[5] < -138.0,
                    "410 Hz Prophet-reference profile missed: {harmonics:?}"
                );
            }
        }

        let soft_sensitive =
            live_self_oscillation_analyzer_harmonics_db(440.0, 1.0, 0.25, 1.0, false, 36)[0];
        let soft_independent =
            live_self_oscillation_analyzer_harmonics_db(440.0, 1.0, 0.25, 0.0, false, 36)[0];
        assert!(soft_sensitive < -34.0, "level={soft_sensitive}");
        assert!(
            (-25.0..=-23.0).contains(&soft_independent),
            "level={soft_independent}"
        );
    }

    #[test]
    fn gain_limited_self_oscillation_is_consistent_with_an_oscillator_active() {
        let autonomous =
            live_self_oscillation_analyzer_harmonics_db(739.99, 1.0, 1.0, 1.0, false, 36);
        let driven = live_self_oscillation_analyzer_harmonics_db(739.99, 1.0, 1.0, 1.0, true, 36);
        assert!(
            (autonomous[0] - driven[0]).abs() <= 2.0,
            "autonomous={autonomous:?} driven={driven:?}"
        );
        assert!(
            driven[1] < -68.0 && driven[2] < -85.0 && driven[3] < -110.0 && driven[4] < -125.0,
            "oscillator-driven output regained post-filter harmonics: {driven:?}"
        );
    }

    #[test]
    fn gain_limited_does_not_add_post_cutoff_harmonics_at_color_threshold() {
        let below = live_self_oscillation_analyzer_harmonics_db(440.0, 0.93, 1.0, 1.0, true, 69);
        let above = live_self_oscillation_analyzer_harmonics_db(440.0, 0.95, 1.0, 1.0, true, 69);
        for harmonic in 1..5 {
            assert!(
                above[harmonic] <= below[harmonic] + 3.0,
                "harmonic={} below={below:?} above={above:?}",
                harmonic + 1
            );
        }
    }

    #[test]
    fn gain_limited_tpt_is_available_and_has_expected_slopes() {
        assert!(FilterType::GainLimitedTpt.is_implemented());
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for (poles, expected) in [(2, 11.0..=12.5), (4, 22.0..=24.5)] {
                let lower = sine_gain(
                    FilterType::GainLimitedTpt,
                    sample_rate,
                    CUTOFF_HZ * 4.0,
                    CUTOFF_HZ,
                    0.0,
                    poles,
                    FilterOversampling::Off,
                    1.0e-4,
                );
                let upper = sine_gain(
                    FilterType::GainLimitedTpt,
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
        }
    }

    #[test]
    #[cfg(all(not(feature = "fast-math"), feature = "filter-all"))]
    fn gain_limited_tpt_linear_response_matches_baseline() {
        const FOUR_POLE_INPUT_GAIN: f32 = 0.40;
        const FOUR_POLE_FEEDBACK: f32 = 3.75;
        const BASELINE_BASS_COMP: f32 = 1.22;
        const CALIBRATED_BASS_COMP: f32 = 0.80;

        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for poles in [2, 4] {
                for (frequency, resonance) in [
                    (CUTOFF_HZ * 0.5, 0.0),
                    (CUTOFF_HZ, 0.65),
                    (CUTOFF_HZ * 2.0, 0.0),
                ] {
                    let gain = |filter_type| {
                        sine_gain(
                            filter_type,
                            sample_rate,
                            frequency,
                            CUTOFF_HZ,
                            resonance,
                            poles,
                            FilterOversampling::Off,
                            1.0e-4,
                        )
                    };
                    let baseline = gain(FilterType::DistributedNewtonTpt);
                    let candidate = gain(FilterType::GainLimitedTpt);
                    let expected = if poles == 2 {
                        baseline
                    } else {
                        let shaped_resonance = resonance.powf(1.75);
                        let baseline_compensation =
                            1.0 + shaped_resonance * FOUR_POLE_FEEDBACK * BASELINE_BASS_COMP;
                        let calibrated_compensation =
                            1.0 + shaped_resonance * FOUR_POLE_FEEDBACK * CALIBRATED_BASS_COMP;
                        baseline * FOUR_POLE_INPUT_GAIN * calibrated_compensation
                            / baseline_compensation
                    };
                    let relative_error = (candidate - expected).abs() / expected.max(1.0e-9);
                    assert!(
                        relative_error < 2.0e-4,
                        "sr={sample_rate} poles={poles} frequency={frequency} baseline={baseline} expected={expected} candidate={candidate}"
                    );
                }
            }
        }
    }

    #[test]
    fn gain_limited_tpt_level_matches_baseline_and_pitch_tracks_five_cutoffs() {
        // Retain the baseline level calibration, but tune pitch to the musical
        // cutoff grid instead of inheriting the baseline model's sharp limit cycle.
        for cutoff_hz in [110.0, 220.0, 440.0, 880.0, 1760.0] {
            let baseline = self_oscillation_tail(
                FilterType::DistributedNewtonTpt,
                cutoff_hz,
                1.0,
                FilterOversampling::Off,
            );
            let candidate = self_oscillation_tail(
                FilterType::GainLimitedTpt,
                cutoff_hz,
                1.0,
                FilterOversampling::Off,
            );
            let baseline = &baseline[24_000..];
            let candidate = &candidate[24_000..];
            let baseline_rms = rms(baseline);
            let candidate_rms = rms(candidate);
            let candidate_pitch = positive_crossing_pitch(candidate);
            let pitch_error_cents = 1200.0 * (candidate_pitch / cutoff_hz).log2();

            assert!(
                (candidate_rms / baseline_rms - 1.0).abs() < 0.08,
                "cutoff={cutoff_hz} baseline_rms={baseline_rms} candidate_rms={candidate_rms}"
            );
            assert!(
                pitch_error_cents.abs() < 20.0,
                "cutoff={cutoff_hz} candidate_pitch={candidate_pitch} error={pitch_error_cents} cents"
            );
        }
    }

    #[test]
    fn gain_limited_tpt_resonance_progression_is_smooth_at_musical_level() {
        let gains = [0.70, 0.71, 0.72, 0.74, 0.75, 0.76, 0.80].map(|resonance| {
            sine_gain(
                FilterType::GainLimitedTpt,
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
        let threshold_step_db = 20.0 * (gains[2] / gains[1]).log10();
        let reported_step_db = 20.0 * (gains[4] / gains[3]).log10();
        assert!(
            threshold_step_db < 0.75,
            "gains={gains:?} step={threshold_step_db}dB"
        );
        assert!(
            reported_step_db < 0.8,
            "gains={gains:?} step={reported_step_db}dB"
        );
    }

    #[test]
    fn gain_limited_tpt_global_oversampling_does_not_switch_at_threshold() {
        let mut filter = configured_filter(
            FilterType::GainLimitedTpt,
            CUTOFF_HZ,
            0.70,
            4,
            FilterOversampling::Auto,
        );
        let step = core::f32::consts::TAU * CUTOFF_HZ / SAMPLE_RATE;
        let mut phase = 0.0f32;
        let mut previous = 0.0;
        for _ in 0..24_000 {
            previous = process(
                &mut filter,
                WideF32::splat(phase.sin() * 0.1),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
            phase += step;
        }
        filter.set_resonance(0.72);
        let crossed = process(
            &mut filter,
            WideF32::splat(phase.sin() * 0.1),
            WideF32::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array()[0];
        assert!(
            (crossed - previous).abs() < 0.04,
            "threshold crossing jumped: before={previous} after={crossed}"
        );
    }

    #[test]
    fn gain_limited_tpt_self_oscillation_onset_and_modes_are_stable() {
        for resonance in [0.85, 0.90, 0.95, 1.0] {
            let baseline = self_oscillation_tail(
                FilterType::DistributedNewtonTpt,
                CUTOFF_HZ,
                resonance,
                FilterOversampling::Off,
            );
            let candidate = self_oscillation_tail(
                FilterType::GainLimitedTpt,
                CUTOFF_HZ,
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
                    (0.75..=1.2).contains(&ratio),
                    "resonance={resonance} baseline={baseline_rms} candidate={candidate_rms}"
                );
            }
        }

        for oversampling in [
            FilterOversampling::Off,
            FilterOversampling::Auto,
            FilterOversampling::X2,
            FilterOversampling::X4,
        ] {
            let samples =
                self_oscillation_tail(FilterType::GainLimitedTpt, CUTOFF_HZ, 1.0, oversampling);
            let tail = &samples[24_000..];
            let tail_rms = rms(tail);
            let peak = tail
                .iter()
                .fold(0.0f32, |peak, value| peak.max(value.abs()));
            assert!(tail.iter().all(|value| value.is_finite()));
            assert!(
                (0.4..0.6).contains(&tail_rms),
                "mode={oversampling:?} rms={tail_rms}"
            );
            assert!(peak < 1.0, "mode={oversampling:?} peak={peak}");
        }
    }

    #[test]
    fn gain_limited_tpt_two_pole_resonance_decays() {
        let mut filter = configured_filter(
            FilterType::GainLimitedTpt,
            CUTOFF_HZ,
            1.0,
            2,
            FilterOversampling::X4,
        );
        for _ in 0..128 {
            let _ = process(
                &mut filter,
                WideF32::splat(0.1),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            );
        }
        let mut first_energy = 0.0;
        let mut last_energy = 0.0;
        for frame in 0..24_000 {
            let output = process(
                &mut filter,
                WideF32::ZERO,
                WideF32::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
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
    fn gain_limited_tpt_remains_finite_across_control_grid() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for oversampling in [
                FilterOversampling::Off,
                FilterOversampling::X2,
                FilterOversampling::X4,
            ] {
                for poles in [2, 4] {
                    for resonance in [0.0, 0.71, 0.9, 1.0] {
                        let mut filter = configured_filter(
                            FilterType::GainLimitedTpt,
                            CUTOFF_HZ,
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
                                WideF32::new(core::array::from_fn(|i| {
                                    [0.8, -0.8, 0.25, -0.25][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [24.0, 60.0, 96.0, 120.0][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [0.0, 0.33, 0.66, 1.0][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [0.0, 0.33, 0.66, 1.0][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [phase.sin(), phase.cos(), -phase.sin(), -phase.cos()][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [-48.0, -12.0, 12.0, 48.0][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [-0.2, -0.05, 0.05, 0.2][i % 4]
                                })),
                                WideF32::new(core::array::from_fn(|i| {
                                    [-0.25, 0.0, 0.25, 0.5][i % 4]
                                })),
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

    #[cfg(not(feature = "wide-1"))]
    #[test]
    fn gain_limited_tpt_reset_and_lanes_are_independent() {
        let make = || {
            configured_filter(
                FilterType::GainLimitedTpt,
                CUTOFF_HZ,
                0.95,
                4,
                FilterOversampling::X4,
            )
        };
        let mut mixed = make();
        let mut lane_zero = make();
        let mut lane_one = make();
        for frame in 0..512 {
            let input = (frame as f32 * 0.037).sin() * 0.1;
            let mixed_output = mixed
                .process(
                    WideF32::splat(input),
                    WideF32::splat(69.0),
                    WideF32::ZERO,
                    WideF32::splat(1.0),
                    WideF32::ZERO,
                    WideF32::ZERO,
                    WideF32::new(core::array::from_fn(|i| [-0.25, 0.05, -0.25, 0.05][i % 4])),
                    WideF32::ZERO,
                    SAMPLE_RATE,
                )
                .to_array();
            let zero = lane_zero
                .process(
                    WideF32::splat(input),
                    WideF32::splat(69.0),
                    WideF32::ZERO,
                    WideF32::splat(1.0),
                    WideF32::ZERO,
                    WideF32::ZERO,
                    WideF32::splat(-0.25),
                    WideF32::ZERO,
                    SAMPLE_RATE,
                )
                .to_array()[0];
            let one = lane_one
                .process(
                    WideF32::splat(input),
                    WideF32::splat(69.0),
                    WideF32::ZERO,
                    WideF32::splat(1.0),
                    WideF32::ZERO,
                    WideF32::ZERO,
                    WideF32::splat(0.05),
                    WideF32::ZERO,
                    SAMPLE_RATE,
                )
                .to_array()[1];
            assert!((mixed_output[0] - zero).abs() < 1.0e-12);
            assert!((mixed_output[1] - one).abs() < 1.0e-12);
        }

        mixed.reset_lane(2);
        let mut fresh = make();
        let reset_lane = process(
            &mut mixed,
            WideF32::splat(0.1),
            WideF32::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array();
        let fresh_lane = process(
            &mut fresh,
            WideF32::splat(0.1),
            WideF32::splat(69.0),
            SAMPLE_RATE,
        )
        .to_array();
        assert_eq!(reset_lane[2], fresh_lane[2]);
        mixed.reset();
        fresh.reset();
        assert_eq!(
            process(
                &mut mixed,
                WideF32::splat(0.1),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            ),
            process(
                &mut fresh,
                WideF32::splat(0.1),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            )
        );
    }

    #[test]
    fn gain_limited_tpt_key_tracking_and_long_run_are_stable() {
        for note in [36.0, 48.0, 60.0, 72.0, 84.0] {
            let mut filter = configured_filter(
                FilterType::GainLimitedTpt,
                110.0,
                1.0,
                4,
                FilterOversampling::Off,
            );
            filter.set_key_track(1.0);
            let mut samples = Vec::with_capacity(24_000);
            for frame in 0..48_000 {
                let output = process(
                    &mut filter,
                    WideF32::ZERO,
                    WideF32::splat(note),
                    SAMPLE_RATE,
                )
                .to_array()[0];
                assert!(output.is_finite() && output.abs() < 1.0);
                if frame >= 24_000 {
                    samples.push(output);
                }
            }
            let expected = 110.0 * 2.0f32.powf((note - 36.0) / 12.0);
            let pitch = positive_crossing_pitch(&samples);
            assert!(
                (pitch / expected - 1.0).abs() < 0.05,
                "note={note} expected={expected} pitch={pitch}"
            );
        }

        let mut filter = configured_filter(
            FilterType::GainLimitedTpt,
            CUTOFF_HZ,
            1.0,
            4,
            FilterOversampling::Off,
        );
        let mut tail_energy = 0.0;
        for frame in 0..192_000 {
            let input = if frame < 24_000 { 0.1 } else { 0.0 };
            let output = process(
                &mut filter,
                WideF32::splat(input),
                WideF32::splat(69.0),
                SAMPLE_RATE,
            )
            .to_array()[0];
            assert!(output.is_finite() && output.abs() < 1.0);
            if frame >= 168_000 {
                tail_energy += output * output;
            }
        }
        assert!((tail_energy / 24_000.0).sqrt() > 0.4);
    }
}
