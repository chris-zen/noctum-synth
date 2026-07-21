//! Dual-oscillator mixer with sub oscillator, noise, sync, and glide.

use crate::analog_oscillator::EngineOscillator;
use crate::f32x4;

#[cfg(feature = "profiling")]
use crate::profiling::NoopProfiler;
use crate::{AnalogSubOscillator, GlideMode, LANES, Waveform, WhiteNoise};
#[cfg(feature = "profiling")]
use crate::{RenderProfiler, RenderStage};

// Give unassigned, keyboard-tracked lanes a real pitch so their
// phases advance before the first note when note reset is off.
const CENTER_FREQUENCY_SEMITONES: f32 = 60.0;
// The guides specify direction and mode semantics but not measured timing.
// Keep this musical logarithmic curve isolated for future hardware calibration.
const MIN_GLIDE_SECONDS: f32 = 0.001;
const MAX_GLIDE_SECONDS: f32 = 16.0;
const MIDI_GLIDE_STEP: f32 = 1.0 / 127.0;
const GLIDE_PITCH_SCALE: f32 = 65_536.0;

#[derive(Clone, Copy)]
struct GlideState {
    current: [f32; LANES],
    target: [f32; LANES],
    current_pitch_q16: [i32; LANES],
    target_pitch_q16: [i32; LANES],
    step_q16: [i32; LANES],
    remainder_q16: [i32; LANES],
    error_q16: [i32; LANES],
    total: [u32; LANES],
    remaining: [u32; LANES],
    active_mask: u8,
}

impl Default for GlideState {
    fn default() -> Self {
        Self {
            current: [CENTER_FREQUENCY_SEMITONES; LANES],
            target: [CENTER_FREQUENCY_SEMITONES; LANES],
            current_pitch_q16: [60 * 65_536; LANES],
            target_pitch_q16: [60 * 65_536; LANES],
            step_q16: [0; LANES],
            remainder_q16: [0; LANES],
            error_q16: [0; LANES],
            total: [0; LANES],
            remaining: [0; LANES],
            active_mask: 0,
        }
    }
}

/// Oscillator section for one [`crate::VoiceBlock`]: two analog oscillators, sub, and noise.
pub struct Oscillators {
    osc1: EngineOscillator,
    osc2: EngineOscillator,
    sub_osc: AnalogSubOscillator,
    noise: WhiteNoise,
    params: OscillatorsParams,
    glide: [GlideState; 2],
    last_frequency_modulation: [f32x4; 2],
    last_shape_modulation: [f32; 2],
    sample_rate: f32,
}

impl Oscillators {
    pub fn new(sample_rate: f32) -> Self {
        let mut oscillators = Self {
            osc1: EngineOscillator::new_engine(sample_rate),
            osc2: EngineOscillator::new_engine(sample_rate),
            sub_osc: AnalogSubOscillator::default(),
            noise: WhiteNoise::default(),
            params: OscillatorsParams::default(),
            glide: [GlideState::default(); 2],
            last_frequency_modulation: [f32x4::splat(0.0); 2],
            last_shape_modulation: [0.0; 2],
            sample_rate,
        };
        oscillators.apply_params_without_frequency();
        oscillators.update_frequencies();
        oscillators
    }

    pub fn params(&self) -> &OscillatorsParams {
        &self.params
    }

    pub fn osc1_frequency_hz(&self) -> f32x4 {
        self.osc1.frequency_hz()
    }

    pub fn set_osc1_enabled(&mut self, enabled: bool) {
        self.params.osc1.enabled = enabled;
        self.osc1.set_enabled(enabled);
    }

    pub fn set_osc2_enabled(&mut self, enabled: bool) {
        self.params.osc2.enabled = enabled;
        self.osc2.set_enabled(enabled);
    }

    pub fn set_osc1_waveform(&mut self, waveform: Waveform) {
        self.params.osc1.waveform = waveform;
        self.osc1.set_waveform(waveform);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        self.last_shape_modulation[0] = 0.0;
    }

    pub fn set_osc2_waveform(&mut self, waveform: Waveform) {
        self.params.osc2.waveform = waveform;
        self.osc2.set_waveform(waveform);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
        self.last_shape_modulation[1] = 0.0;
    }

    pub fn set_osc1_frequency_semitones(&mut self, semitones: f32) {
        self.params.osc1.frequency_semitones = semitones.clamp(0.0, 120.0);
        self.update_frequencies();
    }

    pub fn set_osc2_frequency_semitones(&mut self, semitones: f32) {
        self.params.osc2.frequency_semitones = semitones.clamp(0.0, 120.0);
        self.update_frequencies();
    }

    pub fn set_osc1_fine_tune_cents(&mut self, cents: f32) {
        self.params.osc1.fine_tune_cents = cents.clamp(-50.0, 50.0);
        self.update_frequencies();
    }

    pub fn set_osc2_fine_tune_cents(&mut self, cents: f32) {
        self.params.osc2.fine_tune_cents = cents.clamp(-50.0, 50.0);
        self.update_frequencies();
    }

    pub fn set_osc1_shape_mod(&mut self, shape_mod: f32) {
        self.params.osc1.shape_mod = shape_mod.clamp(0.0, 1.0);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        self.last_shape_modulation[0] = 0.0;
    }

    pub fn set_osc2_shape_mod(&mut self, shape_mod: f32) {
        self.params.osc2.shape_mod = shape_mod.clamp(0.0, 1.0);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
        self.last_shape_modulation[1] = 0.0;
    }

    pub fn set_osc1_note_reset(&mut self, note_reset: bool) {
        self.params.osc1.note_reset = note_reset;
    }

    pub fn set_osc2_note_reset(&mut self, note_reset: bool) {
        self.params.osc2.note_reset = note_reset;
    }

    pub fn set_osc1_keyboard_on(&mut self, keyboard_on: bool) {
        self.params.osc1.keyboard_on = keyboard_on;
        if !keyboard_on {
            self.snap_glide(0);
        }
        self.update_frequencies();
    }

    pub fn set_osc2_keyboard_on(&mut self, keyboard_on: bool) {
        self.params.osc2.keyboard_on = keyboard_on;
        if !keyboard_on {
            self.snap_glide(1);
        }
        self.update_frequencies();
    }

    pub fn set_osc1_glide(&mut self, amount: f32) {
        self.params.osc1.glide = amount.clamp(0.0, 1.0);
        self.retime_glide(0);
    }

    pub fn set_osc2_glide(&mut self, amount: f32) {
        self.params.osc2.glide = amount.clamp(0.0, 1.0);
        self.retime_glide(1);
    }

    pub fn set_glide_mode(&mut self, mode: GlideMode) {
        self.params.glide_mode = mode;
        self.retime_glide(0);
        self.retime_glide(1);
    }

    pub fn set_glide_enabled(&mut self, enabled: bool) {
        self.params.glide_enabled = enabled;
        if enabled {
            self.retime_glide(0);
            self.retime_glide(1);
        } else {
            self.snap_glide(0);
            self.snap_glide(1);
        }
        self.update_frequencies();
    }

    pub fn set_sync(&mut self, sync: bool) {
        self.params.sync = sync;
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.params.osc_mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_sub_octave(&mut self, level: f32) {
        self.params.sub_octave = level.clamp(0.0, 1.0);
    }

    pub fn set_noise(&mut self, level: f32) {
        self.params.noise = level.clamp(0.0, 1.0);
    }

    pub fn set_slop(&mut self, slop: f32) {
        self.params.osc_slop = slop.clamp(0.0, 1.0);
        self.osc1.set_slop_amount(self.params.osc_slop);
        self.osc2.set_slop_amount(self.params.osc_slop);
    }

    pub fn set_note_frequency(&mut self, note_frequency_hz: f32x4) {
        let semitones = note_frequency_hz.to_array().map(frequency_to_semitones);
        let pitches_q16 = semitones.map(pitch_to_q16);
        for state in &mut self.glide {
            state.current = semitones;
            state.target = semitones;
            state.current_pitch_q16 = pitches_q16;
            state.target_pitch_q16 = pitches_q16;
            state.step_q16 = [0; LANES];
            state.remainder_q16 = [0; LANES];
            state.error_q16 = [0; LANES];
            state.total = [0; LANES];
            state.remaining = [0; LANES];
            state.active_mask = 0;
        }
        self.update_frequencies();
    }

    pub(crate) fn set_note_semitones_preserving_glide(&mut self, target_semitones: [f32; LANES]) {
        for state in &mut self.glide {
            for (lane, &target) in target_semitones.iter().enumerate() {
                let target_q16 = pitch_to_q16(target);
                if state.remaining[lane] == 0 {
                    state.current[lane] = target;
                    state.current_pitch_q16[lane] = target_q16;
                } else {
                    let shift_q16 = target_q16 - state.target_pitch_q16[lane];
                    state.current_pitch_q16[lane] += shift_q16;
                    state.current[lane] = q16_to_pitch(state.current_pitch_q16[lane]);
                }
                state.target[lane] = target;
                state.target_pitch_q16[lane] = target_q16;
            }
        }
        self.update_frequencies();
    }

    pub fn note_on(&mut self, lane: usize, note_frequency_hz: f32x4) {
        let target = note_frequency_hz.to_array().map(frequency_to_semitones);
        self.note_on_with_glide(lane, target, None, false);
    }

    pub(crate) fn note_on_with_glide(
        &mut self,
        lane: usize,
        target_semitones: [f32; LANES],
        start_semitones: Option<f32>,
        glide: bool,
    ) {
        self.set_glide_target(lane, target_semitones, start_semitones, glide);
        self.osc1.trigger_lane(lane, self.params.osc1.note_reset);
        self.osc2.trigger_lane(lane, self.params.osc2.note_reset);
        if self.params.osc1.note_reset {
            self.sub_osc.reset_lane(lane);
        }
    }

    pub(crate) fn retune_with_glide(
        &mut self,
        lane: usize,
        target_semitones: [f32; LANES],
        glide: bool,
    ) {
        self.set_glide_target(lane, target_semitones, None, glide);
    }

    fn set_glide_target(
        &mut self,
        lane: usize,
        target_semitones: [f32; LANES],
        start_semitones: Option<f32>,
        should_glide: bool,
    ) {
        for oscillator in 0..2 {
            let keyboard_on = self.oscillator_params(oscillator).keyboard_on;
            let enabled = self.params.glide_enabled && should_glide && keyboard_on;
            let state = &mut self.glide[oscillator];
            state.target[lane] = target_semitones[lane];
            state.target_pitch_q16[lane] = pitch_to_q16(target_semitones[lane]);
            if let Some(start) = start_semitones {
                state.current[lane] = start;
                state.current_pitch_q16[lane] = pitch_to_q16(start);
            }
            if enabled {
                self.configure_glide_lane(oscillator, lane);
            } else {
                state.current[lane] = target_semitones[lane];
                state.current_pitch_q16[lane] = state.target_pitch_q16[lane];
                clear_glide_progress(state, lane);
                state.active_mask &= !(1 << lane);
            }
        }
        self.update_frequencies();
    }

    fn oscillator_params(&self, oscillator: usize) -> &OscillatorParams {
        if oscillator == 0 {
            &self.params.osc1
        } else {
            &self.params.osc2
        }
    }

    fn configure_glide_lane(&mut self, oscillator: usize, lane: usize) {
        let amount = self.oscillator_params(oscillator).glide;
        let fixed_time = self.params.glide_mode.is_fixed_time();
        let state = &mut self.glide[oscillator];
        let distance_q16 = state.target_pitch_q16[lane] - state.current_pitch_q16[lane];
        let distance = (distance_q16 as f32 / GLIDE_PITCH_SCALE).abs();
        if amount <= 0.0 || distance_q16 == 0 {
            state.current[lane] = state.target[lane];
            state.current_pitch_q16[lane] = state.target_pitch_q16[lane];
            clear_glide_progress(state, lane);
            state.active_mask &= !(1 << lane);
            return;
        }
        let base_seconds = glide_seconds(amount);
        let seconds = if fixed_time {
            base_seconds
        } else {
            base_seconds * distance / 12.0
        };
        let max_samples = i32::MAX as f32 - 128.0;
        let samples = crate::math::round(seconds * self.sample_rate).clamp(1.0, max_samples) as u32;
        let samples_i32 = samples as i32;
        state.step_q16[lane] = distance_q16 / samples_i32;
        state.remainder_q16[lane] = distance_q16 % samples_i32;
        state.error_q16[lane] = 0;
        state.total[lane] = samples;
        state.remaining[lane] = samples;
        state.active_mask |= 1 << lane;
    }

    fn retime_glide(&mut self, oscillator: usize) {
        for lane in 0..LANES {
            if !self.params.glide_enabled || !self.oscillator_params(oscillator).keyboard_on {
                self.snap_glide_lane(oscillator, lane);
            } else if self.glide[oscillator].remaining[lane] > 0 {
                self.configure_glide_lane(oscillator, lane);
            }
        }
        self.update_frequencies();
    }

    fn snap_glide(&mut self, oscillator: usize) {
        for lane in 0..LANES {
            self.snap_glide_lane(oscillator, lane);
        }
    }

    fn snap_glide_lane(&mut self, oscillator: usize, lane: usize) {
        let state = &mut self.glide[oscillator];
        state.current[lane] = state.target[lane];
        state.current_pitch_q16[lane] = state.target_pitch_q16[lane];
        clear_glide_progress(state, lane);
        state.active_mask &= !(1 << lane);
    }

    fn advance_glide(&mut self) -> bool {
        let mut changed = false;
        for state in &mut self.glide {
            if state.active_mask == 0 {
                continue;
            }
            for lane in 0..LANES {
                if state.active_mask & (1 << lane) == 0 {
                    continue;
                }
                state.remaining[lane] -= 1;
                if state.remaining[lane] == 0 {
                    state.current[lane] = state.target[lane];
                    state.current_pitch_q16[lane] = state.target_pitch_q16[lane];
                    clear_glide_progress(state, lane);
                    state.active_mask &= !(1 << lane);
                    changed = true;
                } else {
                    let previous_q16 = state.current_pitch_q16[lane];
                    state.current_pitch_q16[lane] += state.step_q16[lane];
                    state.error_q16[lane] += state.remainder_q16[lane];
                    let total = state.total[lane] as i32;
                    if state.error_q16[lane] >= total {
                        state.current_pitch_q16[lane] += 1;
                        state.error_q16[lane] -= total;
                    } else if state.error_q16[lane] <= -total {
                        state.current_pitch_q16[lane] -= 1;
                        state.error_q16[lane] += total;
                    }
                    if state.current_pitch_q16[lane] != previous_q16 {
                        state.current[lane] = q16_to_pitch(state.current_pitch_q16[lane]);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    pub fn update_frequencies(&mut self) {
        self.update_frequencies_modulated(f32x4::splat(0.0), f32x4::splat(0.0));
    }

    pub fn next(&mut self, modulation: OscillatorModulation) -> OscillatorsOutput {
        let shape_modulation = scalar_shape_modulation(modulation);
        #[cfg(feature = "profiling")]
        {
            return self.next_inner(modulation, shape_modulation, &mut NoopProfiler);
        }
        #[cfg(not(feature = "profiling"))]
        self.next_inner(modulation, shape_modulation)
    }

    #[cfg(not(feature = "profiling"))]
    pub(crate) fn next_prepared(
        &mut self,
        modulation: OscillatorModulation,
        shape_modulation: [f32; 2],
    ) -> OscillatorsOutput {
        self.next_inner(modulation, shape_modulation)
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn next_prepared_profiled(
        &mut self,
        modulation: OscillatorModulation,
        shape_modulation: [f32; 2],
        profiler: &mut impl RenderProfiler,
    ) -> OscillatorsOutput {
        self.next_inner(modulation, shape_modulation, profiler)
    }

    fn next_inner(
        &mut self,
        modulation: OscillatorModulation,
        shape_modulation: [f32; 2],
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) -> OscillatorsOutput {
        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::OscillatorControl);
        let frequency_modulation = [
            modulation.osc1_frequency_semitones,
            modulation.osc2_frequency_semitones,
        ];
        if self.advance_glide() || frequency_modulation != self.last_frequency_modulation {
            self.update_frequencies_modulated(frequency_modulation[0], frequency_modulation[1]);
        }

        if shape_modulation[0] != self.last_shape_modulation[0] {
            self.osc1
                .set_shape((self.params.osc1.shape_mod + shape_modulation[0]).clamp(0.0, 1.0));
        }
        if shape_modulation[1] != self.last_shape_modulation[1] {
            self.osc2
                .set_shape((self.params.osc2.shape_mod + shape_modulation[1]).clamp(0.0, 1.0));
        }
        self.last_shape_modulation = shape_modulation;
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::OscillatorControl);

        #[cfg(feature = "profiling")]
        let osc2_step = self.osc2.next_step_profiled(profiler);
        #[cfg(not(feature = "profiling"))]
        let osc2_step = self.osc2.next_step();

        if self.params.sync && self.params.osc2.enabled {
            self.osc1
                .hard_sync_reset(osc2_step.wrapped, osc2_step.subsample_offset);
        }
        #[cfg(feature = "profiling")]
        let osc1 = self.osc1.next_profiled(profiler);
        #[cfg(not(feature = "profiling"))]
        let osc1 = self.osc1.next();

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::OscillatorMix);
        let osc2 = osc2_step.output;
        let sub_level = (f32x4::splat(self.params.sub_octave) + modulation.sub_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let noise_level = (f32x4::splat(self.params.noise) + modulation.noise_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let sub = if sub_level == f32x4::ZERO {
            f32x4::splat(0.0)
        } else {
            self.sub_osc
                .set_frequency(self.osc1.frequency_hz(), self.sample_rate);
            self.sub_osc.next() * sub_level
        };
        let noise = if noise_level == f32x4::ZERO {
            f32x4::splat(0.0)
        } else {
            self.noise.next() * noise_level
        };
        let mix = (f32x4::splat(self.params.osc_mix) + modulation.mix)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let osc1_gain = (f32x4::splat(1.0) - mix + modulation.osc1_level)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let osc2_gain = (mix + modulation.osc2_level).clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let audio = osc1 * osc1_gain + osc2 * osc2_gain + sub + noise;

        let output = OscillatorsOutput {
            osc1,
            osc2,
            sub,
            noise,
            audio,
            audio_source_active: (self.params.osc1.enabled && osc1_gain != f32x4::ZERO)
                || (self.params.osc2.enabled && osc2_gain != f32x4::ZERO)
                || sub_level != f32x4::ZERO
                || noise_level != f32x4::ZERO,
        };
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::OscillatorMix);
        output
    }

    fn apply_params_without_frequency(&mut self) {
        self.osc1.set_enabled(self.params.osc1.enabled);
        self.osc2.set_enabled(self.params.osc2.enabled);
        self.osc1.set_waveform(self.params.osc1.waveform);
        self.osc2.set_waveform(self.params.osc2.waveform);
        apply_shape_mod(&mut self.osc1, &self.params.osc1);
        apply_shape_mod(&mut self.osc2, &self.params.osc2);
    }

    fn update_frequencies_modulated(
        &mut self,
        osc1_frequency_mod_semitones: f32x4,
        osc2_frequency_mod_semitones: f32x4,
    ) {
        self.last_frequency_modulation =
            [osc1_frequency_mod_semitones, osc2_frequency_mod_semitones];
        let osc1 = oscillator_frequency(
            f32x4::new(self.glide[0].current),
            &self.params.osc1,
            osc1_frequency_mod_semitones,
        );
        let osc2 = oscillator_frequency(
            f32x4::new(self.glide[1].current),
            &self.params.osc2,
            osc2_frequency_mod_semitones,
        );
        self.osc1.set_frequency(osc1);
        self.osc2.set_frequency(osc2);
    }
}

pub fn osc_mix_to_gains(mix: f32) -> (f32, f32) {
    let osc2 = mix.clamp(0.0, 1.0);
    (1.0 - osc2, osc2)
}

/// Per-lane modulation offsets applied before the oscillator section renders.
#[derive(Clone, Copy)]
pub struct OscillatorModulation {
    pub osc1_frequency_semitones: f32x4,
    pub osc2_frequency_semitones: f32x4,
    pub osc1_shape: f32x4,
    pub osc2_shape: f32x4,
    pub sub_level: f32x4,
    pub noise_level: f32x4,
    pub mix: f32x4,
    pub osc1_level: f32x4,
    pub osc2_level: f32x4,
}

impl Default for OscillatorModulation {
    fn default() -> Self {
        Self {
            osc1_frequency_semitones: f32x4::splat(0.0),
            osc2_frequency_semitones: f32x4::splat(0.0),
            osc1_shape: f32x4::splat(0.0),
            osc2_shape: f32x4::splat(0.0),
            sub_level: f32x4::splat(0.0),
            noise_level: f32x4::splat(0.0),
            mix: f32x4::splat(0.0),
            osc1_level: f32x4::splat(0.0),
            osc2_level: f32x4::splat(0.0),
        }
    }
}

/// Raw per-oscillator outputs and the mixed audio bus for one sample step.
pub struct OscillatorsOutput {
    pub osc1: f32x4,
    pub osc2: f32x4,
    pub sub: f32x4,
    pub noise: f32x4,
    pub audio: f32x4,
    /// True when a configured source is audibly feeding the filter input.
    pub audio_source_active: bool,
}

/// Patch-level settings for the full oscillator section.
#[derive(Debug, Clone)]
pub struct OscillatorsParams {
    pub osc1: OscillatorParams,
    pub osc2: OscillatorParams,
    pub sync: bool,
    pub osc_mix: f32,
    pub sub_octave: f32,
    pub noise: f32,
    pub osc_slop: f32,
    pub glide_mode: GlideMode,
    pub glide_enabled: bool,
}

impl Default for OscillatorsParams {
    fn default() -> Self {
        let mut osc2 = OscillatorParams::default();
        osc2.enabled = false;
        Self {
            osc1: OscillatorParams::default(),
            osc2,
            sync: false,
            osc_mix: 0.0,
            sub_octave: 0.0,
            noise: 0.0,
            osc_slop: 0.0,
            glide_mode: GlideMode::default(),
            glide_enabled: false,
        }
    }
}

/// Settings for a single analog oscillator (waveform, tuning, glide, etc.).
#[derive(Debug, Clone)]
pub struct OscillatorParams {
    pub enabled: bool,
    pub waveform: Waveform,
    pub frequency_semitones: f32,
    pub fine_tune_cents: f32,
    pub shape_mod: f32,
    pub keyboard_on: bool,
    pub note_reset: bool,
    pub glide: f32,
}

impl Default for OscillatorParams {
    fn default() -> Self {
        Self {
            enabled: true,
            waveform: Waveform::Saw,
            frequency_semitones: CENTER_FREQUENCY_SEMITONES,
            fine_tune_cents: 0.0,
            shape_mod: 0.0,
            keyboard_on: true,
            note_reset: true,
            glide: 0.0,
        }
    }
}

fn apply_shape_mod(osc: &mut EngineOscillator, params: &OscillatorParams) {
    let shape_mod = params.shape_mod.clamp(0.0, 1.0);
    osc.set_shape(shape_mod);
}

fn oscillator_frequency(
    note_semitones: f32x4,
    params: &OscillatorParams,
    mod_semitones: f32x4,
) -> f32x4 {
    let keyboard_semitones = if params.keyboard_on {
        note_semitones
    } else {
        f32x4::splat(CENTER_FREQUENCY_SEMITONES)
    };
    let semitone_offset = params.frequency_semitones - CENTER_FREQUENCY_SEMITONES;
    let scalar_semitones = semitone_offset + params.fine_tune_cents / 100.0;
    let total_semitones = keyboard_semitones + f32x4::splat(scalar_semitones) + mod_semitones;
    f32x4::splat(440.0) * ((total_semitones - f32x4::splat(69.0)) * f32x4::splat(1.0 / 12.0)).exp2()
}

fn frequency_to_semitones(frequency_hz: f32) -> f32 {
    69.0 + 12.0 * crate::math::ln(frequency_hz.max(f32::MIN_POSITIVE) / 440.0)
        / crate::math::ln(2.0)
}

fn pitch_to_q16(semitones: f32) -> i32 {
    crate::math::round(semitones * GLIDE_PITCH_SCALE) as i32
}

fn q16_to_pitch(pitch_q16: i32) -> f32 {
    pitch_q16 as f32 / GLIDE_PITCH_SCALE
}

fn clear_glide_progress(state: &mut GlideState, lane: usize) {
    state.step_q16[lane] = 0;
    state.remainder_q16[lane] = 0;
    state.error_q16[lane] = 0;
    state.total[lane] = 0;
    state.remaining[lane] = 0;
}

fn glide_seconds(amount: f32) -> f32 {
    let normalized = ((amount.clamp(MIDI_GLIDE_STEP, 1.0) - MIDI_GLIDE_STEP)
        / (1.0 - MIDI_GLIDE_STEP))
        .clamp(0.0, 1.0);
    MIN_GLIDE_SECONDS * crate::math::powf(MAX_GLIDE_SECONDS / MIN_GLIDE_SECONDS, normalized)
}

fn scalar_shape_modulation(modulation: OscillatorModulation) -> [f32; 2] {
    [
        modulation.osc1_shape.reduce_add() * 0.25,
        modulation.osc2_shape.reduce_add() * 0.25,
    ]
}

#[cfg(test)]
mod tests {
    use super::{OscillatorModulation, Oscillators, Waveform, osc_mix_to_gains};
    use crate::midi_to_hz;
    use crate::{GlideMode, f32x4};

    const SAMPLE_RATE: f32 = 44_100.0;

    fn settle(oscillators: &mut Oscillators, frames: usize) {
        for _ in 0..frames {
            oscillators.next(OscillatorModulation::default());
        }
    }

    fn configured_glide(mode: GlideMode, osc1: f32, osc2: f32) -> Oscillators {
        let mut oscillators = Oscillators::new(1_000.0);
        oscillators.set_osc1_glide(osc1);
        oscillators.set_osc2_glide(osc2);
        oscillators.set_glide_mode(mode);
        oscillators.set_glide_enabled(true);
        oscillators
    }

    #[test]
    fn fixed_time_duration_is_independent_of_interval() {
        let mut octave = configured_glide(GlideMode::FixedTime, 0.5, 0.5);
        octave.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        let octave_samples = octave.glide[0].remaining[0];

        let mut two_octaves = configured_glide(GlideMode::FixedTime, 0.5, 0.5);
        two_octaves.note_on_with_glide(0, [84.0; 4], Some(60.0), true);

        assert_eq!(two_octaves.glide[0].remaining[0], octave_samples);
    }

    #[test]
    fn fixed_rate_duration_scales_with_interval() {
        let mut octave = configured_glide(GlideMode::FixedRate, 0.5, 0.5);
        octave.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        let octave_samples = octave.glide[0].remaining[0];

        let mut two_octaves = configured_glide(GlideMode::FixedRate, 0.5, 0.5);
        two_octaves.note_on_with_glide(0, [84.0; 4], Some(60.0), true);

        let expected = octave_samples * 2;
        assert!(two_octaves.glide[0].remaining[0].abs_diff(expected) <= 1);
    }

    #[test]
    fn oscillator_glide_amounts_are_independent() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 0.25, 0.75);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);

        assert!(oscillators.glide[0].remaining[0] < oscillators.glide[1].remaining[0]);
    }

    #[test]
    fn glide_duration_is_sample_rate_independent() {
        let mut low_rate = configured_glide(GlideMode::FixedTime, 0.5, 0.5);
        low_rate.note_on_with_glide(0, [72.0; 4], Some(60.0), true);

        let mut high_rate = Oscillators::new(2_000.0);
        high_rate.set_osc1_glide(0.5);
        high_rate.set_glide_mode(GlideMode::FixedTime);
        high_rate.set_glide_enabled(true);
        high_rate.note_on_with_glide(0, [72.0; 4], Some(60.0), true);

        assert!(high_rate.glide[0].remaining[0].abs_diff(low_rate.glide[0].remaining[0] * 2) <= 1);
    }

    #[test]
    fn keyboard_tracking_off_bypasses_glide_for_that_oscillator() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 1.0, 1.0);
        oscillators.set_osc1_keyboard_on(false);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);

        assert_eq!(oscillators.glide[0].remaining[0], 0);
        assert!(oscillators.glide[1].remaining[0] > 0);
    }

    #[test]
    fn disabling_glide_snaps_an_active_transition_to_target() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 1.0, 1.0);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        settle(&mut oscillators, 10);
        assert!(oscillators.glide[0].current[0] < 72.0);

        oscillators.set_glide_enabled(false);

        assert_eq!(oscillators.glide[0].current[0], 72.0);
        assert_eq!(oscillators.glide[0].remaining[0], 0);
    }

    #[test]
    fn interrupted_glide_continues_from_current_pitch() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 1.0, 1.0);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        settle(&mut oscillators, 100);
        let before = oscillators.glide[0].current[0];

        oscillators.retune_with_glide(0, [48.0; 4], true);

        assert_eq!(oscillators.glide[0].current[0], before);
        assert!(oscillators.glide[0].step_q16[0] < 0);
    }

    #[test]
    fn longest_one_semitone_glides_make_progress_in_both_directions() {
        for target in [70.0, 68.0] {
            let mut oscillators = Oscillators::new(96_000.0);
            oscillators.set_osc1_glide(1.0);
            oscillators.set_glide_mode(GlideMode::FixedTime);
            oscillators.set_glide_enabled(true);
            oscillators.note_on_with_glide(0, [target; 4], Some(69.0), true);
            let duration = oscillators.glide[0].remaining[0];

            for _ in 0..duration / 2 {
                oscillators.advance_glide();
            }

            let halfway = oscillators.glide[0].current[0];
            let expected = (69.0 + target) * 0.5;
            assert!(
                (halfway - expected).abs() < 0.001,
                "glide toward {target} stalled or drifted: {halfway}"
            );

            for _ in duration / 2..duration {
                oscillators.advance_glide();
            }
            assert_eq!(oscillators.glide[0].current[0], target);
            assert_eq!(oscillators.glide[0].remaining[0], 0);
        }
    }

    #[test]
    fn starting_another_lane_does_not_retarget_an_active_glide() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 1.0, 1.0);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        settle(&mut oscillators, 100);
        let current = oscillators.glide[0].current[0];
        let remaining = oscillators.glide[0].remaining[0];

        oscillators.note_on_with_glide(1, [84.0, 67.0, 60.0, 60.0], Some(64.0), true);

        assert_eq!(oscillators.glide[0].target[0], 72.0);
        assert_eq!(oscillators.glide[0].current[0], current);
        assert_eq!(oscillators.glide[0].remaining[0], remaining);
        oscillators.advance_glide();
        assert!(oscillators.glide[0].current[0] > current);
    }

    #[test]
    fn tuning_updates_shift_active_glides_without_cancelling_them() {
        let mut oscillators = configured_glide(GlideMode::FixedTime, 1.0, 1.0);
        oscillators.note_on_with_glide(0, [72.0; 4], Some(60.0), true);
        settle(&mut oscillators, 100);
        let current = oscillators.glide[0].current[0];
        let remaining = oscillators.glide[0].remaining[0];

        oscillators.set_note_semitones_preserving_glide([72.25, 60.0, 60.0, 60.0]);

        assert_eq!(oscillators.glide[0].remaining[0], remaining);
        assert!((oscillators.glide[0].current[0] - (current + 0.25)).abs() < 0.000_02);
        assert_eq!(oscillators.glide[0].target[0], 72.25);
    }

    #[test]
    fn note_reset_off_advances_phase_before_first_note() {
        let frequency = f32x4::splat(midi_to_hz(60));
        let mut immediate = Oscillators::new(SAMPLE_RATE);
        let mut delayed = Oscillators::new(SAMPLE_RATE);
        immediate.set_osc1_note_reset(false);
        delayed.set_osc1_note_reset(false);

        settle(&mut delayed, 137);
        immediate.note_on(0, frequency);
        delayed.note_on(0, frequency);

        let immediate_sample = immediate
            .next(OscillatorModulation::default())
            .osc1
            .to_array()[0];
        let delayed_sample = delayed
            .next(OscillatorModulation::default())
            .osc1
            .to_array()[0];
        assert!(
            (immediate_sample - delayed_sample).abs() > 0.1,
            "a delayed first note should inherit an advanced free-running phase"
        );
    }

    #[test]
    fn disabled_osc2_keeps_advancing_when_note_reset_is_off() {
        let frequency = f32x4::splat(220.0);
        let mut audible = Oscillators::new(SAMPLE_RATE);
        let mut muted = Oscillators::new(SAMPLE_RATE);
        for oscillators in [&mut audible, &mut muted] {
            oscillators.set_osc2_enabled(true);
            oscillators.set_osc2_note_reset(false);
            oscillators.note_on(0, frequency);
        }

        muted.set_osc2_enabled(false);
        settle(&mut audible, 137);
        settle(&mut muted, 137);
        muted.set_osc2_enabled(true);

        let audible_sample = audible
            .next(OscillatorModulation::default())
            .osc2
            .to_array()[0];
        let muted_sample = muted.next(OscillatorModulation::default()).osc2.to_array()[0];
        assert!(
            (audible_sample - muted_sample).abs() < 1e-6,
            "muting oscillator 2 must not freeze its free-running phase"
        );
    }

    #[test]
    fn output_reports_whether_an_audio_source_drives_the_filter() {
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        assert!(
            oscillators
                .next(OscillatorModulation::default())
                .audio_source_active
        );

        oscillators.set_mix(1.0);
        assert!(
            !oscillators
                .next(OscillatorModulation::default())
                .audio_source_active,
            "an enabled oscillator at zero mixer gain is not an audible source"
        );
        oscillators.set_osc2_enabled(true);
        assert!(
            oscillators
                .next(OscillatorModulation::default())
                .audio_source_active
        );

        oscillators.set_osc1_enabled(false);
        oscillators.set_osc2_enabled(false);
        assert!(
            !oscillators
                .next(OscillatorModulation::default())
                .audio_source_active
        );

        oscillators.set_noise(0.1);
        assert!(
            oscillators
                .next(OscillatorModulation::default())
                .audio_source_active
        );
    }

    #[test]
    fn pulse_shape_mod_controls_pwm_from_square() {
        fn positive_ratio(shape_mod: f32) -> f32 {
            let frequency = f32x4::splat(110.0);
            let period = (SAMPLE_RATE / 110.0).round() as usize;
            let mut oscillators = Oscillators::new(SAMPLE_RATE);
            oscillators.set_osc1_waveform(Waveform::Pulse);
            oscillators.set_osc1_shape_mod(shape_mod);
            oscillators.set_note_frequency(frequency);

            let mut positive = 0usize;
            for _ in 0..period {
                let output = oscillators.next(OscillatorModulation::default());
                if output.osc1.to_array()[0] > 0.0 {
                    positive += 1;
                }
            }

            positive as f32 / period as f32
        }

        let square = positive_ratio(0.0);
        let modulated = positive_ratio(1.0);

        assert!(
            (square - 0.5).abs() < 0.05,
            "Pulse shape 0 should be a 50% square, got duty {square:.3}"
        );
        assert!(
            modulated > 0.9,
            "Pulse shape 1 should widen pulse width for PWM, got duty {modulated:.3}"
        );
    }

    #[test]
    fn osc_mix_to_gains_maps_balance_and_clamps() {
        assert_eq!(osc_mix_to_gains(0.0), (1.0, 0.0));
        assert_eq!(osc_mix_to_gains(0.25), (0.75, 0.25));
        assert_eq!(osc_mix_to_gains(1.0), (0.0, 1.0));
        assert_eq!(osc_mix_to_gains(2.0), (0.0, 1.0));
        assert_eq!(osc_mix_to_gains(-1.0), (1.0, 0.0));
    }

    #[test]
    fn default_has_osc2_disabled_and_osc1_only_mix() {
        let oscillators = Oscillators::new(SAMPLE_RATE);
        let params = oscillators.params();

        assert!(params.osc1.enabled);
        assert!(!params.osc2.enabled);
        assert_eq!(params.osc_mix, 0.0);
    }

    #[test]
    fn mix_crossfade_routes_audio_between_oscillators() {
        let frequency = f32x4::splat(440.0);
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(frequency);
        oscillators.set_sub_octave(0.0);
        oscillators.set_noise(0.0);
        settle(&mut oscillators, 64);

        oscillators.set_mix(0.0);
        let osc1_only = oscillators.next(OscillatorModulation::default());
        let osc1_audio = osc1_only.audio.to_array()[0];
        let osc1_expected = osc1_only.osc1.to_array()[0];
        assert!(
            (osc1_audio - osc1_expected).abs() < 1e-4,
            "mix 0 should pass only osc1, got {osc1_audio} vs {osc1_expected}"
        );
        assert!(
            osc1_only.osc2.to_array()[0].abs() < 1e-4,
            "default osc2 should be disabled and silent"
        );

        oscillators.set_osc2_enabled(true);
        settle(&mut oscillators, 16);

        oscillators.set_mix(1.0);
        let osc2_only = oscillators.next(OscillatorModulation::default());
        let osc2_audio = osc2_only.audio.to_array()[0];
        let osc2_expected = osc2_only.osc2.to_array()[0];
        assert!(
            (osc2_audio - osc2_expected).abs() < 1e-4,
            "mix 1 should pass only osc2, got {osc2_audio} vs {osc2_expected}"
        );

        oscillators.set_mix(0.35);
        let blended = oscillators.next(OscillatorModulation::default());
        let expected = blended.osc1.to_array()[0] * 0.65 + blended.osc2.to_array()[0] * 0.35;
        assert!(
            (blended.audio.to_array()[0] - expected).abs() < 1e-4,
            "mix 0.35 should crossfade oscillators"
        );
    }

    #[test]
    fn keyboard_off_uses_center_frequency_instead_of_note() {
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(f32x4::splat(midi_to_hz(72)));
        oscillators.set_osc1_keyboard_on(false);
        oscillators.update_frequencies();

        let freq = oscillators.osc1_frequency_hz().to_array()[0];
        let expected = midi_to_hz(60);
        assert!(
            (freq - expected).abs() < 0.1,
            "keyboard off should use center pitch, got {freq} expected {expected}"
        );
    }

    #[test]
    fn hard_sync_increases_osc1_wrap_rate_with_faster_osc2() {
        let frequency = f32x4::splat(midi_to_hz(60));
        let samples = 1_200;

        let mut without_sync = Oscillators::new(SAMPLE_RATE);
        without_sync.set_osc2_enabled(true);
        without_sync.set_note_frequency(frequency);
        without_sync.set_osc1_frequency_semitones(48.0);
        without_sync.set_osc2_frequency_semitones(84.0);
        without_sync.set_sync(false);
        let unsynced_gap = max_wrap_gap(&mut without_sync, samples);

        let mut with_sync = Oscillators::new(SAMPLE_RATE);
        with_sync.set_osc2_enabled(true);
        with_sync.set_note_frequency(frequency);
        with_sync.set_osc1_frequency_semitones(48.0);
        with_sync.set_osc2_frequency_semitones(84.0);
        with_sync.set_sync(true);
        let synced_gap = max_wrap_gap(&mut with_sync, samples);

        assert!(
            synced_gap < unsynced_gap / 2,
            "hard sync should cap osc1 wrap spacing to osc2 period, unsynced gap {unsynced_gap}, synced gap {synced_gap}"
        );
    }

    #[test]
    fn hard_sync_does_not_reset_osc1_when_osc2_disabled() {
        let frequency = f32x4::splat(midi_to_hz(60));
        let samples = 400;

        let mut reference = Oscillators::new(SAMPLE_RATE);
        reference.set_osc2_enabled(false);
        reference.set_note_frequency(frequency);
        reference.set_sync(false);
        let reference_gap = max_wrap_gap(&mut reference, samples);

        let mut with_sync = Oscillators::new(SAMPLE_RATE);
        with_sync.set_osc2_enabled(false);
        with_sync.set_note_frequency(frequency);
        with_sync.set_sync(true);
        let synced_gap = max_wrap_gap(&mut with_sync, samples);

        assert_eq!(
            synced_gap, reference_gap,
            "sync with osc2 off should not change osc1 wrap spacing"
        );
    }

    #[test]
    fn modulation_levels_affect_sub_noise_and_mix() {
        let frequency = f32x4::splat(220.0);
        let mut oscillators = Oscillators::new(SAMPLE_RATE);
        oscillators.set_note_frequency(frequency);
        oscillators.set_sub_octave(0.25);
        oscillators.set_noise(0.25);
        oscillators.set_mix(0.0);
        settle(&mut oscillators, 32);

        let base = oscillators.next(OscillatorModulation::default());
        assert!(
            base.sub.to_array()[0].abs() > 0.0,
            "sub octave level should contribute to sub output"
        );
        assert!(
            base.noise.to_array()[0].abs() > 0.0,
            "noise level should contribute to noise output"
        );

        let mut boosted = OscillatorModulation::default();
        boosted.sub_level = f32x4::splat(0.75);
        boosted.noise_level = f32x4::splat(0.75);
        let modulated = oscillators.next(boosted);

        assert!(
            modulated.sub.to_array()[0].abs() > base.sub.to_array()[0].abs(),
            "sub_level modulation should increase sub contribution"
        );
        assert!(
            modulated.noise.to_array()[0].abs() > base.noise.to_array()[0].abs(),
            "noise_level modulation should increase noise contribution"
        );

        oscillators.set_sub_octave(0.0);
        oscillators.set_noise(0.0);
        oscillators.set_osc2_enabled(true);
        settle(&mut oscillators, 16);

        let mut mix_modulation = OscillatorModulation::default();
        mix_modulation.mix = f32x4::splat(1.0);
        let osc2_only = oscillators.next(mix_modulation);
        assert!(
            (osc2_only.audio.to_array()[0] - osc2_only.osc2.to_array()[0]).abs() < 1e-4,
            "mix modulation should push output to osc2 only when sub and noise are off"
        );
    }

    #[test]
    fn note_on_resets_sub_only_when_osc1_note_reset() {
        let frequency = f32x4::splat(220.0);

        let mut reference = Oscillators::new(SAMPLE_RATE);
        reference.set_sub_octave(1.0);
        reference.set_note_frequency(frequency);
        reference.note_on(0, frequency);
        let phase_start = reference
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        let mut with_reset = Oscillators::new(SAMPLE_RATE);
        with_reset.set_sub_octave(1.0);
        with_reset.set_note_frequency(frequency);
        with_reset.set_osc1_note_reset(true);
        with_reset.note_on(0, frequency);
        settle(&mut with_reset, 300);
        with_reset.note_on(0, frequency);
        let after_reset = with_reset
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        let mut without_reset = Oscillators::new(SAMPLE_RATE);
        without_reset.set_sub_octave(1.0);
        without_reset.set_note_frequency(frequency);
        without_reset.set_osc1_note_reset(false);
        without_reset.note_on(0, frequency);
        settle(&mut without_reset, 300);
        without_reset.note_on(0, frequency);
        let after_continue = without_reset
            .next(OscillatorModulation::default())
            .sub
            .to_array()[0];

        assert!(
            (after_reset - phase_start).abs() < 0.05,
            "note reset should restart sub phase, got {after_reset} vs {phase_start}"
        );
        assert!(
            (after_continue - phase_start).abs() > 0.1,
            "without note reset sub phase should continue, got {after_continue} vs {phase_start}"
        );
    }

    #[test]
    fn runtime_shape_modulation_averages_across_lanes() {
        let frequency = f32x4::splat(110.0);
        let period = (SAMPLE_RATE / 110.0).round() as usize;

        let positive_ratio = |shape_mod: f32, shape_modulation: f32| {
            let mut oscillators = Oscillators::new(SAMPLE_RATE);
            oscillators.set_osc1_waveform(Waveform::Pulse);
            oscillators.set_osc1_shape_mod(shape_mod);
            oscillators.set_note_frequency(frequency);

            let mut modulation = OscillatorModulation::default();
            modulation.osc1_shape = f32x4::splat(shape_modulation);

            let mut positive = 0usize;
            for _ in 0..period {
                let output = oscillators.next(modulation);
                if output.osc1.to_array()[0] > 0.0 {
                    positive += 1;
                }
            }

            positive as f32 / period as f32
        };

        let param_only = positive_ratio(0.5, 0.0);
        let modulated = positive_ratio(0.0, 0.5);

        assert!(
            (param_only - modulated).abs() < 0.05,
            "shape param 0.5 and +0.5 lane modulation should match, got {param_only} vs {modulated}"
        );
    }

    fn max_wrap_gap(oscillators: &mut Oscillators, samples: usize) -> usize {
        let mut prev = 0.0f32;
        let mut gap = 0usize;
        let mut max_gap = 0usize;
        for _ in 0..samples {
            let sample = oscillators
                .next(OscillatorModulation::default())
                .osc1
                .to_array()[0];
            // Detect the falling zero crossing rather than assuming that a
            // band-limited edge completes within one sample.
            if prev > 0.0 && sample <= 0.0 && sample < prev - 0.1 {
                max_gap = max_gap.max(gap);
                gap = 0;
            } else {
                gap += 1;
            }
            prev = sample;
        }
        max_gap
    }
}
