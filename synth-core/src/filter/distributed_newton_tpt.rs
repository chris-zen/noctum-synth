//! Existing distributed-Newton TPT baseline implementation.

use crate::{LANES, f32x4};

use crate::filter::{
    FilterAlgorithm, FilterFrame, MAX_CUTOFF_HZ, MIN_CUTOFF_HZ, SELF_OSC_PITCH_TUNING_CENTS,
    SELF_OSC_RESONANCE_START,
};
/// Maximum 2-pole feedback; intentionally below self-oscillation.
const TWO_POLE_MAX_RESONANCE: f32 = 1.9;
/// Maximum linear 4-pole feedback before the nonlinear self-oscillation region.
const FOUR_POLE_MAX_RESONANCE: f32 = 3.75;
/// SynthLab-style fraction of 4-pole feedback reused for bass compensation.
const RESONANCE_BASS_COMP: f32 = 1.22;
/// Nonlinear 4-pole feedback at the start of audible self-oscillation.
const FOUR_POLE_SELF_OSC_START_RESONANCE: f32 = 4.05;
/// Maximum 4-pole feedback used by the nonlinear self-oscillation solver.
const FOUR_POLE_SELF_OSC_RESONANCE: f32 = 5.25;
/// Per-sample noise seed level that lets max resonance start from silence.
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
/// Drive applied inside the self-oscillation limiter; lower values reduce harmonic spread.
const SELF_OSC_LIMITER_DRIVE: f32 = 0.4;
/// Output calibration applied only as the nonlinear self-oscillation path fades in.
const NONLINEAR_SELF_OSC_OUTPUT_MAKEUP: f32 = 1.2;
/// Fixed internal drive into the analog-style soft clipper.
const INTERNAL_DRIVE: f32 = 0.85;
/// Absolute clamp for nonlinear Newton solver state.
const NONLINEAR_STATE_LIMIT: f32 = 8.0;
/// Newton iterations used by the nonlinear 4-pole feedback solve.
const NONLINEAR_NEWTON_STEPS: usize = 3;
/// Number of one-pole stages used to suppress oversampled nonlinear foldback.
const OVERSAMPLE_DECIMATOR_POLES: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
struct StaticCoefficientCache {
    /// Sample-rate and effective pitch-trim bits. Cutoff changes explicitly
    /// invalidate the cache through the runtime wrapper.
    key: [u32; 2],
    value: f32,
}

/// Four-lane Rev2-style low-pass with 2- or 4-pole slope, key tracking, and
/// 4-pole self-oscillation.
pub(super) struct DistributedNewtonTpt {
    self_osc_pitch_tuning_cents: f32,
    static_coefficient_cache: StaticCoefficientCache,
    z: [f32x4; 4],
    oversample_decimator_z: [f32x4; OVERSAMPLE_DECIMATOR_POLES],
    excitation_seed: [u32; LANES],
}

impl Default for DistributedNewtonTpt {
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

impl DistributedNewtonTpt {
    /// Overrides the self-oscillation pitch trim for calibration.
    ///
    /// The public synth parameter surface does not expose this value; callers
    /// should normally use [`SELF_OSC_PITCH_TUNING_CENTS`]. This setter exists
    /// so measurement tests and analysis tools can sweep the trim without
    /// recompiling the crate.
    pub fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    /// Returns the self-oscillation pitch trim in cents.
    pub fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    pub fn reset(&mut self) {
        self.z = [f32x4::splat(0.0); 4];
        self.oversample_decimator_z = [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES];
    }

    /// Clears one SIMD lane of filter and oversampling state.
    pub fn reset_lane(&mut self, lane: usize) {
        for stage in &mut self
            .z
            .iter_mut()
            .chain(self.oversample_decimator_z.iter_mut())
        {
            let mut values = stage.to_array();
            values[lane] = 0.0;
            *stage = f32x4::new(values);
        }
    }

    /// Processes one SIMD frame through the selected 2-pole or 4-pole filter.
    ///
    /// The filter stays linear below the self-oscillation threshold. Above the
    /// threshold, 4-pole mode crossfades from the linear cascade into the
    /// nonlinear self-oscillation solver; optional oversampling is applied only
    /// to that nonlinear branch.
    pub fn process(&mut self, frame: FilterFrame) -> f32x4 {
        if self.uses_nonlinear_self_oscillation(frame) {
            let factor = frame.oversampling.factor(frame.sample_rate);
            if factor > 1 {
                return self.process_oversampled_self_oscillation(frame, factor);
            }

            let g = self.coefficients(frame, frame.sample_rate);
            return self.process_self_oscillation_sample(
                frame.input,
                g,
                frame.shaped_resonance,
                frame.resonance_control,
                frame.poles,
            );
        }

        let g = self.coefficients(frame, frame.sample_rate);
        let max_resonance = if frame.poles == 2 {
            TWO_POLE_MAX_RESONANCE
        } else {
            FOUR_POLE_MAX_RESONANCE
        };
        let resonance = frame.shaped_resonance * f32x4::splat(max_resonance);
        let driven_input = self.resonance_compensated_input(
            analog_soft_clip(frame.input),
            frame.shaped_resonance,
            frame.poles,
        );
        self.process_linear_cascade(driven_input, g, resonance, frame.poles)
    }

    fn process_oversampled_self_oscillation(&mut self, frame: FilterFrame, factor: usize) -> f32x4 {
        let amount = self.self_oscillation_amount(frame.resonance_control);
        let driven_input = self.resonance_compensated_input(
            analog_soft_clip(frame.input),
            frame.shaped_resonance,
            frame.poles,
        );
        let blend = self_oscillation_blend(amount);

        // Keep the linear branch at the host sample rate. Oversampling the
        // mixed output would run the mostly-linear response through the
        // decimator as soon as resonance crosses the self-oscillation
        // threshold, creating an analyzer-visible response jump.
        if all_lanes_near_zero(f32x4::splat(1.0) - blend) {
            return self.process_oversampled_nonlinear_self_oscillation(
                driven_input,
                frame,
                amount,
                factor,
            );
        }

        let original_z = self.z;
        let g = self.coefficients(frame, frame.sample_rate);
        let linear_output = self.process_linear_cascade(
            driven_input,
            g,
            frame.shaped_resonance * f32x4::splat(FOUR_POLE_MAX_RESONANCE),
            frame.poles,
        );
        let linear_z = self.z;

        self.z = original_z;
        let nonlinear_output = self.process_oversampled_nonlinear_self_oscillation(
            driven_input,
            frame,
            amount,
            factor,
        );
        let nonlinear_z = self.z;

        self.blend_filter_state(linear_z, nonlinear_z, blend);
        linear_output * (f32x4::splat(1.0) - blend) + nonlinear_output * blend
    }

    fn process_oversampled_nonlinear_self_oscillation(
        &mut self,
        driven_input: f32x4,
        frame: FilterFrame,
        amount: f32x4,
        factor: usize,
    ) -> f32x4 {
        let oversampled_rate = frame.sample_rate * factor as f32;
        let k = self_oscillation_feedback(amount);
        let mut output = f32x4::splat(0.0);

        for _ in 0..factor {
            let g = self.coefficients(frame, oversampled_rate);
            let excitation = self.self_oscillation_excitation(amount);
            output = self.process_nonlinear_four_pole(driven_input + excitation, g, k)
                * self_oscillation_output_makeup(amount);
            output = self.decimate_oversampled_output(output, frame.sample_rate, oversampled_rate);
        }

        output
    }

    fn process_self_oscillation_sample(
        &mut self,
        input: f32x4,
        g: f32x4,
        shaped_resonance: f32x4,
        resonance_control: f32x4,
        poles: u8,
    ) -> f32x4 {
        let amount = self.self_oscillation_amount(resonance_control);
        let k = self_oscillation_feedback(amount);
        let driven_input =
            self.resonance_compensated_input(analog_soft_clip(input), shaped_resonance, poles);
        let blend = self_oscillation_blend(amount);

        if all_lanes_near_zero(f32x4::splat(1.0) - blend) {
            let excitation = self.self_oscillation_excitation(amount);
            return self.process_nonlinear_four_pole(driven_input + excitation, g, k)
                * self_oscillation_output_makeup(amount);
        }

        let original_z = self.z;
        let linear_output = self.process_linear_cascade(
            driven_input,
            g,
            shaped_resonance * f32x4::splat(FOUR_POLE_MAX_RESONANCE),
            poles,
        );
        let linear_z = self.z;

        self.z = original_z;
        let excitation = self.self_oscillation_excitation(amount);
        let nonlinear_output = self.process_nonlinear_four_pole(driven_input + excitation, g, k)
            * self_oscillation_output_makeup(amount);
        let nonlinear_z = self.z;

        self.blend_filter_state(linear_z, nonlinear_z, blend);
        linear_output * (f32x4::splat(1.0) - blend) + nonlinear_output * blend
    }

    fn decimate_oversampled_output(
        &mut self,
        output: f32x4,
        sample_rate: f32,
        oversampled_rate: f32,
    ) -> f32x4 {
        let cutoff = sample_rate * 0.45;
        let g = crate::math::tan(core::f32::consts::PI * cutoff / oversampled_rate);
        let a = f32x4::splat(g / (1.0 + g));
        let mut filtered = output;
        for z in &mut self.oversample_decimator_z {
            filtered = tpt_one_pole(filtered, z, a);
        }
        filtered
    }

    fn blend_filter_state(&mut self, linear_z: [f32x4; 4], nonlinear_z: [f32x4; 4], blend: f32x4) {
        let linear_blend = f32x4::splat(1.0) - blend;
        for stage in 0..self.z.len() {
            self.z[stage] = linear_z[stage] * linear_blend + nonlinear_z[stage] * blend;
        }
    }

    fn process_linear_cascade(
        &mut self,
        driven_input: f32x4,
        g: f32x4,
        resonance: f32x4,
        poles: u8,
    ) -> f32x4 {
        let x = self.zero_delay_feedback_input(driven_input, g, resonance, poles);

        let y0 = tpt_one_pole(x, &mut self.z[0], g);
        let y1 = tpt_one_pole(y0, &mut self.z[1], g);
        let y2 = tpt_one_pole(y1, &mut self.z[2], g);
        let y3 = tpt_one_pole(y2, &mut self.z[3], g);

        if poles == 2 { y1 } else { y3 }
    }

    fn uses_nonlinear_self_oscillation(&self, frame: FilterFrame) -> bool {
        frame.poles == 4
            && frame
                .resonance_control
                .simd_gt(f32x4::splat(SELF_OSC_RESONANCE_START))
                .any()
    }

    fn self_oscillation_amount(&self, resonance_control: f32x4) -> f32x4 {
        ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
            / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
        .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
    }

    fn resonance_compensated_input(
        &self,
        input: f32x4,
        shaped_resonance: f32x4,
        poles: u8,
    ) -> f32x4 {
        if poles == 4 {
            input
                * (f32x4::splat(1.0)
                    + shaped_resonance
                        * f32x4::splat(FOUR_POLE_MAX_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            input
        }
    }

    fn zero_delay_feedback_input(
        &self,
        input: f32x4,
        g: f32x4,
        resonance: f32x4,
        poles: u8,
    ) -> f32x4 {
        let one = f32x4::splat(1.0);
        let s0 = stage_offset(self.z[0], g);
        let s1 = stage_offset(self.z[1], g);
        let s2 = stage_offset(self.z[2], g);
        let s3 = stage_offset(self.z[3], g);

        if poles == 2 {
            let g2 = g * g;
            let state_offset = g * s0 + s1;
            let denominator = one + resonance * g2;
            return (input - resonance * state_offset) / denominator;
        }

        let g2 = g * g;
        let g3 = g2 * g;
        let g4 = g2 * g2;
        let state_offset = g3 * s0 + g2 * s1 + g * s2 + s3;
        let denominator = one + resonance * g4;
        (input - resonance * state_offset) / denominator
    }

    fn process_nonlinear_four_pole(&mut self, input: f32x4, g: f32x4, resonance: f32x4) -> f32x4 {
        let g_raw = raw_integrator_coefficient(g);
        let z0 = self.z[0];
        let z1 = self.z[1];
        let z2 = self.z[2];
        let z3 = self.z[3];
        let s0 = stage_offset(z0, g);
        let s1 = stage_offset(z1, g);
        let s2 = stage_offset(z2, g);
        let s3 = stage_offset(z3, g);

        let u = self.zero_delay_feedback_input(input, g, resonance, 4);
        let mut y0 = g * u + s0;
        let mut y1 = g * y0 + s1;
        let mut y2 = g * y1 + s2;
        let mut y3 = g * y2 + s3;

        for _ in 0..NONLINEAR_NEWTON_STEPS {
            // Coupled nonlinear ladder equations:
            // y[n] = z[n] + g_raw * (tanh(input[n]) - tanh(y[n])).
            let feedback_input = input - resonance * y3;
            let (t0, d0) = nonlinear_with_derivative(feedback_input);
            let (t1, d1) = nonlinear_with_derivative(y0);
            let (t2, d2) = nonlinear_with_derivative(y1);
            let (t3, d3) = nonlinear_with_derivative(y2);
            let (t4, d4) = nonlinear_with_derivative(y3);

            let f0 = y0 - z0 - g_raw * (t0 - t1);
            let f1 = y1 - z1 - g_raw * (t1 - t2);
            let f2 = y2 - z2 - g_raw * (t2 - t3);
            let f3 = y3 - z3 - g_raw * (t3 - t4);

            let a0 = f32x4::splat(1.0) + g_raw * d1;
            let b0 = g_raw * resonance * d0;
            let c1 = -g_raw * d1;
            let a1 = f32x4::splat(1.0) + g_raw * d2;
            let c2 = -g_raw * d2;
            let a2 = f32x4::splat(1.0) + g_raw * d3;
            let c3 = -g_raw * d3;
            let a3 = f32x4::splat(1.0) + g_raw * d4;

            // Solve the 4x4 Newton step by eliminating the lower cascade and
            // leaving only the top-right resonance feedback term.
            let (p0, q0) = divide_solver_pair(f0, -b0, a0);
            let (p1, q1) = divide_solver_pair(f1 - c1 * p0, -c1 * q0, a1);
            let (p2, q2) = divide_solver_pair(f2 - c2 * p1, -c2 * q1, a2);
            let delta3 = (f3 - c3 * p2) / (a3 + c3 * q2 + f32x4::splat(1.0e-6));
            let delta2 = p2 + q2 * delta3;
            let delta1 = p1 + q1 * delta3;
            let delta0 = p0 + q0 * delta3;

            y0 = clamp_nonlinear_state(y0 - delta0);
            y1 = clamp_nonlinear_state(y1 - delta1);
            y2 = clamp_nonlinear_state(y2 - delta2);
            y3 = clamp_nonlinear_state(y3 - delta3);
        }

        commit_tpt_output(&mut self.z[0], y0);
        commit_tpt_output(&mut self.z[1], y1);
        commit_tpt_output(&mut self.z[2], y2);
        commit_tpt_output(&mut self.z[3], y3);
        y3
    }

    fn self_oscillation_excitation(&mut self, amount: f32x4) -> f32x4 {
        let gains = (amount * amount * f32x4::splat(SELF_OSC_EXCITATION)).to_array();
        let mut out = [0.0; LANES];
        for (lane, sample) in out.iter_mut().enumerate() {
            let seed = self.excitation_seed[lane]
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            self.excitation_seed[lane] = seed;
            let normalized = ((seed >> 8) as f32) * (1.0 / 16_777_216.0);
            *sample = (normalized * 2.0 - 1.0) * gains[lane];
        }
        f32x4::new(out)
    }

    fn coefficients(&mut self, frame: FilterFrame, sample_rate: f32) -> f32x4 {
        if frame.static_cutoff {
            return f32x4::splat(self.static_coefficient(frame, sample_rate));
        }

        let cutoff = self.modulated_cutoff(frame, sample_rate);
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        coefficients_from_cutoff(cutoff, max_cutoff, sample_rate)
    }

    fn static_coefficient(&mut self, frame: FilterFrame, sample_rate: f32) -> f32 {
        let uses_pitch_tuning = frame.poles == 4
            && frame
                .resonance_control
                .simd_gt(f32x4::splat(SELF_OSC_RESONANCE_START))
                .any();
        let self_osc_cents = if uses_pitch_tuning {
            self_oscillation_pitch_amount(self.self_oscillation_amount(frame.resonance_control))
                .to_array()[0]
                * self.self_osc_pitch_tuning_cents
        } else {
            0.0
        };
        let key = [sample_rate.to_bits(), self_osc_cents.to_bits()];
        if self.static_coefficient_cache.key == key {
            return self.static_coefficient_cache.value;
        }

        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let hz = if uses_pitch_tuning {
            let scale = crate::math::exp2(self_osc_cents / 1200.0);
            frame.cutoff_hz * scale
        } else {
            frame.cutoff_hz
        }
        .clamp(MIN_CUTOFF_HZ, max_cutoff);
        let g = crate::math::tan(core::f32::consts::PI * hz / sample_rate);
        let value = g / (1.0 + g);
        self.static_coefficient_cache = StaticCoefficientCache { key, value };
        value
    }

    fn modulated_cutoff(&self, frame: FilterFrame, sample_rate: f32) -> f32x4 {
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let self_osc_semitones = if frame.poles == 4 {
            self_oscillation_pitch_amount(self.self_oscillation_amount(frame.resonance_control))
                * f32x4::splat(self.self_osc_pitch_tuning_cents / 100.0)
        } else {
            f32x4::splat(0.0)
        };
        let total_semitones = frame.cutoff_mod_semitones + self_osc_semitones;
        let scale = (total_semitones * f32x4::splat(1.0 / 12.0)).exp2();
        (f32x4::splat(frame.cutoff_hz) * scale)
            .clamp(f32x4::splat(MIN_CUTOFF_HZ), f32x4::splat(max_cutoff))
    }
}

impl FilterAlgorithm for DistributedNewtonTpt {
    fn reset(&mut self) {
        DistributedNewtonTpt::reset(self);
    }

    fn reset_lane(&mut self, lane: usize) {
        DistributedNewtonTpt::reset_lane(self, lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.static_coefficient_cache = StaticCoefficientCache::default();
    }

    fn clear_oversampling_state(&mut self) {
        self.oversample_decimator_z = [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES];
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        DistributedNewtonTpt::set_self_osc_pitch_tuning_cents(self, cents);
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        DistributedNewtonTpt::self_osc_pitch_tuning_cents(self)
    }

    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        DistributedNewtonTpt::process(self, frame)
    }
}

fn stage_offset(z: f32x4, g: f32x4) -> f32x4 {
    z * (f32x4::splat(1.0) - g)
}

fn raw_integrator_coefficient(g: f32x4) -> f32x4 {
    g / (f32x4::splat(1.0) - g)
}

fn self_oscillation_blend(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    smoothstep(value)
}

fn self_oscillation_pitch_amount(value: f32x4) -> f32x4 {
    self_oscillation_blend(value)
}

fn self_oscillation_feedback(amount: f32x4) -> f32x4 {
    let amount = amount.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    f32x4::splat(FOUR_POLE_SELF_OSC_START_RESONANCE)
        + smoothstep(amount)
            * f32x4::splat(FOUR_POLE_SELF_OSC_RESONANCE - FOUR_POLE_SELF_OSC_START_RESONANCE)
}

fn self_oscillation_output_makeup(value: f32x4) -> f32x4 {
    let makeup = f32x4::splat(NONLINEAR_SELF_OSC_OUTPUT_MAKEUP - 1.0);
    f32x4::splat(1.0) + self_oscillation_blend(value) * makeup
}

fn smoothstep(value: f32x4) -> f32x4 {
    let value = value.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    value * value * (f32x4::splat(3.0) - f32x4::splat(2.0) * value)
}

fn analog_soft_clip(value: f32x4) -> f32x4 {
    (value * f32x4::splat(INTERNAL_DRIVE)).tanh() / f32x4::splat(INTERNAL_DRIVE)
}

fn nonlinear_with_derivative(value: f32x4) -> (f32x4, f32x4) {
    let drive = f32x4::splat(SELF_OSC_LIMITER_DRIVE);
    let y = nonlinear_tanh(clamp_nonlinear_state(value) * drive);
    let derivative = f32x4::splat(1.0) - y * y;
    let y = y / drive;
    (y, derivative)
}

#[inline]
fn divide_solver_pair(left: f32x4, right: f32x4, denominator: f32x4) -> (f32x4, f32x4) {
    #[cfg(feature = "embedded-math")]
    {
        let reciprocal = f32x4::splat(1.0) / denominator;
        (left * reciprocal, right * reciprocal)
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        (left / denominator, right / denominator)
    }
}

#[inline]
fn nonlinear_tanh(value: f32x4) -> f32x4 {
    #[cfg(feature = "embedded-math")]
    {
        let limit = f32x4::splat(NONLINEAR_STATE_LIMIT * SELF_OSC_LIMITER_DRIVE);
        let x = value.clamp(-limit, limit);
        let x2 = x * x;
        let numerator = x
            * (f32x4::splat(135_135.0)
                + x2 * (f32x4::splat(17_325.0) + x2 * (f32x4::splat(378.0) + x2)));
        let denominator = f32x4::splat(135_135.0)
            + x2 * (f32x4::splat(62_370.0)
                + x2 * (f32x4::splat(3_150.0) + x2 * f32x4::splat(28.0)));
        numerator / denominator
    }
    #[cfg(not(feature = "embedded-math"))]
    {
        value.tanh()
    }
}

fn clamp_nonlinear_state(value: f32x4) -> f32x4 {
    value.clamp(
        f32x4::splat(-NONLINEAR_STATE_LIMIT),
        f32x4::splat(NONLINEAR_STATE_LIMIT),
    )
}

fn commit_tpt_output(z: &mut f32x4, y: f32x4) {
    *z = y + (y - *z);
}

fn all_lanes_near_zero(value: f32x4) -> bool {
    value.abs().simd_lt(f32x4::splat(f32::EPSILON)).all()
}

fn coefficients_from_cutoff(cutoff: f32x4, max_cutoff: f32, sample_rate: f32) -> f32x4 {
    let mut values = cutoff.to_array();
    for value in &mut values {
        let hz = value.clamp(MIN_CUTOFF_HZ, max_cutoff);
        *value = core::f32::consts::PI * hz / sample_rate;
    }
    let g = f32x4::new(values).tan();
    g / (f32x4::splat(1.0) + g)
}

fn tpt_one_pole(input: f32x4, z: &mut f32x4, a: f32x4) -> f32x4 {
    let v = (input - *z) * a;
    let y = v + *z;
    *z = y + v;
    y
}

#[cfg(all(test, feature = "embedded-math"))]
mod embedded_solver_tests {
    use super::*;

    #[test]
    fn shared_solver_reciprocal_stays_close_to_independent_division() {
        const SAMPLES: usize = 65_536;
        let mut maximum_error = 0.0f32;

        for start in (0..=SAMPLES).step_by(4) {
            let fractions: [f32; 4] =
                core::array::from_fn(|lane| (start + lane).min(SAMPLES) as f32 / SAMPLES as f32);
            let left = f32x4::new(fractions.map(|fraction| -8.0 + 16.0 * fraction));
            let right = f32x4::new(fractions.map(|fraction| 8.0 - 16.0 * fraction));
            let denominator = f32x4::new(fractions.map(|fraction| 0.5 + 3.5 * fraction));
            let (actual_left, actual_right) = divide_solver_pair(left, right, denominator);
            let expected_left = left / denominator;
            let expected_right = right / denominator;

            for (actual, expected) in actual_left
                .to_array()
                .into_iter()
                .chain(actual_right.to_array())
                .zip(
                    expected_left
                        .to_array()
                        .into_iter()
                        .chain(expected_right.to_array()),
                )
            {
                maximum_error = maximum_error.max((actual - expected).abs());
            }
        }

        assert!(
            maximum_error <= 4.0e-6,
            "shared reciprocal error {maximum_error} exceeds tolerance"
        );
    }

    #[test]
    fn rational_tanh_matches_value_and_derivative_over_solver_range() {
        const SAMPLES: usize = 65_536;
        let limit = NONLINEAR_STATE_LIMIT * SELF_OSC_LIMITER_DRIVE;
        let mut maximum_value_error = 0.0f32;
        let mut maximum_derivative_error = 0.0f32;
        let mut previous = -1.0f32;

        for start in (0..=SAMPLES).step_by(4) {
            let inputs: [f32; 4] = core::array::from_fn(|lane| {
                let index = (start + lane).min(SAMPLES);
                -limit + 2.0 * limit * index as f32 / SAMPLES as f32
            });
            let input = f32x4::new(inputs);
            let actual = nonlinear_tanh(input).to_array();
            let expected = inputs.map(libm::tanhf);

            for lane in 0..4 {
                maximum_value_error =
                    maximum_value_error.max((actual[lane] - expected[lane]).abs());
                let actual_derivative = 1.0 - actual[lane] * actual[lane];
                let expected_derivative = 1.0 - expected[lane] * expected[lane];
                maximum_derivative_error =
                    maximum_derivative_error.max((actual_derivative - expected_derivative).abs());
                assert!(actual[lane] >= previous, "approximation is not monotonic");
                assert!(actual[lane].abs() <= 1.0, "approximation is not bounded");
                previous = actual[lane];
            }

            let mirrored = nonlinear_tanh(-input).to_array();
            for lane in 0..4 {
                assert!(
                    (actual[lane] + mirrored[lane]).abs() <= f32::EPSILON,
                    "approximation is not odd"
                );
            }
        }

        assert!(
            maximum_value_error <= 5.0e-5,
            "tanh value error {maximum_value_error} exceeds tolerance"
        );
        assert!(
            maximum_derivative_error <= 1.0e-4,
            "tanh derivative error {maximum_derivative_error} exceeds tolerance"
        );
    }
}
