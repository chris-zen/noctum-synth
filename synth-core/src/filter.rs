//! Nonlinear Moog-style ladder low-pass filter.

use wide::f32x4;

use crate::LANES;

const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 18_000.0;
const ENV_DEPTH_SEMITONES: f32 = 96.0;
const AUDIO_MOD_DEPTH_SEMITONES: f32 = 48.0;
const FOUR_POLE_MAX_RESONANCE: f32 = 3.6;
const TWO_POLE_MAX_RESONANCE: f32 = 1.6;
const FOUR_POLE_SELF_OSC_RESONANCE: f32 = 4.25;
const SELF_OSC_RESONANCE_START: f32 = 0.90;
const SELF_OSC_EXCITATION: f32 = 1.0e-7;
const NONLINEAR_STATE_LIMIT: f32 = 8.0;
const NONLINEAR_NEWTON_STEPS: usize = 3;

/// Four-lane ladder filter with 2- or 4-pole slope, key tracking, and self-oscillation.
pub struct LadderFilter {
    cutoff: f32,
    resonance: f32,
    poles: u8,
    key_track: f32,
    env_amount: f32,
    env_velocity_amount: f32,
    audio_mod: f32,
    z: [f32x4; 4],
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
            z: [f32x4::splat(0.0); 4],
            excitation_seed: [0x1234_5678, 0x8765_4321, 0x9e37_79b9, 0x7f4a_7c15],
        }
    }
}

impl LadderFilter {
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
    }

    pub fn cutoff(&self) -> f32 {
        self.cutoff
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 1.0);
    }

    pub fn resonance(&self) -> f32 {
        self.resonance
    }

    pub fn set_poles(&mut self, poles: u8) {
        self.poles = if poles <= 2 { 2 } else { 4 };
    }

    pub fn set_key_track(&mut self, key_track: f32) {
        self.key_track = key_track.clamp(0.0, 1.0);
    }

    pub fn set_env_amount(&mut self, env_amount: f32) {
        self.env_amount = env_amount.clamp(-1.0, 1.0);
    }

    pub fn set_env_velocity_amount(&mut self, env_velocity_amount: f32) {
        self.env_velocity_amount = env_velocity_amount.clamp(0.0, 1.0);
    }

    pub fn set_audio_mod(&mut self, audio_mod: f32) {
        self.audio_mod = audio_mod.clamp(0.0, 1.0);
    }

    pub fn is_neutral(&self) -> bool {
        self.cutoff >= MAX_CUTOFF_HZ
            && self.resonance < 0.01
            && self.key_track == 0.0
            && self.env_amount == 0.0
            && self.audio_mod == 0.0
    }

    pub fn reset(&mut self) {
        self.z = [f32x4::splat(0.0); 4];
    }

    pub fn reset_lane(&mut self, lane: usize) {
        for stage in &mut self.z {
            let mut values = stage.to_array();
            values[lane] = 0.0;
            *stage = f32x4::new(values);
        }
    }

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
        let g = self.coefficients(
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            audio_mod,
            sample_rate,
        );
        let resonance_value = (f32x4::splat(self.resonance) + resonance_mod)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        if self.uses_nonlinear_self_oscillation() {
            let amount = self.self_oscillation_amount();
            let k = resonance_value
                * f32x4::splat(
                    FOUR_POLE_MAX_RESONANCE
                        + amount * (FOUR_POLE_SELF_OSC_RESONANCE - FOUR_POLE_MAX_RESONANCE),
                );
            let excitation = self.self_oscillation_excitation(amount);
            return self.process_nonlinear_four_pole(input + excitation, g, k);
        }

        let max_resonance = if self.poles == 2 {
            TWO_POLE_MAX_RESONANCE
        } else {
            FOUR_POLE_MAX_RESONANCE
        };
        let resonance = resonance_value * f32x4::splat(max_resonance);
        let x = self.zero_delay_feedback_input(input, g, resonance);

        let y0 = tpt_one_pole(x, &mut self.z[0], g);
        let y1 = tpt_one_pole(y0, &mut self.z[1], g);
        let y2 = tpt_one_pole(y1, &mut self.z[2], g);
        let y3 = tpt_one_pole(y2, &mut self.z[3], g);

        if self.poles == 2 { y1 } else { y3 }
    }

    fn uses_nonlinear_self_oscillation(&self) -> bool {
        self.poles == 4 && self.resonance > SELF_OSC_RESONANCE_START
    }

    fn self_oscillation_amount(&self) -> f32 {
        ((self.resonance - SELF_OSC_RESONANCE_START) / (1.0 - SELF_OSC_RESONANCE_START))
            .clamp(0.0, 1.0)
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

    fn self_oscillation_excitation(&mut self, amount: f32) -> f32x4 {
        let gain = SELF_OSC_EXCITATION * amount * amount;
        let mut out = [0.0; LANES];
        for (lane, sample) in out.iter_mut().enumerate() {
            let seed = self.excitation_seed[lane]
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            self.excitation_seed[lane] = seed;
            let normalized = ((seed >> 8) as f32) * (1.0 / 16_777_216.0);
            *sample = (normalized * 2.0 - 1.0) * gain;
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
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        if self.uses_static_cutoff(cutoff_mod_semitones, audio_mod) {
            return f32x4::splat(self.static_coefficient(sample_rate));
        }

        let cutoff = self.modulated_cutoff(
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            audio_mod,
            sample_rate,
        );
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        coefficients_from_cutoff(cutoff, max_cutoff, sample_rate)
    }

    fn uses_static_cutoff(&self, cutoff_mod_semitones: f32x4, audio_mod: f32x4) -> bool {
        self.key_track == 0.0
            && self.env_amount == 0.0
            && self.audio_mod == 0.0
            && all_lanes_near_zero(cutoff_mod_semitones)
            && all_lanes_near_zero(audio_mod)
    }

    fn static_coefficient(&self, sample_rate: f32) -> f32 {
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);
        let hz = self.cutoff.clamp(MIN_CUTOFF_HZ, max_cutoff);
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
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        let notes = note.to_array();
        let env = filter_env.to_array();
        let velocities = velocity.to_array();
        let osc1 = osc1_audio.to_array();
        let audio_mod = audio_mod.to_array();
        let cutoff_mod_semitones = cutoff_mod_semitones.to_array();
        let mut cutoff = [0.0; LANES];
        let max_cutoff = (sample_rate * 0.45).min(MAX_CUTOFF_HZ);

        for lane in 0..LANES {
            let key_semitones = (notes[lane] - 60.0) * self.key_track;
            let velocity_scale = 1.0 - self.env_velocity_amount
                + velocities[lane].clamp(0.0, 1.0) * self.env_velocity_amount;
            let env_semitones =
                env[lane].clamp(0.0, 1.0) * velocity_scale * self.env_amount * ENV_DEPTH_SEMITONES;
            let audio_mod_amount = (self.audio_mod + audio_mod[lane]).clamp(0.0, 1.0);
            let audio_semitones =
                osc1[lane].clamp(-1.0, 1.0) * audio_mod_amount * AUDIO_MOD_DEPTH_SEMITONES;
            let total_semitones =
                key_semitones + env_semitones + audio_semitones + cutoff_mod_semitones[lane];
            let scale = crate::math::powf(2.0, total_semitones / 12.0);
            cutoff[lane] = (self.cutoff * scale).clamp(MIN_CUTOFF_HZ, max_cutoff);
        }

        f32x4::new(cutoff)
    }
}

fn stage_offset(z: f32x4, g: f32x4) -> f32x4 {
    z * (f32x4::splat(1.0) - g)
}

fn raw_integrator_coefficient(g: f32x4) -> f32x4 {
    g / (f32x4::splat(1.0) - g)
}

fn nonlinear_with_derivative(value: f32x4) -> (f32x4, f32x4) {
    let y = clamp_nonlinear_state(value).tanh();
    let derivative = f32x4::splat(1.0) - y * y;
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
