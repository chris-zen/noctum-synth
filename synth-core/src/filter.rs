//! Rev2/Curtis-inspired low-pass filter.

use wide::f32x4;

use crate::LANES;

/// Lowest cutoff accepted by the filter core.
const MIN_CUTOFF_HZ: f32 = 20.0;
/// Highest cutoff accepted by the filter core.
const MAX_CUTOFF_HZ: f32 = 18_000.0;
/// Full filter-envelope modulation depth in semitones.
const ENV_DEPTH_SEMITONES: f32 = 96.0;
/// Full audio-rate filter modulation depth in semitones.
const AUDIO_MOD_DEPTH_SEMITONES: f32 = 48.0;
/// MIDI note that produces zero semitones of filter keyboard tracking.
///
/// A lower reference than middle C matches Prophet-style behavior where full
/// keyboard tracking substantially opens the self-oscillating filter at C4.
const KEY_TRACK_REFERENCE_NOTE: f32 = 36.0;
/// Exponent applied to the public resonance control before DSP calibration.
const RESONANCE_CONTROL_EXPONENT: f32 = 1.75;
/// Maximum 2-pole feedback; intentionally below self-oscillation.
const TWO_POLE_MAX_RESONANCE: f32 = 1.9;
/// Maximum linear 4-pole feedback before the nonlinear self-oscillation region.
const FOUR_POLE_MAX_RESONANCE: f32 = 3.75;
/// SynthLab-style fraction of 4-pole feedback reused for bass compensation.
const RESONANCE_BASS_COMP: f32 = 1.22;
/// Public resonance value where 4-pole nonlinear self-oscillation begins.
pub const SELF_OSC_RESONANCE_START: f32 = 0.71;
/// Nonlinear 4-pole feedback at the start of audible self-oscillation.
const FOUR_POLE_SELF_OSC_START_RESONANCE: f32 = 4.05;
/// Maximum 4-pole feedback used by the nonlinear self-oscillation solver.
const FOUR_POLE_SELF_OSC_RESONANCE: f32 = 5.25;
/// Pitch trim, in cents, applied as nonlinear self-oscillation fades in.
///
/// The TPT cascade's free-running oscillation lands slightly flat relative to
/// the user-facing cutoff value; this trim keeps max-resonance oscillation
/// closer to measured Prophet-family behavior. Lower this value if the
/// self-oscillation beats too quickly above a tuned oscillator; raise it if the
/// self-oscillation is audibly flat.
pub const SELF_OSC_PITCH_TUNING_CENTS: f32 = 133.0;
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

/// Runtime quality setting for nonlinear filter self-oscillation oversampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "std", serde(rename_all = "snake_case"))]
pub enum FilterOversampling {
    /// Disable filter oversampling.
    Off,
    /// Select an oversampling factor from the effective audio sample rate.
    #[default]
    Auto,
    /// Run the nonlinear self-oscillation path at twice the audio sample rate.
    X2,
    /// Run the nonlinear self-oscillation path at four times the audio sample rate.
    X4,
}

impl FilterOversampling {
    /// Resolves this setting to an actual integer oversampling factor.
    pub fn factor(self, sample_rate: f32) -> usize {
        match self {
            Self::Off => 1,
            Self::Auto if sample_rate >= 176_400.0 => 1,
            Self::Auto if sample_rate >= 88_200.0 => 2,
            Self::Auto => 4,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
}

/// Four-lane Rev2-style low-pass with 2- or 4-pole slope, key tracking, and
/// 4-pole self-oscillation.
pub struct LadderFilter {
    cutoff: f32,
    resonance: f32,
    poles: u8,
    key_track: f32,
    env_amount: f32,
    env_velocity_amount: f32,
    audio_mod: f32,
    self_osc_pitch_tuning_cents: f32,
    oversampling: FilterOversampling,
    z: [f32x4; 4],
    oversample_decimator_z: [f32x4; OVERSAMPLE_DECIMATOR_POLES],
    excitation_seed: [u32; LANES],
}

impl Default for LadderFilter {
    fn default() -> Self {
        Self {
            cutoff: MAX_CUTOFF_HZ,
            resonance: 0.0,
            poles: 4,
            key_track: 0.0,
            env_amount: 0.0,
            env_velocity_amount: 0.0,
            audio_mod: 0.0,
            self_osc_pitch_tuning_cents: SELF_OSC_PITCH_TUNING_CENTS,
            oversampling: FilterOversampling::Auto,
            z: [f32x4::splat(0.0); 4],
            oversample_decimator_z: [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES],
            excitation_seed: [0x1234_5678, 0x8765_4321, 0x9e37_79b9, 0x7f4a_7c15],
        }
    }
}

impl LadderFilter {
    /// Sets the base cutoff frequency in hertz.
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
    }

    /// Returns the base cutoff frequency in hertz.
    pub fn cutoff(&self) -> f32 {
        self.cutoff
    }

    /// Sets the public resonance control value.
    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 1.0);
    }

    /// Returns the public resonance control value.
    pub fn resonance(&self) -> f32 {
        self.resonance
    }

    /// Selects the low-pass slope. Values up to 2 select 2-pole mode; all
    /// higher values select 4-pole mode.
    pub fn set_poles(&mut self, poles: u8) {
        self.poles = if poles <= 2 { 2 } else { 4 };
    }

    /// Sets keyboard tracking depth from no tracking to full tracking.
    pub fn set_key_track(&mut self, key_track: f32) {
        self.key_track = key_track.clamp(0.0, 1.0);
    }

    /// Sets filter envelope depth as a bipolar fraction of the internal
    /// envelope modulation range.
    pub fn set_env_amount(&mut self, env_amount: f32) {
        self.env_amount = env_amount.clamp(-1.0, 1.0);
    }

    /// Sets how much note velocity scales the filter envelope.
    pub fn set_env_velocity_amount(&mut self, env_velocity_amount: f32) {
        self.env_velocity_amount = env_velocity_amount.clamp(0.0, 1.0);
    }

    /// Sets audio-rate cutoff modulation depth from oscillator 1.
    pub fn set_audio_mod(&mut self, audio_mod: f32) {
        self.audio_mod = audio_mod.clamp(0.0, 1.0);
    }

    /// Sets the oversampling policy for the nonlinear self-oscillation path.
    ///
    /// Changing this at runtime clears only the oversampling decimator state so
    /// the filter core keeps its musical state while avoiding stale decimator
    /// tails from a previous factor.
    pub fn set_oversampling(&mut self, oversampling: FilterOversampling) {
        if self.oversampling != oversampling {
            self.oversampling = oversampling;
            self.oversample_decimator_z = [f32x4::splat(0.0); OVERSAMPLE_DECIMATOR_POLES];
        }
    }

    /// Returns the current oversampling policy.
    pub fn oversampling(&self) -> FilterOversampling {
        self.oversampling
    }

    /// Overrides the self-oscillation pitch trim for calibration.
    ///
    /// The public synth parameter surface does not expose this value; callers
    /// should normally use [`SELF_OSC_PITCH_TUNING_CENTS`]. This setter exists
    /// so measurement tests and analysis tools can sweep the trim without
    /// recompiling the crate.
    pub fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.self_osc_pitch_tuning_cents = cents.clamp(-1200.0, 1200.0);
    }

    /// Returns the self-oscillation pitch trim in cents.
    pub fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.self_osc_pitch_tuning_cents
    }

    /// Returns true when the public controls describe a fully open,
    /// unmodulated filter.
    ///
    /// This is only a control-state predicate. The voice path still processes
    /// the filter so analyzer and audio paths share the same insertion gain.
    pub fn is_neutral(&self) -> bool {
        self.cutoff >= MAX_CUTOFF_HZ
            && self.resonance < 0.01
            && self.key_track == 0.0
            && self.env_amount == 0.0
            && self.audio_mod == 0.0
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
    pub fn process(
        &mut self,
        input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        let resonance_control = (f32x4::splat(self.resonance) + resonance_mod)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));

        if self.uses_nonlinear_self_oscillation(resonance_control) {
            let shaped_resonance = shape_resonance_control(resonance_control);
            let factor = self.oversampling.factor(sample_rate);
            if factor > 1 {
                return self.process_oversampled_self_oscillation(
                    input,
                    note,
                    filter_env,
                    velocity,
                    osc1_audio,
                    cutoff_mod_semitones,
                    resonance_mod,
                    resonance_control,
                    shaped_resonance,
                    audio_mod,
                    sample_rate,
                    factor,
                );
            }

            let g = self.coefficients(
                note,
                filter_env,
                velocity,
                osc1_audio,
                cutoff_mod_semitones,
                resonance_mod,
                resonance_control,
                audio_mod,
                sample_rate,
            );
            return self.process_self_oscillation_sample(
                input,
                g,
                shaped_resonance,
                resonance_control,
            );
        }

        let shaped_resonance = shape_resonance_control(resonance_control);
        let g = self.coefficients(
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            resonance_mod,
            resonance_control,
            audio_mod,
            sample_rate,
        );
        let max_resonance = if self.poles == 2 {
            TWO_POLE_MAX_RESONANCE
        } else {
            FOUR_POLE_MAX_RESONANCE
        };
        let resonance = shaped_resonance * f32x4::splat(max_resonance);
        let driven_input =
            self.resonance_compensated_input(analog_soft_clip(input), shaped_resonance);
        self.process_linear_cascade(driven_input, g, resonance)
    }

    fn process_oversampled_self_oscillation(
        &mut self,
        input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        resonance_control: f32x4,
        shaped_resonance: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
        factor: usize,
    ) -> f32x4 {
        let amount = self.self_oscillation_amount(resonance_control);
        let driven_input =
            self.resonance_compensated_input(analog_soft_clip(input), shaped_resonance);
        let blend = self_oscillation_blend(amount);

        // Keep the linear branch at the host sample rate. Oversampling the
        // mixed output would run the mostly-linear response through the
        // decimator as soon as resonance crosses the self-oscillation
        // threshold, creating an analyzer-visible response jump.
        if all_lanes_near_zero(f32x4::splat(1.0) - blend) {
            return self.process_oversampled_nonlinear_self_oscillation(
                driven_input,
                note,
                filter_env,
                velocity,
                osc1_audio,
                cutoff_mod_semitones,
                resonance_mod,
                resonance_control,
                amount,
                audio_mod,
                sample_rate,
                factor,
            );
        }

        let original_z = self.z;
        let g = self.coefficients(
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            resonance_mod,
            resonance_control,
            audio_mod,
            sample_rate,
        );
        let linear_output = self.process_linear_cascade(
            driven_input,
            g,
            shaped_resonance * f32x4::splat(FOUR_POLE_MAX_RESONANCE),
        );
        let linear_z = self.z;

        self.z = original_z;
        let nonlinear_output = self.process_oversampled_nonlinear_self_oscillation(
            driven_input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            resonance_mod,
            resonance_control,
            amount,
            audio_mod,
            sample_rate,
            factor,
        );
        let nonlinear_z = self.z;

        self.blend_filter_state(linear_z, nonlinear_z, blend);
        linear_output * (f32x4::splat(1.0) - blend) + nonlinear_output * blend
    }

    fn process_oversampled_nonlinear_self_oscillation(
        &mut self,
        driven_input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        resonance_control: f32x4,
        amount: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
        factor: usize,
    ) -> f32x4 {
        let oversampled_rate = sample_rate * factor as f32;
        let k = self_oscillation_feedback(amount);
        let mut output = f32x4::splat(0.0);

        for _ in 0..factor {
            let g = self.coefficients(
                note,
                filter_env,
                velocity,
                osc1_audio,
                cutoff_mod_semitones,
                resonance_mod,
                resonance_control,
                audio_mod,
                oversampled_rate,
            );
            let excitation = self.self_oscillation_excitation(amount);
            output = self.process_nonlinear_four_pole(driven_input + excitation, g, k)
                * self_oscillation_output_makeup(amount);
            output = self.decimate_oversampled_output(output, sample_rate, oversampled_rate);
        }

        output
    }

    fn process_self_oscillation_sample(
        &mut self,
        input: f32x4,
        g: f32x4,
        shaped_resonance: f32x4,
        resonance_control: f32x4,
    ) -> f32x4 {
        let amount = self.self_oscillation_amount(resonance_control);
        let k = self_oscillation_feedback(amount);
        let driven_input =
            self.resonance_compensated_input(analog_soft_clip(input), shaped_resonance);
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

    fn blend_filter_state(
        &mut self,
        linear_z: [f32x4; 4],
        nonlinear_z: [f32x4; 4],
        blend: f32x4,
    ) {
        let linear_blend = f32x4::splat(1.0) - blend;
        for stage in 0..self.z.len() {
            self.z[stage] = linear_z[stage] * linear_blend + nonlinear_z[stage] * blend;
        }
    }

    fn process_linear_cascade(&mut self, driven_input: f32x4, g: f32x4, resonance: f32x4) -> f32x4 {
        let x = self.zero_delay_feedback_input(driven_input, g, resonance);

        let y0 = tpt_one_pole(x, &mut self.z[0], g);
        let y1 = tpt_one_pole(y0, &mut self.z[1], g);
        let y2 = tpt_one_pole(y1, &mut self.z[2], g);
        let y3 = tpt_one_pole(y2, &mut self.z[3], g);

        if self.poles == 2 { y1 } else { y3 }
    }

    fn uses_nonlinear_self_oscillation(&self, resonance_control: f32x4) -> bool {
        self.poles == 4
            && resonance_control
                .simd_gt(f32x4::splat(SELF_OSC_RESONANCE_START))
                .any()
    }

    fn self_oscillation_amount(&self, resonance_control: f32x4) -> f32x4 {
        ((resonance_control - f32x4::splat(SELF_OSC_RESONANCE_START))
            / f32x4::splat(1.0 - SELF_OSC_RESONANCE_START))
        .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
    }

    fn resonance_compensated_input(&self, input: f32x4, shaped_resonance: f32x4) -> f32x4 {
        if self.poles == 4 {
            input
                * (f32x4::splat(1.0)
                    + shaped_resonance
                        * f32x4::splat(FOUR_POLE_MAX_RESONANCE * RESONANCE_BASS_COMP))
        } else {
            input
        }
    }

    fn zero_delay_feedback_input(&self, input: f32x4, g: f32x4, resonance: f32x4) -> f32x4 {
        let one = f32x4::splat(1.0);
        let s0 = stage_offset(self.z[0], g);
        let s1 = stage_offset(self.z[1], g);
        let s2 = stage_offset(self.z[2], g);
        let s3 = stage_offset(self.z[3], g);

        if self.poles == 2 {
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

        let u = self.zero_delay_feedback_input(input, g, resonance);
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
            let p0 = f0 / a0;
            let q0 = -b0 / a0;
            let p1 = (f1 - c1 * p0) / a1;
            let q1 = (-c1 * q0) / a1;
            let p2 = (f2 - c2 * p1) / a2;
            let q2 = (-c2 * q1) / a2;
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

    fn coefficients(
        &self,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        resonance_control: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        if self.uses_static_cutoff(cutoff_mod_semitones, resonance_mod, audio_mod) {
            return f32x4::splat(self.static_coefficient(sample_rate, resonance_control));
        }

        let cutoff = self.modulated_cutoff(
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            resonance_control,
            audio_mod,
            sample_rate,
        );
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        coefficients_from_cutoff(cutoff, max_cutoff, sample_rate)
    }

    fn uses_static_cutoff(
        &self,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        audio_mod: f32x4,
    ) -> bool {
        self.key_track == 0.0
            && self.env_amount == 0.0
            && self.audio_mod == 0.0
            && all_lanes_near_zero(cutoff_mod_semitones)
            && all_lanes_near_zero(resonance_mod)
            && all_lanes_near_zero(audio_mod)
    }

    fn static_coefficient(&self, sample_rate: f32, resonance_control: f32x4) -> f32 {
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let hz = if self.poles == 4
            && resonance_control
                .simd_gt(f32x4::splat(SELF_OSC_RESONANCE_START))
                .any()
        {
            let self_osc_cents =
                self_oscillation_pitch_amount(self.self_oscillation_amount(resonance_control))
                    .to_array()[0]
                    * self.self_osc_pitch_tuning_cents;
            let scale = crate::math::powf(2.0, self_osc_cents / 1200.0);
            self.cutoff * scale
        } else {
            self.cutoff
        }
        .clamp(MIN_CUTOFF_HZ, max_cutoff);
        let g = crate::math::tan(core::f32::consts::PI * hz / sample_rate);
        g / (1.0 + g)
    }

    fn modulated_cutoff(
        &self,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_control: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let key_semitones =
            (note - f32x4::splat(KEY_TRACK_REFERENCE_NOTE)) * f32x4::splat(self.key_track);
        let velocity_scale = f32x4::splat(1.0 - self.env_velocity_amount)
            + velocity.clamp(f32x4::splat(0.0), f32x4::splat(1.0))
                * f32x4::splat(self.env_velocity_amount);
        let env_semitones = filter_env.clamp(f32x4::splat(0.0), f32x4::splat(1.0))
            * velocity_scale
            * f32x4::splat(self.env_amount * ENV_DEPTH_SEMITONES);
        let audio_mod_amount =
            (f32x4::splat(self.audio_mod) + audio_mod).clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let audio_semitones = osc1_audio.clamp(f32x4::splat(-1.0), f32x4::splat(1.0))
            * audio_mod_amount
            * f32x4::splat(AUDIO_MOD_DEPTH_SEMITONES);
        let self_osc_semitones = if self.poles == 4 {
            self_oscillation_pitch_amount(self.self_oscillation_amount(resonance_control))
                * f32x4::splat(self.self_osc_pitch_tuning_cents / 100.0)
        } else {
            f32x4::splat(0.0)
        };
        let total_semitones = key_semitones
            + env_semitones
            + audio_semitones
            + cutoff_mod_semitones
            + self_osc_semitones;
        let scale = (total_semitones * f32x4::splat(1.0 / 12.0)).exp2();
        (f32x4::splat(self.cutoff) * scale).clamp(
            f32x4::splat(MIN_CUTOFF_HZ),
            f32x4::splat(max_cutoff),
        )
    }
}

fn stage_offset(z: f32x4, g: f32x4) -> f32x4 {
    z * (f32x4::splat(1.0) - g)
}

fn raw_integrator_coefficient(g: f32x4) -> f32x4 {
    g / (f32x4::splat(1.0) - g)
}

fn shape_resonance_control(value: f32x4) -> f32x4 {
    let mut values = value.to_array();
    for value in &mut values {
        *value = crate::math::powf(value.clamp(0.0, 1.0), RESONANCE_CONTROL_EXPONENT);
    }
    f32x4::new(values)
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
    let y = (clamp_nonlinear_state(value) * drive).tanh();
    let derivative = f32x4::splat(1.0) - y * y;
    let y = y / drive;
    (y, derivative)
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
        let g = crate::math::tan(core::f32::consts::PI * hz / sample_rate);
        *value = g / (1.0 + g);
    }
    f32x4::new(values)
}

fn tpt_one_pole(input: f32x4, z: &mut f32x4, a: f32x4) -> f32x4 {
    let v = (input - *z) * a;
    let y = v + *z;
    *z = y + v;
    y
}
