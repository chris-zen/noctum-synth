//! Four-stage TPT cascade with an analytic, gain-limited feedback loop.

use crate::f32x4;

use crate::filter::{
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
fn uniform_lane_value(value: f32x4) -> Option<f32> {
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
    z: [f32x4; 4],
    oversample_decimator_z: f32x4,
    previous_output: f32x4,
    smoothed_feedback: f32x4,
}

impl Default for GainLimitedTpt {
    fn default() -> Self {
        Self {
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            static_coefficient_cache: StaticCoefficientCache::default(),
            z: [f32x4::splat(0.0); 4],
            oversample_decimator_z: f32x4::splat(0.0),
            previous_output: f32x4::splat(0.0),
            smoothed_feedback: f32x4::splat(0.0),
        }
    }
}

impl GainLimitedTpt {
    fn reset(&mut self) {
        self.z = [f32x4::splat(0.0); 4];
        self.previous_output = f32x4::splat(0.0);
        self.smoothed_feedback = f32x4::splat(0.0);
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
            *state = f32x4::new(values);
        }
    }

    fn clear_oversampling_state(&mut self) {
        self.oversample_decimator_z = f32x4::splat(0.0);
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        // Oversampling is a global run setting, independent of resonance.
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
        let transition = smoothstep(amount);
        let linear_feedback = if frame.poles == 2 {
            frame.shaped_resonance * f32x4::splat(TWO_POLE_MAX_RESONANCE)
        } else {
            frame.shaped_resonance * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE)
        };
        let requested_feedback = if frame.poles == 4 {
            self_oscillation_feedback(linear_feedback, transition)
        } else {
            linear_feedback
        };
        let nonlinear = amount.simd_gt(f32x4::splat(0.0));
        let effective_feedback = if nonlinear.any() {
            let limiter_drive = transition * f32x4::splat(LIMITER_DRIVE);
            let output_power = self.previous_output * self.previous_output;
            let limited_feedback =
                requested_feedback / (f32x4::splat(1.0) + limiter_drive * output_power);
            // This keeps the smoothing time at a stable fraction of the cutoff
            // period across pitch, sample rate, and global oversampling factors.
            let smoothing = (g * f32x4::splat(LIMITER_SMOOTHING_SCALE))
                .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
            self.smoothed_feedback += (limited_feedback - self.smoothed_feedback) * smoothing;
            self.smoothed_feedback = nonlinear.blend(self.smoothed_feedback, linear_feedback);
            self.smoothed_feedback
        } else {
            self.smoothed_feedback = linear_feedback;
            linear_feedback
        };

        let audio_input = if frame.poles == 4 {
            frame.input * f32x4::splat(AUDIO_INPUT_GAIN)
        } else {
            frame.input
        };
        let compensated_input = if frame.poles == 4 {
            audio_input
                * (f32x4::splat(1.0)
                    + frame.shaped_resonance
                        * f32x4::splat(FOUR_POLE_MAX_LINEAR_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            audio_input
        };
        let input = compensated_input + self_oscillation_excitation(amount);
        let (a, b) = self.output_affine_form(g, frame.poles);
        let u = (input - effective_feedback * b) / (f32x4::splat(1.0) + effective_feedback * a);

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
                output + self_oscillation_color(resonance_band, amount) - resonance_band
            } else {
                output
            }
        }
    }

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

    fn decimate(&mut self, output: f32x4, sample_rate: f32, oversampled_rate: f32) -> f32x4 {
        let mut state = self.oversample_decimator_z.to_array();
        let output_values = output.to_array();
        for lane in 0..state.len() {
            if state[lane] == 0.0 {
                state[lane] = output_values[lane];
            }
        }
        self.oversample_decimator_z = f32x4::new(state);
        let cutoff = sample_rate * 0.45;
        let raw = crate::math::tan(core::f32::consts::PI * cutoff / oversampled_rate);
        let g = f32x4::splat(raw / (1.0 + raw));
        tpt_one_pole(output, &mut self.oversample_decimator_z, g)
    }

    fn coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> f32x4 {
        if frame.static_cutoff {
            return f32x4::splat(self.static_coefficient(frame, sample_rate));
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
            f32x4::splat(pitch)
        } else if frame.poles == 4 {
            smoothstep(self_oscillation_amount(frame.resonance_control))
                * f32x4::splat(self.self_osc_pitch_tuning_cents / 100.0)
        } else {
            f32x4::splat(0.0)
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
        let cutoff = (frame.cutoff_hz * crate::math::powf(2.0, pitch_cents / 1200.0))
            .clamp(MIN_CUTOFF_HZ, max_cutoff);
        let raw = crate::math::tan(core::f32::consts::PI * cutoff / sample_rate);
        let value = raw / (1.0 + raw);
        self.static_coefficient_cache = StaticCoefficientCache { key, value };
        value
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

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        GainLimitedTpt::process(self, frame)
    }
}

fn self_oscillation_amount(resonance_control: f32x4) -> f32x4 {
    ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
        / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
    .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
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

fn self_oscillation_feedback(linear: f32x4, transition: f32x4) -> f32x4 {
    let target = f32x4::splat(FOUR_POLE_SELF_OSC_START_RESONANCE)
        + transition
            * f32x4::splat(FOUR_POLE_SELF_OSC_MAX_RESONANCE - FOUR_POLE_SELF_OSC_START_RESONANCE);
    linear + (target - linear) * transition
}

fn self_oscillation_output_makeup(transition: f32x4) -> f32x4 {
    f32x4::splat(1.0) + transition * f32x4::splat(SELF_OSC_OUTPUT_MAKEUP - 1.0)
}

/// Shapes the autonomous self-oscillation with a finite Chebyshev polynomial.
/// The second and third partials use relative coefficients 0.0076 and -0.00116;
/// tiny fourth/fifth-partial terms (0.000036/-0.000015) suppress residual upper
/// color without adding another polynomial order or an unlimited harmonic tail.
fn self_oscillation_color(output: f32x4, amount: f32x4) -> f32x4 {
    let ramp = ((amount - f32x4::splat(0.8)) * f32x4::splat(5.0))
        .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    let normalized = (output * f32x4::splat(SELF_OSC_COLOR_REFERENCE_LEVEL_INV))
        .clamp(f32x4::splat(-1.0), f32x4::splat(1.0));
    let polynomial = f32x4::splat(SELF_OSC_COLOR_B4) + normalized * f32x4::splat(SELF_OSC_COLOR_B5);
    let polynomial = f32x4::splat(SELF_OSC_COLOR_B3) + normalized * polynomial;
    let polynomial = f32x4::splat(SELF_OSC_COLOR_B2) + normalized * polynomial;
    let polynomial = f32x4::splat(SELF_OSC_COLOR_B1) + normalized * polynomial;
    let polynomial = f32x4::splat(SELF_OSC_COLOR_B0) + normalized * polynomial;
    let colored = polynomial * f32x4::splat(SELF_OSC_COLOR_REFERENCE_LEVEL);
    output + (colored - output) * ramp
}

fn self_oscillation_excitation(amount: f32x4) -> f32x4 {
    let gain = amount * amount * f32x4::splat(SELF_OSC_EXCITATION);
    gain * f32x4::new([1.0, -0.75, 0.5, -0.25])
}

fn smoothstep(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    value * value * (f32x4::splat(3.0) - f32x4::splat(2.0) * value)
}

#[inline(always)]
fn smoothstep_scalar(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_stays_within_runtime_size_budget() {
        assert!(core::mem::size_of::<GainLimitedTpt>() <= 128);
    }

    #[test]
    fn embedded_uniform_coefficients_keep_filter_output_within_error_bounds() {
        for resonance in [0.0f32, 0.65, 0.9] {
            let shaped_resonance = crate::math::powf(resonance, 1.75);
            let resonance_control = f32x4::splat(resonance);
            let amount = self_oscillation_amount(resonance_control);
            let pitch_semitones =
                smoothstep(amount) * f32x4::splat(SELF_OSC_PITCH_TUNING_CENTS / 100.0);
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
                let cutoff_mod = f32x4::splat(triangle * 36.0);
                let input_phase = (sample % 97) as f32 / 97.0;
                let input = f32x4::new([
                    input_phase * 2.0 - 1.0,
                    0.75 - input_phase,
                    input_phase - 0.25,
                    0.5 - input_phase * 0.5,
                ]);
                let frame = FilterFrame {
                    input,
                    cutoff_hz: 1_200.0,
                    cutoff_mod_semitones: cutoff_mod,
                    cutoff_mod_uniform_semitones: Some(triangle * 36.0),
                    resonance_control,
                    shaped_resonance: f32x4::splat(shaped_resonance),
                    poles: 4,
                    oversampling: crate::FilterOversampling::Off,
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
    fn limiter_reduces_feedback_as_output_grows() {
        let requested = f32x4::splat(5.25);
        let drive = f32x4::splat(LIMITER_DRIVE);
        let previous = f32x4::new([0.0, 0.25, 0.5, 1.0]);
        let limited = requested / (f32x4::splat(1.0) + drive * previous * previous);
        assert!(limited.to_array().windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn harmonic_color_does_not_inflate_the_self_oscillation_crest() {
        let mut filter = GainLimitedTpt::default();
        let frame = FilterFrame {
            input: f32x4::ZERO,
            cutoff_hz: 739.99,
            cutoff_mod_semitones: f32x4::ZERO,
            cutoff_mod_uniform_semitones: Some(0.0),
            resonance_control: f32x4::splat(1.0),
            shaped_resonance: f32x4::splat(1.0),
            poles: 4,
            oversampling: crate::FilterOversampling::Off,
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
}
