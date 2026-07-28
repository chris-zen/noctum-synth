//! Voice layer: polyphony manager and per-block signal chain.

mod amplifier;
mod aux_env;
mod filter;
mod lanes;
mod lfo;
mod manager;
mod modulation;
mod oscillators;
mod pan;

pub use manager::{ActiveNotes, VoiceManager, unison_detune_cents};

use crate::dsp::{
    DEFAULT_PARAMETER_SMOOTHING_SECONDS, DcBlocker, FilterOversampling, FilterType, LfoWaveform,
    ParameterSmoother, Waveform,
};
use crate::effects::EffectModulation;
use crate::math::{F32, WideF32};
#[cfg(test)]
use crate::patch::DedicatedModSource;
use crate::patch::{ClockDivision, LFO_COUNT, LfoSyncDivision, ModDestination, PanModMode, Patch};
use crate::profiling::{RenderContext, RenderStage};
use crate::{DEFAULT_TEMPO_BPM, GlideMode, ModSource, ParamId, VOICE_COUNT};
use amplifier::Amplifier;
use aux_env::AuxEnv;
use filter::Filter;
use lanes::Lanes;
use lfo::Lfo;
use modulation::ModSignalContext;
use pan::Pan;

pub use modulation::PatchModulation;
pub use oscillators::{
    OscillatorModulation, OscillatorParams, Oscillators, OscillatorsOutput, OscillatorsParams,
    glide_seconds,
};

const LFO_PITCH_DEPTH_SEMITONES: f32 = 12.0;
const LFO_CUTOFF_DEPTH_SEMITONES: f32 = 127.0;
/// Short smooth release used before replacing an audible voice (SynthLab precedent).
const VOICE_STEAL_SHUTDOWN_SECONDS: f32 = 0.005;

/// Provisional Rev2-16 physical-voice pan pattern.
///
/// Sequential documents a deterministic alternating pattern whose voices move
/// progressively toward center. These coefficients are isolated so measured
/// hardware values can replace the estimate without changing pan semantics.
pub const REV2_VOICE_PAN_POSITIONS: [f32; VOICE_COUNT] = [
    1.0, -1.0, 0.875, -0.875, 0.75, -0.75, 0.625, -0.625, 0.5, -0.5, 0.375, -0.375, 0.25, -0.25,
    0.125, -0.125,
];

/// Return the deterministic spread coefficient for one physical voice.
///
/// Voices alternate right/left. Each pair moves one equal step toward center,
/// with the step size derived from the available polyphony. `voice_count` is
/// expected to be a non-zero even number, as all engine configurations contain
/// four-lane voice blocks.
pub const fn voice_pan_position(voice_index: usize, voice_count: usize) -> f32 {
    if voice_count == 0 {
        return 0.0;
    }
    let pair_count = voice_count.div_ceil(2);
    let wrapped_index = voice_index % voice_count;
    let pair_index = wrapped_index / 2;
    let magnitude = (pair_count - pair_index) as f32 / pair_count as f32;
    if wrapped_index.is_multiple_of(2) {
        magnitude
    } else {
        -magnitude
    }
}

#[derive(Clone, Copy)]
pub struct PerformanceModulation {
    pub pitch_bend: f32,
    pub mod_wheel: f32,
    pub pressure: f32,
    pub breath: f32,
    pub foot: f32,
    pub expression: f32,
}

impl Default for PerformanceModulation {
    fn default() -> Self {
        Self {
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            pressure: 0.0,
            breath: 0.0,
            foot: 0.0,
            expression: 0.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct NoteGlide {
    pub start_note: Option<u8>,
    pub enabled: bool,
}

/// How an inactive [`VoiceBlock`] should advance for one sample.
///
/// The manager chooses this from allocation foresight: which silent block will
/// likely receive the next note. The block itself does not decide.
///
/// In hardware the oscillator, filter, and DC-coupling stages keep running while
/// the VCA gate is closed. Digitally we only pay that cost for blocks that are
/// about to speak; everyone else only keeps free-running LFOs in phase.
#[derive(Clone, Copy)]
pub(crate) enum IdleAdvance {
    /// No note is expected soon: advance LFOs (and their control routing) only.
    ///
    /// Skips oscillators, filter, and DC blocker. Used for silent packs that are
    /// not the next polyphonic steal target and are outside the unison warm set.
    Cold,
    /// A note is expected soon: run the full pre-VCA chain so state is settled.
    ///
    /// Needed so wide-pulse / asymmetric waveforms do not click when the gate
    /// opens: the DC blocker and filter need time at the upcoming waveform
    /// before the amp becomes audible. Used for the next poly allocation's pack
    /// and for the leading packs in a unison group.
    Warm,
}

/// Four-lane subtractive voice: oscillators → filter → amplifier.
///
/// Each lane can represent a separate note. Envelopes, LFOs, and modulation are
/// evaluated per lane each sample step.
pub struct VoiceBlock {
    lanes: Lanes,
    oscillators: Oscillators,
    filter: Filter,
    amplifier: Amplifier,
    dc_blocker: DcBlocker,
    aux: AuxEnv,
    aux_amount: ParameterSmoother,
    pan: Pan,
    lfos: [Lfo; LFO_COUNT],
    last_effect_modulation: EffectModulation,
    pitch_bend_range: f32,
    tempo_bpm: f32,
    clock_division: ClockDivision,

    sample_rate: f32,
}

impl VoiceBlock {
    pub fn new(sample_rate: f32) -> Self {
        let block = Self {
            lanes: Lanes::new(sample_rate),
            oscillators: Oscillators::new(sample_rate),
            filter: Filter::new(sample_rate),
            amplifier: Amplifier::new(sample_rate),
            dc_blocker: DcBlocker::new(sample_rate),
            aux: AuxEnv::new(sample_rate),
            aux_amount: ParameterSmoother::new(
                0.0,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
            pan: Pan::new(),
            lfos: core::array::from_fn(|_| Lfo::new(sample_rate)),
            last_effect_modulation: EffectModulation::default(),
            pitch_bend_range: 0.0,
            tempo_bpm: DEFAULT_TEMPO_BPM,
            clock_division: ClockDivision::default(),
            sample_rate,
        };
        block
    }

    pub(crate) fn refresh_lfo_engines(&mut self) {
        for lfo in &mut self.lfos {
            let depth = lfo.base_depth();
            lfo.refresh_engine_rate(self.tempo_bpm, self.clock_division);
            lfo.apply_engine_depth(depth);
        }
    }

    fn lifecycle_shutdown_samples(&self) -> u32 {
        (F32(self.sample_rate * VOICE_STEAL_SHUTDOWN_SECONDS)
            .round()
            .as_f32() as u32)
            .max(1)
    }

    pub fn note_on(&mut self, lane: usize, note: u8, velocity: f32, reset_key_synced_lfos: bool) {
        self.note_on_tuned(
            lane,
            note,
            velocity,
            reset_key_synced_lfos,
            [0.0; WideF32::LANES],
        );
    }

    pub(crate) fn note_on_tuned(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        reset_key_synced_lfos: bool,
        tuning_cents: [f32; WideF32::LANES],
    ) {
        self.note_on_tuned_with_glide(
            lane,
            note,
            velocity,
            reset_key_synced_lfos,
            tuning_cents,
            NoteGlide::default(),
        );
    }

    pub(crate) fn note_on_tuned_with_glide(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        reset_key_synced_lfos: bool,
        tuning_cents: [f32; WideF32::LANES],
        glide: NoteGlide,
    ) {
        self.lanes.activate_lifecycle_lane(
            lane,
            self.amplifier.initial_level(),
            self.lifecycle_shutdown_samples(),
        );
        self.lanes
            .begin_note_on(lane, note, velocity, tuning_cents[lane]);
        self.amplifier.trigger_lane(lane);
        self.filter.trigger_lane(lane);
        self.aux.trigger_lane(lane);

        if reset_key_synced_lfos {
            self.reset_key_synced_lfos();
        }
        let semitones = self.lanes.note_semitones();
        let start = glide
            .start_note
            .map(|start| f32::from(start) + tuning_cents[lane] / 100.0);
        self.oscillators
            .note_on_with_glide(lane, semitones, start, glide.enabled);
    }

    pub(crate) fn retrigger_sounding_with_glide(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        tuning_cents: [f32; WideF32::LANES],
        should_glide: bool,
    ) {
        self.lanes.activate_lifecycle_lane(
            lane,
            self.amplifier.initial_level(),
            self.lifecycle_shutdown_samples(),
        );
        self.lanes
            .begin_note_on(lane, note, velocity, tuning_cents[lane]);
        self.amplifier.trigger_lane(lane);
        self.filter.trigger_lane(lane);
        self.aux.trigger_lane(lane);
        self.oscillators
            .retune_with_glide(lane, self.lanes.note_semitones(), should_glide);
    }

    /// Changes a sounding lane's pitch and velocity without retriggering its DSP state.
    pub(crate) fn retune_lane(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        tuning_cents: [f32; WideF32::LANES],
        should_glide: bool,
    ) {
        if self
            .lanes
            .update_pending_lane(lane, note, velocity, tuning_cents[lane], should_glide)
        {
            return;
        }
        self.lanes
            .update_sounding_lane(lane, note, velocity, tuning_cents[lane]);
        self.oscillators
            .retune_with_glide(lane, self.lanes.note_semitones(), should_glide);
    }

    pub(crate) fn set_tuning_cents(&mut self, tuning_cents: [f32; WideF32::LANES]) {
        self.lanes.set_tuning_cents_array(tuning_cents);
        self.oscillators
            .set_note_semitones_preserving_glide(self.lanes.note_semitones());
    }

    #[cfg(test)]
    pub(crate) fn schedule_note_on(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        reset_key_synced_lfos: bool,
    ) {
        self.schedule_note_on_tuned_with_glide(
            lane,
            note,
            velocity,
            reset_key_synced_lfos,
            [0.0; WideF32::LANES],
            NoteGlide::default(),
        );
    }

    pub(crate) fn schedule_note_on_tuned_with_glide(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        reset_key_synced_lfos: bool,
        tuning_cents: [f32; WideF32::LANES],
        glide: NoteGlide,
    ) {
        let shutdown_in_progress = self.lanes.has_pending(lane);
        self.lanes.set_pending(
            lane,
            lanes::PendingNote {
                note,
                velocity,
                reset_key_synced_lfos,
                glide,
            },
            tuning_cents[lane],
        );
        if shutdown_in_progress {
            return;
        }
        self.lanes.release_gate(lane);
        self.lanes
            .fade_out_lifecycle_lane(lane, self.lifecycle_shutdown_samples());
        self.amplifier
            .shutdown_lane(lane, VOICE_STEAL_SHUTDOWN_SECONDS);
        self.filter.release_lane(lane);
        self.aux.release_lane(lane);
    }

    pub fn note_off(&mut self, note: u8) {
        for lane in 0..WideF32::LANES {
            if self.active_note(lane) == Some(note) {
                self.note_off_lane(lane);
            }
        }
    }

    pub fn note_off_lane(&mut self, lane: usize) {
        self.lanes.clear_pending(lane);
        self.lanes.release_gate(lane);
        self.amplifier.release_lane(lane);
        self.filter.release_lane(lane);
        self.aux.release_lane(lane);
    }

    pub fn all_notes_off(&mut self) {
        self.lanes.clear_all_pending();
        self.lanes.release_all_gates();
        self.amplifier.release_all();
        self.filter.release_all();
        self.aux.release_all();
    }

    pub fn next(
        &mut self,
        performance: PerformanceModulation,
        modulation: &PatchModulation,
        ctx: &mut RenderContext<'_>,
    ) -> (f32, f32) {
        self.lanes.advance_ages();
        self.start_pending_notes();
        self.lanes.smooth_velocities();
        crate::profiler_begin!(ctx, RenderStage::EnvelopesAndModulation);
        crate::profiler_begin!(ctx, RenderStage::EnvelopeAdvance);
        let velocities = self.lanes.velocities();
        self.aux_amount.set_target(modulation.aux_amount());
        self.aux_amount.next();
        self.aux.advance_smoothers();
        let (aux_env, aux_signal) = self.aux.next_signal(velocities, self.aux_amount.value());
        let filter_env = self.filter.next_envelope();
        let amp = self.amplifier.next_envelope();
        let lifecycle_gain = self.lanes.next_lifecycle_gain();
        crate::profiler_end!(ctx, RenderStage::EnvelopeAdvance);

        let context = ModSignalContext {
            performance,
            velocities,
            filter_env,
            amp_env: amp,
            aux_env,
            aux_signal,
        };
        let pitch_bend = WideF32::splat(performance.pitch_bend * self.pitch_bend_range);
        let mut lfo_modulation = LfoModulation::default();
        lfo_modulation.oscillators.osc1_frequency_semitones = pitch_bend;
        lfo_modulation.oscillators.osc2_frequency_semitones = pitch_bend;
        let plan = modulation.plan();
        if plan.any_modulation {
            crate::profiler_begin!(ctx, RenderStage::LfoControlRouting);
            let lfo_control = if plan.control_count == 0 {
                LfoControlModulation::default()
            } else {
                self.evaluate_lfo_control_routes(modulation, context)
            };
            crate::profiler_end!(ctx, RenderStage::LfoControlRouting);

            crate::profiler_begin!(ctx, RenderStage::LfoGeneration);
            self.advance_lfos(lfo_control, modulation);
            crate::profiler_end!(ctx, RenderStage::LfoGeneration);

            crate::profiler_begin!(ctx, RenderStage::AudioModulationRouting);
            if let Some(route) = plan.single_pwm_route {
                lfo_modulation.osc1_shape =
                    self.lfos[route.lfo_index as usize].output().reduce_mean() * route.amount;
            } else if let Some(route) = plan.single_filter_cutoff_route {
                let index = route.lfo_index as usize;
                let scale = route.amount * LFO_CUTOFF_DEPTH_SEMITONES;
                if self.lfos[index].output_is_uniform() {
                    lfo_modulation
                        .filter_cutoff
                        .set_uniform(self.lfos[index].output().to_array()[0] * scale);
                } else {
                    lfo_modulation
                        .filter_cutoff
                        .add(self.lfos[index].output() * WideF32::splat(scale));
                }
            } else {
                self.apply_audio_modulation_routes(modulation, &mut lfo_modulation, context);
            }
            crate::profiler_end!(ctx, RenderStage::AudioModulationRouting);
        }
        self.last_effect_modulation = lfo_modulation.effects;
        crate::profiler_end!(ctx, RenderStage::EnvelopesAndModulation);

        crate::profiler_begin!(ctx, RenderStage::Oscillators);
        let osc = self.oscillators.next(
            lfo_modulation.oscillators,
            [lfo_modulation.osc1_shape, lfo_modulation.osc2_shape],
            ctx,
        );
        let mix = osc.audio;
        self.filter
            .set_self_oscillation_color_enabled(!osc.audio_source_active);
        crate::profiler_end!(ctx, RenderStage::Oscillators);

        crate::profiler_begin!(ctx, RenderStage::Filter);
        let notes = self.oscillators.current_keyboard_semitones();
        let filtered = self.filter.process_prepared(
            mix,
            notes,
            filter_env,
            velocities,
            osc.osc1,
            lfo_modulation.filter_cutoff.lanes,
            lfo_modulation.filter_cutoff.uniform,
            lfo_modulation.filter_resonance,
            lfo_modulation.filter_audio_mod,
            self.sample_rate,
        );
        crate::profiler_end!(ctx, RenderStage::Filter);

        crate::profiler_begin!(ctx, RenderStage::AmplifierAndPan);
        self.amplifier.advance_smoothers();
        let amp_lfo_gain = (WideF32::splat(1.0) + lfo_modulation.amp_gain)
            .clamp(WideF32::ZERO, WideF32::splat(2.0));
        let output = self.dc_blocker.process(filtered)
            * self.amplifier.gain(amp, velocities, amp_lfo_gain)
            * lifecycle_gain;

        let stereo = self
            .pan
            .pan_lanes(output, lfo_modulation.pan, self.lanes.pan_positions());
        crate::profiler_end!(ctx, RenderStage::AmplifierAndPan);
        stereo
    }

    fn evaluate_lfo_control_routes(
        &self,
        modulation: &PatchModulation,
        context: ModSignalContext,
    ) -> LfoControlModulation {
        let mut lfo_control = LfoControlModulation::default();
        for route in modulation.plan().control_routes() {
            let signal = route.signal(self, context);
            lfo_control.apply(route.destination(), signal.reduce_mean());
        }
        lfo_control
    }

    fn advance_lfos(&mut self, lfo_control: LfoControlModulation, modulation: &PatchModulation) {
        let plan = modulation.plan();
        let tempo_bpm = self.tempo_bpm;
        let clock_division = self.clock_division;
        let base_rates = lfo::base_rates(&self.lfos);
        let base_depths = lfo::base_depths(&self.lfos);

        if plan.rate_target_mask == 0 && plan.depth_target_mask == 0 {
            for (index, lfo) in self.lfos.iter_mut().enumerate() {
                if plan.active_lfo_mask & (1 << index) != 0 {
                    lfo.generate();
                } else {
                    lfo.advance_idle();
                }
            }
            return;
        }

        let rates = if lfo_control.rate_mod == [0.0; LFO_COUNT] {
            base_rates
        } else {
            core::array::from_fn(|i| {
                base_rates[i] * F32(lfo_control.rate_mod[i] * 4.0).exp2().as_f32()
            })
        };
        let depths = if lfo_control.depth_mod == [0.0; LFO_COUNT] {
            base_depths
        } else {
            core::array::from_fn(|i| (base_depths[i] + lfo_control.depth_mod[i]).clamp(0.0, 1.0))
        };

        for (index, lfo) in self.lfos.iter_mut().enumerate() {
            let bit = 1 << index;
            if plan.rate_target_mask & bit != 0 {
                let rate_hz = if lfo.clock_sync() {
                    lfo.effective_rate_hz(tempo_bpm, clock_division)
                } else {
                    rates[index]
                };
                lfo.apply_engine_rate(rate_hz);
            }
            if plan.depth_target_mask & bit != 0 {
                lfo.apply_engine_depth(depths[index]);
            }
            if plan.active_lfo_mask & bit != 0 {
                lfo.generate();
            } else {
                lfo.advance_idle();
            }
        }
    }

    pub(crate) fn reset_key_synced_lfos(&mut self) {
        for lfo in &mut self.lfos {
            lfo.reset_if_key_synced();
        }
    }

    pub(crate) fn advance_idle(
        &mut self,
        mode: IdleAdvance,
        performance: PerformanceModulation,
        modulation: &PatchModulation,
        ctx: &mut RenderContext<'_>,
    ) {
        match mode {
            IdleAdvance::Cold => {
                if !modulation.plan().any_modulation {
                    return;
                }
                let context = ModSignalContext {
                    performance,
                    velocities: WideF32::ZERO,
                    filter_env: WideF32::ZERO,
                    amp_env: WideF32::ZERO,
                    aux_env: WideF32::ZERO,
                    aux_signal: WideF32::ZERO,
                };
                let lfo_control = if modulation.plan().control_count == 0 {
                    LfoControlModulation::default()
                } else {
                    self.evaluate_lfo_control_routes(modulation, context)
                };
                self.advance_lfos(lfo_control, modulation);
            }
            IdleAdvance::Warm => {
                let _ = self.next(performance, modulation, ctx);
            }
        }
    }

    #[cfg(test)]
    fn pan_lanes(&self, lanes: WideF32, pan_mod: WideF32) -> (f32, f32) {
        self.pan
            .pan_lanes(lanes, pan_mod, self.lanes.pan_positions())
    }

    pub fn is_lane_silent(&self, lane: usize) -> bool {
        let biased_lane_is_audible =
            self.amplifier.initial_level() > 0.0 && self.lanes.lifecycle_gain(lane) > 0.0;
        self.lanes.pending(lane).is_none()
            && !self.lanes.gate(lane)
            && self.amplifier.is_idle_lane(lane)
            && !biased_lane_is_audible
    }

    pub fn is_lane_released(&self, lane: usize) -> bool {
        self.lanes.pending(lane).is_none() && !self.lanes.gate(lane)
    }

    pub fn for_each_active_note(&self, f: impl FnMut(u8)) {
        self.lanes.for_each_active_note(f);
    }

    pub(crate) fn active_note(&self, lane: usize) -> Option<u8> {
        self.lanes.active_note(lane)
    }

    pub(crate) fn has_pending_note(&self, lane: usize) -> bool {
        self.lanes.has_pending(lane)
    }

    fn start_pending_notes(&mut self) {
        if self.lanes.pending_mask() == 0 {
            return;
        }
        for lane in 0..WideF32::LANES {
            if self.lanes.pending_mask() & (1 << lane) == 0 {
                continue;
            }
            if !self.amplifier.is_idle_lane(lane) {
                continue;
            }
            if self.lanes.lifecycle_fade_remaining(lane) != 0
                || self.lanes.lifecycle_gain(lane) > 0.0
            {
                continue;
            }
            let Some(pending) = self.lanes.take_pending(lane) else {
                continue;
            };

            self.amplifier.reset_lane(lane);
            self.filter.reset_envelope_lane(lane);
            self.aux.reset_lane(lane);
            self.note_on_tuned_with_glide(
                lane,
                pending.note,
                pending.velocity,
                pending.reset_key_synced_lfos,
                self.lanes.tuning_cents_array(),
                pending.glide,
            );
        }
    }

    pub fn active_lane_count(&self) -> usize {
        (0..WideF32::LANES)
            .filter(|&lane| !self.is_lane_silent(lane))
            .count()
    }

    pub fn oldest_lane(&self) -> usize {
        (0..WideF32::LANES)
            .max_by_key(|&lane| self.lanes.age(lane))
            .unwrap_or(0)
    }

    pub fn set_osc1_note_param(&mut self, note_param: f32) {
        self.oscillators.set_osc1_frequency_semitones(note_param);
    }

    pub fn set_osc2_note_param(&mut self, note_param: f32) {
        self.oscillators.set_osc2_frequency_semitones(note_param);
    }

    pub fn set_osc1_fine(&mut self, cents: f32) {
        self.oscillators.set_osc1_fine_tune_cents(cents);
    }

    pub fn set_osc2_fine(&mut self, cents: f32) {
        self.oscillators.set_osc2_fine_tune_cents(cents);
    }

    pub fn set_osc1_waveform(&mut self, waveform: Waveform) {
        self.oscillators.set_osc1_waveform(waveform);
    }

    pub fn set_osc2_waveform(&mut self, waveform: Waveform) {
        self.oscillators.set_osc2_waveform(waveform);
    }

    pub fn set_osc1_enabled(&mut self, enabled: bool) {
        self.oscillators.set_osc1_enabled(enabled);
    }

    pub fn set_osc2_enabled(&mut self, enabled: bool) {
        self.oscillators.set_osc2_enabled(enabled);
    }

    pub fn set_osc1_shape_mod(&mut self, shape_mod: f32) {
        self.oscillators.set_osc1_shape_mod(shape_mod);
    }

    pub fn set_osc2_shape_mod(&mut self, shape_mod: f32) {
        self.oscillators.set_osc2_shape_mod(shape_mod);
    }

    pub fn set_osc1_note_reset(&mut self, note_reset: bool) {
        self.oscillators.set_osc1_note_reset(note_reset);
    }

    pub fn set_osc2_note_reset(&mut self, note_reset: bool) {
        self.oscillators.set_osc2_note_reset(note_reset);
    }

    pub fn set_osc1_keyboard_on(&mut self, keyboard_on: bool) {
        self.oscillators.set_osc1_keyboard_on(keyboard_on);
    }

    pub fn set_osc2_keyboard_on(&mut self, keyboard_on: bool) {
        self.oscillators.set_osc2_keyboard_on(keyboard_on);
    }

    pub fn set_osc1_glide(&mut self, amount: f32) {
        self.oscillators.set_osc1_glide(amount);
    }

    pub fn set_osc2_glide(&mut self, amount: f32) {
        self.oscillators.set_osc2_glide(amount);
    }

    pub fn set_glide_mode(&mut self, mode: GlideMode) {
        self.oscillators.set_glide_mode(mode);
    }

    pub fn set_glide_enabled(&mut self, enabled: bool) {
        self.oscillators.set_glide_enabled(enabled);
    }

    pub fn set_osc_mix(&mut self, mix: f32) {
        self.oscillators.set_mix(mix);
    }

    pub fn set_sub_osc_level(&mut self, level: f32) {
        self.oscillators.set_sub_octave(level);
    }

    pub fn set_noise_level(&mut self, level: f32) {
        self.oscillators.set_noise(level);
    }

    pub fn set_hard_sync(&mut self, sync: bool) {
        self.oscillators.set_sync(sync);
    }

    pub fn set_osc_slop(&mut self, slop: f32) {
        self.oscillators.set_slop(slop);
    }

    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.filter.set_oversampling(oversampling);
    }

    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.filter.set_filter_type(filter_type);
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter.set_cutoff(cutoff);
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.filter.set_resonance(resonance);
    }

    pub fn set_filter_poles(&mut self, poles: u8) {
        self.filter.set_poles(poles);
    }

    pub fn set_filter_key_track(&mut self, key_track: f32) {
        self.filter.set_key_track(key_track);
    }

    pub fn set_filter_env_amount(&mut self, env_amount: f32) {
        self.filter.set_env_amount(env_amount);
    }

    pub fn set_filter_velocity(&mut self, velocity: f32) {
        self.filter.set_env_velocity_amount(velocity);
    }

    pub fn set_filter_audio_mod(&mut self, audio_mod: f32) {
        self.filter.set_audio_mod(audio_mod);
    }

    pub fn set_filter_delay(&mut self, seconds: f32) {
        self.filter.set_delay_seconds(seconds);
    }

    pub fn set_filter_attack(&mut self, seconds: f32) {
        self.filter.set_attack_seconds(seconds);
    }

    pub fn set_filter_decay(&mut self, seconds: f32) {
        self.filter.set_decay_seconds(seconds);
    }

    pub fn set_filter_sustain(&mut self, sustain: f32) {
        self.filter.set_sustain_level(sustain);
    }

    pub fn set_filter_release(&mut self, seconds: f32) {
        self.filter.set_release_seconds(seconds);
    }

    pub fn set_amp_attack(&mut self, seconds: f32) {
        self.amplifier.set_attack_seconds(seconds);
    }

    pub fn set_amp_delay(&mut self, seconds: f32) {
        self.amplifier.set_delay_seconds(seconds);
    }

    pub fn set_amp_decay(&mut self, seconds: f32) {
        self.amplifier.set_decay_seconds(seconds);
    }

    pub fn set_amp_sustain(&mut self, sustain: f32) {
        self.amplifier.set_sustain_level(sustain);
    }

    pub fn set_amp_release(&mut self, seconds: f32) {
        self.amplifier.set_release_seconds(seconds);
    }

    pub fn set_vca_initial_level(&mut self, level: f32) {
        self.amplifier.set_initial_level(level);
    }

    pub fn set_amp_env_amount(&mut self, amount: f32) {
        self.amplifier.set_env_amount(amount);
    }

    pub fn set_amp_velocity_amount(&mut self, amount: f32) {
        self.amplifier.set_velocity_amount(amount);
    }

    pub fn set_pan_spread(&mut self, spread: f32) {
        self.pan.set_spread(spread);
    }

    pub fn set_pan_mod_mode(&mut self, mode: PanModMode) {
        self.pan.set_mod_mode(mode);
    }

    pub fn set_aux_velocity_amount(&mut self, amount: f32) {
        self.aux.set_velocity_amount(amount);
    }

    pub fn set_aux_delay(&mut self, seconds: f32) {
        self.aux.set_delay_seconds(seconds);
    }

    pub fn set_aux_attack(&mut self, seconds: f32) {
        self.aux.set_attack_seconds(seconds);
    }

    pub fn set_aux_decay(&mut self, seconds: f32) {
        self.aux.set_decay_seconds(seconds);
    }

    pub fn set_aux_sustain(&mut self, sustain: f32) {
        self.aux.set_sustain_level(sustain);
    }

    pub fn set_aux_release(&mut self, seconds: f32) {
        self.aux.set_release_seconds(seconds);
    }

    pub fn set_aux_repeat(&mut self, repeat: bool) {
        self.aux.set_repeat(repeat);
    }

    pub fn set_pitch_bend_range(&mut self, semitones: f32) {
        self.pitch_bend_range = semitones.clamp(0.0, 12.0);
    }

    pub(crate) fn set_pan_positions(&mut self, positions: [f32; WideF32::LANES]) {
        self.lanes.set_pan_positions(positions);
    }

    pub fn set_lfo_rate_hz(&mut self, index: usize, rate_hz: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_base_rate_hz(rate_hz);
            if !lfo.clock_sync() {
                lfo.refresh_engine_rate(self.tempo_bpm, self.clock_division);
            }
        }
    }

    pub fn set_lfo_depth(&mut self, index: usize, depth: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_depth(depth);
        }
    }

    pub fn set_lfo_waveform(&mut self, index: usize, waveform: LfoWaveform) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_waveform(waveform);
        }
    }

    pub fn set_lfo_destination(&mut self, index: usize, destination: ModDestination) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_destination(destination);
        }
    }

    pub fn set_lfo_clock_sync(&mut self, index: usize, clock_sync: bool) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_clock_sync(clock_sync);
            self.refresh_lfo_rate(index);
        }
    }

    pub fn set_lfo_sync_division(&mut self, index: usize, division: LfoSyncDivision) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_sync_division(division);
            if lfo.clock_sync() {
                self.refresh_lfo_rate(index);
            }
        }
    }

    pub fn set_tempo_bpm(&mut self, bpm: f32) {
        self.tempo_bpm = bpm.clamp(30.0, 250.0);
        self.refresh_synced_lfo_rates();
    }

    pub fn set_clock_division(&mut self, division: ClockDivision) {
        self.clock_division = division;
        self.refresh_synced_lfo_rates();
    }

    fn refresh_lfo_rate(&mut self, index: usize) {
        if index < self.lfos.len() {
            self.lfos[index].refresh_engine_rate(self.tempo_bpm, self.clock_division);
        }
    }

    fn refresh_synced_lfo_rates(&mut self) {
        for index in 0..self.lfos.len() {
            if self.lfos[index].clock_sync() {
                self.refresh_lfo_rate(index);
            }
        }
    }

    pub fn set_lfo_key_sync(&mut self, index: usize, key_sync: bool) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_key_sync(key_sync);
        }
    }

    fn apply_audio_modulation_routes(
        &self,
        patch_modulation: &PatchModulation,
        modulation: &mut LfoModulation,
        context: ModSignalContext,
    ) {
        for route in patch_modulation.plan().audio_routes() {
            modulation.apply_destination(route.destination(), route.signal(self, context));
        }
    }

    pub(crate) fn apply_voice_patch(&mut self, patch: &Patch) {
        self.oscillators.apply_params(patch);
        self.filter.apply_params(&patch.filter);
        self.amplifier.apply_params(&patch.amplifier);
        self.pan
            .apply_params(patch.amplifier.pan_spread, patch.amplifier.pan_mod_mode);
        self.aux.apply_params(&patch.aux_envelope);
        self.aux_amount.snap(patch.aux_envelope.amount);
        for (index, params) in patch.lfos.iter().enumerate() {
            self.lfos[index].apply_params(params, self.tempo_bpm, self.clock_division);
        }
        patch.for_each_param(|id, value| {
            if is_section_patch_param(id) {
                return;
            }
            self.set_param(id, value);
        });
    }

    pub(crate) fn take_effect_modulation(&mut self) -> EffectModulation {
        core::mem::take(&mut self.last_effect_modulation)
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) {
        if self.oscillators.set_param(id, value)
            || self.filter.set_param(id, value)
            || self.amplifier.set_param(id, value)
            || self.aux.set_param(id, value)
            || self.pan.set_param(id, value)
        {
            return;
        }
        match id {
            ParamId::AuxEgDestination | ParamId::AuxEgAmount => {}
            ParamId::Lfo1Rate => self.set_lfo_rate_hz(0, value),
            ParamId::Lfo2Rate => self.set_lfo_rate_hz(1, value),
            ParamId::Lfo3Rate => self.set_lfo_rate_hz(2, value),
            ParamId::Lfo4Rate => self.set_lfo_rate_hz(3, value),
            ParamId::Lfo1Depth => self.set_lfo_depth(0, value),
            ParamId::Lfo2Depth => self.set_lfo_depth(1, value),
            ParamId::Lfo3Depth => self.set_lfo_depth(2, value),
            ParamId::Lfo4Depth => self.set_lfo_depth(3, value),
            ParamId::Lfo1Waveform => {
                self.set_lfo_waveform(0, LfoWaveform::from_index(value as usize))
            }
            ParamId::Lfo2Waveform => {
                self.set_lfo_waveform(1, LfoWaveform::from_index(value as usize))
            }
            ParamId::Lfo3Waveform => {
                self.set_lfo_waveform(2, LfoWaveform::from_index(value as usize))
            }
            ParamId::Lfo4Waveform => {
                self.set_lfo_waveform(3, LfoWaveform::from_index(value as usize))
            }
            ParamId::Lfo1Destination => {
                self.set_lfo_destination(0, ModDestination::from_index(value as usize));
            }
            ParamId::Lfo2Destination => {
                self.set_lfo_destination(1, ModDestination::from_index(value as usize));
            }
            ParamId::Lfo3Destination => {
                self.set_lfo_destination(2, ModDestination::from_index(value as usize));
            }
            ParamId::Lfo4Destination => {
                self.set_lfo_destination(3, ModDestination::from_index(value as usize));
            }
            ParamId::Lfo1ClockSync => self.set_lfo_clock_sync(0, value >= 0.5),
            ParamId::Lfo2ClockSync => self.set_lfo_clock_sync(1, value >= 0.5),
            ParamId::Lfo3ClockSync => self.set_lfo_clock_sync(2, value >= 0.5),
            ParamId::Lfo4ClockSync => self.set_lfo_clock_sync(3, value >= 0.5),
            ParamId::Lfo1SyncDivision => {
                self.set_lfo_sync_division(0, LfoSyncDivision::from_index(value as usize))
            }
            ParamId::Lfo2SyncDivision => {
                self.set_lfo_sync_division(1, LfoSyncDivision::from_index(value as usize))
            }
            ParamId::Lfo3SyncDivision => {
                self.set_lfo_sync_division(2, LfoSyncDivision::from_index(value as usize))
            }
            ParamId::Lfo4SyncDivision => {
                self.set_lfo_sync_division(3, LfoSyncDivision::from_index(value as usize))
            }
            ParamId::Lfo1KeySync => self.set_lfo_key_sync(0, value >= 0.5),
            ParamId::Lfo2KeySync => self.set_lfo_key_sync(1, value >= 0.5),
            ParamId::Lfo3KeySync => self.set_lfo_key_sync(2, value >= 0.5),
            ParamId::Lfo4KeySync => self.set_lfo_key_sync(3, value >= 0.5),
            ParamId::PitchBendRange => self.set_pitch_bend_range(value),
            ParamId::MasterVolume
            | ParamId::KeyMode
            | ParamId::UnisonEnabled
            | ParamId::UnisonMode
            | ParamId::UnisonDetune => {}
            ParamId::Bpm => self.set_tempo_bpm(value),
            ParamId::ClockDivide => {
                self.set_clock_division(ClockDivision::from_index(value as usize))
            }
            _ => {}
        }
    }

    pub(crate) fn mod_source_signal(
        &self,
        source: ModSource,
        context: ModSignalContext,
    ) -> WideF32 {
        match source {
            ModSource::Off
            | ModSource::Seq1
            | ModSource::Seq2
            | ModSource::Seq3
            | ModSource::Seq4
            | ModSource::Noise
            | ModSource::AudioOut => WideF32::ZERO,
            ModSource::Lfo1 => self.lfos[0].output(),
            ModSource::Lfo2 => self.lfos[1].output(),
            ModSource::Lfo3 => self.lfos[2].output(),
            ModSource::Lfo4 => self.lfos[3].output(),
            ModSource::EnvLpf => context.filter_env,
            ModSource::EnvVca => context.amp_env,
            ModSource::Env3 => context.aux_env,
            ModSource::PitchBend => WideF32::splat(context.performance.pitch_bend),
            ModSource::ModWheel => WideF32::splat(context.performance.mod_wheel),
            ModSource::Pressure => WideF32::splat(context.performance.pressure),
            ModSource::Breath => WideF32::splat(context.performance.breath),
            ModSource::FootPedal => WideF32::splat(context.performance.foot),
            ModSource::ExpressionPedal => WideF32::splat(context.performance.expression),
            ModSource::Velocity => context.velocities,
            ModSource::NoteNumber => self.lanes.notes_as_f32() * WideF32::splat(1.0 / 127.0),
            ModSource::Dc => WideF32::splat(1.0),
        }
    }
}

#[cfg(test)]
impl VoiceBlock {
    pub(crate) fn lanes(&self) -> &Lanes {
        &self.lanes
    }

    pub(crate) fn filter(&self) -> &Filter {
        &self.filter
    }

    pub(crate) fn amplifier(&self) -> &Amplifier {
        &self.amplifier
    }

    pub(crate) fn aux(&self) -> &AuxEnv {
        &self.aux
    }

    pub(crate) fn aux_mut(&mut self) -> &mut AuxEnv {
        &mut self.aux
    }

    pub(crate) fn pan(&self) -> &Pan {
        &self.pan
    }

    pub(crate) fn lfos(&self) -> &[Lfo; LFO_COUNT] {
        &self.lfos
    }

    pub(crate) fn oscillators(&self) -> &Oscillators {
        &self.oscillators
    }

    fn effective_lfo_rate_hz(&self, index: usize) -> f32 {
        self.lfos[index].effective_rate_hz(self.tempo_bpm, self.clock_division)
    }

    fn evaluate_lfo_control_routes_for_test(
        &self,
        modulation: &PatchModulation,
        context: ModSignalContext,
    ) -> LfoControlModulation {
        self.evaluate_lfo_control_routes(modulation, context)
    }

    fn advance_lfos_for_test(
        &mut self,
        control: LfoControlModulation,
        modulation: &PatchModulation,
    ) {
        self.advance_lfos(control, modulation);
    }

    fn apply_audio_modulation_routes_for_test(
        &self,
        patch_modulation: &PatchModulation,
        modulation: &mut LfoModulation,
        context: ModSignalContext,
    ) {
        self.apply_audio_modulation_routes(patch_modulation, modulation, context);
    }

    fn for_each_modulation_route(
        &self,
        patch_modulation: &PatchModulation,
        context: ModSignalContext,
        mut apply: impl FnMut(ModDestination, WideF32),
    ) {
        for lfo in &self.lfos {
            apply(lfo.destination(), lfo.output());
        }

        apply(patch_modulation.aux_destination(), context.aux_signal);

        for slot in patch_modulation.matrix_free_slots() {
            if !slot.enabled {
                continue;
            }

            let signal = self.mod_source_signal(slot.source, context) * WideF32::splat(slot.amount);
            apply(slot.destination, signal);
        }

        for (index, slot) in patch_modulation
            .matrix_dedicated_slots()
            .iter()
            .copied()
            .enumerate()
        {
            if !slot.enabled {
                continue;
            }

            let dedicated_source = DedicatedModSource::ALL[index].source();
            let signal =
                self.mod_source_signal(dedicated_source, context) * WideF32::splat(slot.amount);
            apply(slot.destination, signal);
        }
    }
}

const fn is_section_patch_param(id: ParamId) -> bool {
    matches!(
        id,
        ParamId::Osc1Waveform
            | ParamId::Osc1Enabled
            | ParamId::Osc1Frequency
            | ParamId::Osc1FineTune
            | ParamId::Osc1ShapeMod
            | ParamId::Osc1Level
            | ParamId::Osc1NoteReset
            | ParamId::Osc1KeyboardOn
            | ParamId::Osc1Glide
            | ParamId::Osc2Waveform
            | ParamId::Osc2Enabled
            | ParamId::Osc2Frequency
            | ParamId::Osc2FineTune
            | ParamId::Osc2ShapeMod
            | ParamId::Osc2Level
            | ParamId::Osc2NoteReset
            | ParamId::Osc2KeyboardOn
            | ParamId::Osc2Glide
            | ParamId::OscMix
            | ParamId::SubOscLevel
            | ParamId::NoiseLevel
            | ParamId::HardSync
            | ParamId::OscSlop
            | ParamId::AnalogDrift
            | ParamId::GlideMode
            | ParamId::GlideEnabled
            | ParamId::FilterCutoff
            | ParamId::FilterResonance
            | ParamId::FilterPoles
            | ParamId::FilterKeyTrack
            | ParamId::FilterEnvAmount
            | ParamId::FilterVelocity
            | ParamId::FilterAudioMod
            | ParamId::FilterEgDelay
            | ParamId::FilterEgAttack
            | ParamId::FilterEgDecay
            | ParamId::FilterEgSustain
            | ParamId::FilterEgRelease
            | ParamId::VcaInitialLevel
            | ParamId::AmpEnvAmount
            | ParamId::AmpVelocity
            | ParamId::AmpEgDelay
            | ParamId::AmpEgAttack
            | ParamId::AmpEgDecay
            | ParamId::AmpEgSustain
            | ParamId::AmpEgRelease
            | ParamId::PanSpread
            | ParamId::PanModMode
            | ParamId::AuxEgVelocity
            | ParamId::AuxEgDelay
            | ParamId::AuxEgAttack
            | ParamId::AuxEgDecay
            | ParamId::AuxEgSustain
            | ParamId::AuxEgRelease
            | ParamId::AuxEgLoop
            | ParamId::Lfo1Rate
            | ParamId::Lfo2Rate
            | ParamId::Lfo3Rate
            | ParamId::Lfo4Rate
            | ParamId::Lfo1Depth
            | ParamId::Lfo2Depth
            | ParamId::Lfo3Depth
            | ParamId::Lfo4Depth
            | ParamId::Lfo1Waveform
            | ParamId::Lfo2Waveform
            | ParamId::Lfo3Waveform
            | ParamId::Lfo4Waveform
            | ParamId::Lfo1Destination
            | ParamId::Lfo2Destination
            | ParamId::Lfo3Destination
            | ParamId::Lfo4Destination
            | ParamId::Lfo1ClockSync
            | ParamId::Lfo2ClockSync
            | ParamId::Lfo3ClockSync
            | ParamId::Lfo4ClockSync
            | ParamId::Lfo1SyncDivision
            | ParamId::Lfo2SyncDivision
            | ParamId::Lfo3SyncDivision
            | ParamId::Lfo4SyncDivision
            | ParamId::Lfo1KeySync
            | ParamId::Lfo2KeySync
            | ParamId::Lfo3KeySync
            | ParamId::Lfo4KeySync
    )
}

#[derive(Default)]
struct LfoControlModulation {
    rate_mod: [f32; LFO_COUNT],
    depth_mod: [f32; LFO_COUNT],
}

impl LfoControlModulation {
    fn apply(&mut self, destination: ModDestination, value: f32) {
        match destination {
            ModDestination::Lfo1Frequency => self.rate_mod[0] += value,
            ModDestination::Lfo2Frequency => self.rate_mod[1] += value,
            ModDestination::Lfo3Frequency => self.rate_mod[2] += value,
            ModDestination::Lfo4Frequency => self.rate_mod[3] += value,
            ModDestination::LfoAllFrequency => {
                for rate_mod in &mut self.rate_mod {
                    *rate_mod += value;
                }
            }
            ModDestination::Lfo1Amount => self.depth_mod[0] += value,
            ModDestination::Lfo2Amount => self.depth_mod[1] += value,
            ModDestination::Lfo3Amount => self.depth_mod[2] += value,
            ModDestination::Lfo4Amount => self.depth_mod[3] += value,
            ModDestination::LfoAllAmount => {
                for depth_mod in &mut self.depth_mod {
                    *depth_mod += value;
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct LfoModulation {
    oscillators: OscillatorModulation,
    osc1_shape: f32,
    osc2_shape: f32,
    filter_cutoff: PreparedCutoffModulation,
    filter_resonance: WideF32,
    filter_audio_mod: WideF32,
    amp_gain: WideF32,
    pan: WideF32,
    effects: EffectModulation,
}

impl LfoModulation {
    fn apply_destination(&mut self, destination: ModDestination, signal: WideF32) {
        match destination {
            ModDestination::Off => {}
            ModDestination::Osc1Frequency => {
                self.oscillators.osc1_frequency_semitones +=
                    signal * WideF32::splat(LFO_PITCH_DEPTH_SEMITONES);
            }
            ModDestination::Osc2Frequency => {
                self.oscillators.osc2_frequency_semitones +=
                    signal * WideF32::splat(LFO_PITCH_DEPTH_SEMITONES);
            }
            ModDestination::OscAllFrequency => {
                let pitch = signal * WideF32::splat(LFO_PITCH_DEPTH_SEMITONES);
                self.oscillators.osc1_frequency_semitones += pitch;
                self.oscillators.osc2_frequency_semitones += pitch;
            }
            ModDestination::OscMix => self.oscillators.mix += signal,
            ModDestination::NoiseLevel => self.oscillators.noise_level += signal,
            ModDestination::SubOscLevel => self.oscillators.sub_level += signal,
            ModDestination::Osc1ShapeMod => self.osc1_shape += signal.reduce_mean(),
            ModDestination::Osc2ShapeMod => self.osc2_shape += signal.reduce_mean(),
            ModDestination::OscAllShapeMod => {
                let shape = signal.reduce_mean();
                self.osc1_shape += shape;
                self.osc2_shape += shape;
            }
            ModDestination::FilterCutoff => {
                self.filter_cutoff
                    .add(signal * WideF32::splat(LFO_CUTOFF_DEPTH_SEMITONES));
            }
            ModDestination::FilterResonance => self.filter_resonance += signal,
            ModDestination::FilterAudioMod => self.filter_audio_mod += signal,
            ModDestination::Vca => self.amp_gain += signal,
            ModDestination::Pan => self.pan += signal,
            ModDestination::FxMix => self.effects.mix += signal.reduce_mean(),
            ModDestination::FxParam1 => self.effects.param1 += signal.reduce_mean(),
            ModDestination::FxParam2 => self.effects.param2 += signal.reduce_mean(),
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
struct PreparedCutoffModulation {
    lanes: WideF32,
    uniform: Option<f32>,
}

impl Default for PreparedCutoffModulation {
    fn default() -> Self {
        Self {
            lanes: WideF32::ZERO,
            uniform: Some(0.0),
        }
    }
}

impl PreparedCutoffModulation {
    #[inline(always)]
    fn set_uniform(&mut self, value: f32) {
        self.lanes = WideF32::splat(value);
        self.uniform = Some(value);
    }

    #[inline(always)]
    fn add(&mut self, contribution: WideF32) {
        self.lanes += contribution;
        self.uniform = self.uniform.and_then(|value| {
            Self::uniform_lane_value(contribution).map(|contribution| value + contribution)
        });
    }

    #[inline(always)]
    fn uniform_lane_value(value: WideF32) -> Option<f32> {
        let lanes = value.to_array();
        lanes[1..]
            .iter()
            .all(|lane| lane.to_bits() == lanes[0].to_bits())
            .then_some(lanes[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::MAX_LFO_RATE_HZ;
    use crate::patch::{DedicatedModSource, ModRoute};
    use crate::voice::VoiceManager;
    use crate::{ControlMessage, ParamId};

    fn test_block(sample_rate: f32, patch: &Patch) -> (VoiceBlock, PatchModulation) {
        let mut modulation = PatchModulation::default();
        modulation.apply_from_patch(patch);
        let mut block = VoiceBlock::new(sample_rate);
        block.apply_voice_patch(patch);
        block.refresh_lfo_engines();
        (block, modulation)
    }

    fn stereo_rms(voices: &mut VoiceManager, frames: usize) -> (f32, f32) {
        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let mut ctx = crate::create_render_context!();
        for _ in 0..frames {
            let (left, right) = voices.next(&mut ctx);
            left_sum += left * left;
            right_sum += right * right;
        }
        (
            (left_sum / frames as f32).sqrt(),
            (right_sum / frames as f32).sqrt(),
        )
    }

    fn process_frames(voices: &mut VoiceManager, frames: usize) {
        let mut ctx = crate::create_render_context!();
        for _ in 0..frames {
            voices.next(&mut ctx);
        }
    }

    fn voice_block_next(block: &mut VoiceBlock, modulation: &PatchModulation) -> (f32, f32) {
        let mut ctx = crate::create_render_context!();
        block.next(PerformanceModulation::default(), modulation, &mut ctx)
    }

    #[test]
    fn synchronized_lfo_rate_tracks_bpm_and_clock_division() {
        let (mut block, _) = test_block(1_000.0, &Patch::default());
        block.set_lfo_sync_division(0, LfoSyncDivision::Step1);
        block.set_lfo_clock_sync(0, true);
        assert_eq!(block.effective_lfo_rate_hz(0), 2.0);

        block.set_tempo_bpm(60.0);
        assert_eq!(block.effective_lfo_rate_hz(0), 1.0);
        block.set_clock_division(ClockDivision::Sixteenth);
        assert_eq!(block.effective_lfo_rate_hz(0), 4.0);

        block.set_tempo_bpm(30.0);
        block.set_clock_division(ClockDivision::Half);
        block.set_lfo_sync_division(0, LfoSyncDivision::Steps32);
        assert_eq!(block.effective_lfo_rate_hz(0), 1.0 / 128.0);

        block.set_tempo_bpm(250.0);
        block.set_clock_division(ClockDivision::SixtyFourthTriplet);
        block.set_lfo_sync_division(0, LfoSyncDivision::StepOneSixteenth);
        assert_eq!(block.effective_lfo_rate_hz(0), MAX_LFO_RATE_HZ);
    }

    #[test]
    fn master_clock_changes_do_not_reset_lfo_phase() {
        let (mut block, mut modulation) = test_block(1_000.0, &Patch::default());
        block.set_lfo_depth(0, 1.0);
        modulation.set_lfo_depth(0, 1.0);
        block.set_lfo_clock_sync(0, true);
        block.refresh_lfo_engines();
        block.advance_lfos_for_test(LfoControlModulation::default(), &modulation);
        let before = block.lfos()[0].output();

        block.set_tempo_bpm(90.0);
        block.set_clock_division(ClockDivision::Sixteenth);
        assert_eq!(block.lfos()[0].output(), before);

        block.advance_lfos_for_test(LfoControlModulation::default(), &modulation);
        assert_ne!(block.lfos()[0].output(), before);
    }

    #[test]
    fn direct_patch_application_sets_the_master_clock() {
        let mut patch = Patch::default();
        patch.bpm = 90.0;
        patch.clock_divide = ClockDivision::EighthTriplet;
        patch.lfos[0].clock_sync = true;
        patch.lfos[0].sync_division = LfoSyncDivision::StepTwoThirds;

        let (block, _) = test_block(1_000.0, &patch);
        assert_eq!(block.effective_lfo_rate_hz(0), 6.75);
    }

    #[test]
    fn free_lfo_rate_is_independent_of_master_clock() {
        let (mut block, _) = test_block(1_000.0, &Patch::default());
        block.set_lfo_rate_hz(0, 3.25);
        block.set_tempo_bpm(250.0);
        block.set_clock_division(ClockDivision::SixtyFourthTriplet);
        assert_eq!(block.effective_lfo_rate_hz(0), 3.25);
    }

    #[test]
    fn synchronized_lfo_ignores_rate_modulation() {
        let (mut block, mut modulation) = test_block(1_000.0, &Patch::default());
        block.set_lfo_depth(0, 1.0);
        block.set_lfo_sync_division(0, LfoSyncDivision::Step1);
        block.set_lfo_clock_sync(0, true);
        modulation.test_plan_mut().rate_target_mask = 1;
        modulation.test_plan_mut().active_lfo_mask = 1;

        // Advance once with rate_mod=1.0 (would 4× the rate if applied)
        let mut control = LfoControlModulation::default();
        control.rate_mod[0] = 1.0;
        block.advance_lfos_for_test(control, &modulation);

        // Capture the output after rate-modulated advance
        let modulated_output = block.lfos()[0].output();

        // Reset a fresh block and advance once WITHOUT rate modulation
        let (mut block2, mut modulation2) = test_block(1_000.0, &Patch::default());
        block2.set_lfo_depth(0, 1.0);
        block2.set_lfo_sync_division(0, LfoSyncDivision::Step1);
        block2.set_lfo_clock_sync(0, true);
        modulation2.test_plan_mut().rate_target_mask = 1;
        modulation2.test_plan_mut().active_lfo_mask = 1;
        block2.advance_lfos_for_test(LfoControlModulation::default(), &modulation2);

        let clean_output = block2.lfos()[0].output();

        assert_eq!(
            modulated_output, clean_output,
            "synced LFO rate must be unaffected by rate modulation"
        );
    }

    fn modulation_context(sample: usize) -> ModSignalContext {
        let ramp = (sample & 255) as f32 / 255.0;
        ModSignalContext {
            performance: PerformanceModulation {
                pitch_bend: ramp * 2.0 - 1.0,
                mod_wheel: ramp,
                pressure: 1.0 - ramp,
                breath: 0.25,
                foot: 0.5,
                expression: 0.75,
            },
            velocities: WideF32::splat([0.2, 0.4, 0.6, 0.8][sample % 4]),
            filter_env: WideF32::splat(ramp),
            amp_env: WideF32::splat(1.0 - ramp),
            aux_env: WideF32::splat(0.5),
            aux_signal: WideF32::splat(0.3),
        }
    }

    fn modulation_step_compiled(
        block: &mut VoiceBlock,
        patch_modulation: &PatchModulation,
        context: ModSignalContext,
    ) -> LfoModulation {
        let control = block.evaluate_lfo_control_routes_for_test(patch_modulation, context);
        block.advance_lfos_for_test(control, patch_modulation);
        let mut modulation = LfoModulation::default();
        block.apply_audio_modulation_routes_for_test(patch_modulation, &mut modulation, context);
        modulation
    }

    fn modulation_step_reference(
        block: &mut VoiceBlock,
        patch_modulation: &PatchModulation,
        context: ModSignalContext,
    ) -> LfoModulation {
        let mut control = LfoControlModulation::default();
        block.for_each_modulation_route(patch_modulation, context, |destination, signal| {
            control.apply(destination, signal.reduce_mean());
        });
        let rates: [f32; LFO_COUNT] = core::array::from_fn(|i| {
            lfo::base_rates(&block.lfos)[i] * F32(control.rate_mod[i] * 4.0).exp2().as_f32()
        });
        let depths: [f32; LFO_COUNT] = core::array::from_fn(|i| {
            (lfo::base_depths(&block.lfos)[i] + control.depth_mod[i]).clamp(0.0, 1.0)
        });
        for (index, lfo) in block.lfos.iter_mut().enumerate() {
            lfo.apply_engine_rate(rates[index]);
            lfo.apply_engine_depth(depths[index]);
            lfo.generate();
        }
        let mut modulation = LfoModulation::default();
        block.for_each_modulation_route(patch_modulation, context, |destination, signal| {
            apply_destination_modulation_reference(&mut modulation, destination, signal);
        });
        modulation
    }

    fn apply_destination_modulation_reference(
        modulation: &mut LfoModulation,
        destination: ModDestination,
        signal: WideF32,
    ) {
        match destination {
            ModDestination::Osc1ShapeMod => modulation.oscillators.osc1_shape += signal,
            ModDestination::Osc2ShapeMod => modulation.oscillators.osc2_shape += signal,
            ModDestination::OscAllShapeMod => {
                modulation.oscillators.osc1_shape += signal;
                modulation.oscillators.osc2_shape += signal;
            }
            _ => modulation.apply_destination(destination, signal),
        }
    }

    fn assert_lanes_equal(actual: WideF32, expected: WideF32) {
        assert_eq!(
            actual.to_array().map(f32::to_bits),
            expected.to_array().map(f32::to_bits)
        );
    }

    fn assert_modulation_equal(actual: &LfoModulation, expected: &LfoModulation) {
        assert_lanes_equal(
            actual.oscillators.osc1_frequency_semitones,
            expected.oscillators.osc1_frequency_semitones,
        );
        let expected_osc1_shape = expected.oscillators.osc1_shape.reduce_mean();
        let expected_osc2_shape = expected.oscillators.osc2_shape.reduce_mean();
        assert!((actual.osc1_shape - expected_osc1_shape).abs() <= 2e-6);
        assert!((actual.osc2_shape - expected_osc2_shape).abs() <= 2e-6);
        assert_lanes_equal(actual.filter_cutoff.lanes, expected.filter_cutoff.lanes);
        assert_lanes_equal(actual.amp_gain, expected.amp_gain);
        assert_eq!(actual.effects.mix.to_bits(), expected.effects.mix.to_bits());
    }

    fn configure_compiled_reference_case(
        block: &mut VoiceBlock,
        patch_modulation: &mut PatchModulation,
    ) {
        block.set_lfo_rate_hz(0, 3.0);
        block.set_lfo_depth(0, 0.8);
        block.set_lfo_waveform(0, LfoWaveform::Triangle);
        block.set_lfo_destination(0, ModDestination::Osc1ShapeMod);
        patch_modulation.set_lfo_depth(0, 0.8);
        patch_modulation.set_lfo_destination(0, ModDestination::Osc1ShapeMod);
        block.set_lfo_rate_hz(1, 0.7);
        block.set_lfo_depth(1, 0.6);
        block.set_lfo_waveform(1, LfoWaveform::Saw);
        patch_modulation.set_lfo_depth(1, 0.6);
        patch_modulation.set_aux_destination(ModDestination::Lfo2Frequency);
        patch_modulation.set_aux_amount(0.3);
        patch_modulation.set_mod_route(
            ModRoute::Free(0),
            true,
            ModSource::Lfo2,
            ModDestination::FilterCutoff,
            -0.7,
        );
        patch_modulation.set_mod_route(
            ModRoute::Free(1),
            true,
            ModSource::Velocity,
            ModDestination::Osc2ShapeMod,
            0.4,
        );
        patch_modulation.set_mod_route(
            ModRoute::Free(2),
            false,
            ModSource::Lfo1,
            ModDestination::Vca,
            1.0,
        );
        patch_modulation.set_mod_route(
            ModRoute::Dedicated(DedicatedModSource::ModWheel),
            true,
            ModSource::Off,
            ModDestination::FxMix,
            0.25,
        );
        block.refresh_lfo_engines();
    }

    #[test]
    fn compiled_modulation_matches_two_pass_reference_for_4096_samples() {
        let (mut compiled, mut compiled_modulation) = test_block(48_000.0, &Patch::default());
        let (mut reference, mut reference_modulation) = test_block(48_000.0, &Patch::default());
        configure_compiled_reference_case(&mut compiled, &mut compiled_modulation);
        configure_compiled_reference_case(&mut reference, &mut reference_modulation);

        for sample in 0..4096 {
            let context = modulation_context(sample);
            let actual = modulation_step_compiled(&mut compiled, &compiled_modulation, context);
            let expected =
                modulation_step_reference(&mut reference, &reference_modulation, context);
            for index in 0..4 {
                assert_lanes_equal(
                    compiled.lfos()[index].output(),
                    reference.lfos()[index].output(),
                );
            }
            assert_modulation_equal(&actual, &expected);
        }
    }

    #[test]
    fn compiled_route_changes_apply_on_next_sample_and_preserve_sample_hold_phase() {
        let (mut compiled, mut compiled_modulation) = test_block(100.0, &Patch::default());
        let (mut reference, mut reference_modulation) = test_block(100.0, &Patch::default());
        for block in [&mut compiled, &mut reference] {
            block.set_lfo_rate_hz(0, 10.0);
            block.set_lfo_depth(0, 1.0);
            block.set_lfo_waveform(0, LfoWaveform::Triangle);
            block.set_lfo_rate_hz(1, 7.0);
            block.set_lfo_waveform(1, LfoWaveform::SampleAndHold);
        }
        compiled_modulation.set_lfo_depth(0, 1.0);
        reference_modulation.set_lfo_depth(0, 1.0);

        for sample in 0..64 {
            let context = modulation_context(sample);
            let _ = modulation_step_compiled(&mut compiled, &compiled_modulation, context);
            let _ = modulation_step_reference(&mut reference, &reference_modulation, context);
        }
        compiled.set_lfo_depth(1, 1.0);
        reference.set_lfo_depth(1, 1.0);
        compiled.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        reference.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        compiled_modulation.set_lfo_depth(1, 1.0);
        reference_modulation.set_lfo_depth(1, 1.0);
        compiled_modulation.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        reference_modulation.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        compiled.refresh_lfo_engines();
        reference.refresh_lfo_engines();
        assert_eq!(compiled_modulation.plan().audio_count, 1);
        assert_eq!(compiled_modulation.plan().active_lfo_mask & 0b11, 0b11);

        let context = modulation_context(64);
        let actual = modulation_step_compiled(&mut compiled, &compiled_modulation, context);
        let expected = modulation_step_reference(&mut reference, &reference_modulation, context);
        assert_lanes_equal(compiled.lfos()[1].output(), reference.lfos()[1].output());
        assert_modulation_equal(&actual, &expected);
    }

    #[test]
    fn single_pwm_fast_path_requires_exactly_one_lfo_to_osc1_shape_route() {
        let (mut block, mut patch_modulation) = test_block(48_000.0, &Patch::default());
        block.set_lfo_depth(0, 1.0);
        patch_modulation.set_lfo_depth(0, 1.0);
        patch_modulation.set_mod_route(
            ModRoute::Free(0),
            true,
            ModSource::Lfo1,
            ModDestination::Osc1ShapeMod,
            0.49,
        );
        let route = patch_modulation
            .plan()
            .single_pwm_route
            .expect("single PWM route should compile to the fast path");
        assert_eq!(route.lfo_index, 0);
        assert_eq!(route.amount.to_bits(), 0.49f32.to_bits());

        patch_modulation.set_mod_route(
            ModRoute::Free(1),
            true,
            ModSource::ModWheel,
            ModDestination::FilterCutoff,
            0.5,
        );
        assert!(patch_modulation.plan().single_pwm_route.is_none());
    }

    fn pan_lanes_reference(block: &VoiceBlock, lanes: WideF32, pan_mod: WideF32) -> (f32, f32) {
        let voice_position = WideF32::new(block.lanes().pan_positions_array());
        let position = match block.pan().mod_mode() {
            PanModMode::Alternate => {
                let spread = (WideF32::splat(block.pan().spread()) + pan_mod)
                    .clamp(WideF32::ZERO, WideF32::splat(1.0));
                voice_position * spread
            }
            PanModMode::Fixed => (voice_position * WideF32::splat(block.pan().spread()) + pan_mod)
                .clamp(WideF32::splat(-1.0), WideF32::splat(1.0)),
        };
        let angle = (position + WideF32::splat(1.0)) * WideF32::splat(core::f32::consts::FRAC_PI_4);
        let (sin, cos) = angle.sin_cos();

        ((lanes * cos).reduce_add(), (lanes * sin).reduce_add())
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn pan_lanes_matches_rev2_mode_equations() {
        let lanes = WideF32::new(core::array::from_fn(|i| {
            [0.75, -0.25, 0.125, -0.0625][i % 4]
        }));
        let mut block = VoiceBlock::new(44_100.0);

        block.set_pan_spread(1.0);
        block.note_on(0, 60, 1.0, false);
        let expected = pan_lanes_reference(&block, lanes, WideF32::splat(0.75));
        let actual = block.pan_lanes(lanes, WideF32::splat(0.75));
        assert_eq!(actual.0.to_bits(), expected.0.to_bits());
        assert_eq!(actual.1.to_bits(), expected.1.to_bits());

        block.set_pan_mod_mode(PanModMode::Fixed);
        let expected = pan_lanes_reference(&block, lanes, WideF32::splat(-0.25));
        let actual = block.pan_lanes(lanes, WideF32::splat(-0.25));
        assert_eq!(actual.0.to_bits(), expected.0.to_bits());
        assert_eq!(actual.1.to_bits(), expected.1.to_bits());
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn a_single_voice_keeps_its_physical_pan_position() {
        let lanes = WideF32::new(core::array::from_fn(|i| [1.0, 0.0, 0.0, 0.0][i % 4]));
        let mut block = VoiceBlock::new(44_100.0);
        block.set_pan_spread(1.0);

        let (left, right) = block.pan_lanes(lanes, WideF32::ZERO);

        assert!(
            left.abs() < 1.0e-6,
            "voice 1 should be hard right, got {left}"
        );
        assert!(
            (right - 1.0).abs() < 1.0e-6,
            "voice 1 right gain was {right}"
        );
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn alternate_modulation_changes_width_and_fixed_modulation_translates() {
        let lanes = WideF32::new(core::array::from_fn(|i| [1.0, 1.0, 0.0, 0.0][i % 4]));
        let mut block = VoiceBlock::new(44_100.0);
        block.set_pan_spread(0.5);

        let alternate = block.pan_lanes(lanes, WideF32::splat(0.25));
        assert!((alternate.0 - alternate.1).abs() < 1.0e-6);

        block.set_pan_mod_mode(PanModMode::Fixed);
        let fixed = block.pan_lanes(lanes, WideF32::splat(0.25));
        assert!(
            fixed.1 > fixed.0,
            "positive Fixed modulation should move the program right"
        );
    }

    #[test]
    fn matrix_route_can_modulate_lfo_frequency_before_lfo_outputs_are_used() {
        fn lfo2_peak(matrix_enabled: bool) -> f32 {
            let (mut block, mut patch_modulation) = test_block(100.0, &Patch::default());
            block.set_lfo_rate_hz(0, 1.0);
            block.set_lfo_depth(0, 1.0);
            block.set_lfo_waveform(0, LfoWaveform::Square);
            block.set_lfo_rate_hz(1, 0.1);
            block.set_lfo_depth(1, 1.0);
            block.set_lfo_waveform(1, LfoWaveform::Saw);
            patch_modulation.set_lfo_depth(0, 1.0);
            patch_modulation.set_lfo_depth(1, 1.0);
            patch_modulation.set_mod_route(
                ModRoute::Free(0),
                matrix_enabled,
                ModSource::Lfo1,
                ModDestination::Lfo2Frequency,
                1.0,
            );
            block.refresh_lfo_engines();

            let mut peak = 0.0f32;
            for _ in 0..64 {
                voice_block_next(&mut block, &patch_modulation);
                peak = peak.max(block.lfos()[1].output().to_array()[0]);
            }
            peak
        }

        let static_peak = lfo2_peak(false);
        let modulated_peak = lfo2_peak(true);

        assert!(
            modulated_peak > static_peak + 0.25,
            "LFO1 -> LFO2 Frequency should accelerate LFO2 before it modulates downstream destinations, static {static_peak}, modulated {modulated_peak}"
        );
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn aux_envelope_route_can_modulate_lfo_frequency_as_internal_route() {
        fn lfo2_peak(aux_enabled: bool) -> f32 {
            let (mut block, mut patch_modulation) = test_block(100.0, &Patch::default());
            block.set_lfo_rate_hz(1, 0.1);
            block.set_lfo_depth(1, 1.0);
            block.set_lfo_waveform(1, LfoWaveform::Saw);
            patch_modulation.set_lfo_depth(1, 1.0);
            patch_modulation.set_aux_destination(if aux_enabled {
                ModDestination::Lfo2Frequency
            } else {
                ModDestination::Off
            });
            patch_modulation.set_aux_amount(1.0);
            block.set_aux_attack(0.0005);
            block.set_aux_decay(5.0);
            block.set_aux_sustain(1.0);
            block.note_on(0, 60, 1.0, false);
            block.refresh_lfo_engines();

            let mut peak = 0.0f32;
            for _ in 0..64 {
                voice_block_next(&mut block, &patch_modulation);
                peak = peak.max(block.lfos()[1].output().to_array()[0]);
            }
            peak
        }

        let static_peak = lfo2_peak(false);
        let modulated_peak = lfo2_peak(true);

        assert!(
            modulated_peak > static_peak * 1.5,
            "Aux Envelope -> LFO2 Frequency should share the pre-LFO route phase, static {static_peak}, modulated {modulated_peak}"
        );
    }

    #[test]
    fn pan_spread_creates_stereo_separation() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let mut diff_sum = 0.0;
        let frames = 4096;
        let mut ctx = crate::create_render_context!();
        for _ in 0..frames {
            let (left, right) = voices.next(&mut ctx);
            left_sum += left * left;
            right_sum += right * right;
            let diff = left - right;
            diff_sum += diff * diff;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();
        let difference = (diff_sum / frames as f32).sqrt();

        assert!(
            difference > left.max(right) * 0.5,
            "two voices should create stereo difference at full spread, left {left}, right {right}, diff {difference}"
        );
    }

    #[test]
    fn pan_spread_pans_a_single_voice_to_its_physical_position() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let (left, right) = stereo_rms(&mut voices, 4096);

        assert!(
            right > left * 100.0,
            "physical voice 1 should pan right at full spread, left {left}, right {right}"
        );
    }

    #[test]
    fn pan_lfo_modulates_spread_width_instead_of_offset() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 0.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Waveform, 3.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::SetParam(
            ParamId::Lfo1Destination,
            ModDestination::Pan.index() as f32,
        ));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let (left, right) = stereo_rms(&mut voices, 4096);

        assert!(
            (left - right).abs() < left.max(right) * 0.1,
            "positive pan modulation should widen alternating voices symmetrically, left {left}, right {right}"
        );
    }

    #[test]
    fn repeated_notes_advance_through_physical_voice_pan_positions() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let (first_left, first_right) = stereo_rms(&mut voices, 2048);
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        process_frames(&mut voices, 512);

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let (second_left, second_right) = stereo_rms(&mut voices, 2048);

        assert!(
            first_right > first_left * 100.0,
            "physical voice 1 should pan right, left {first_left}, right {first_right}"
        );
        assert!(
            second_left > second_right * 100.0,
            "physical voice 2 should pan left, left {second_left}, right {second_right}"
        );
    }

    #[test]
    fn oscillator_tuning_param_does_not_replace_midi_note() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 72.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.lanes().gates_array().iter().any(|gate| *gate))
            .unwrap();
        let lane = block
            .lanes()
            .gates_array()
            .iter()
            .position(|gate| *gate)
            .unwrap();
        let expected = crate::midi_to_hz(112);
        let osc1_freq = block.oscillators().osc1_frequency_hz().to_array()[lane];
        assert_eq!(block.lanes().note(lane), 64);
        assert!(
            (osc1_freq - expected).abs() < 0.1,
            "osc1 should track MIDI note + (freq - 24), got {} expected {expected}",
            osc1_freq
        );
    }

    #[test]
    fn oscillator_frequency_and_fine_tune_use_natural_units_and_clamp() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 240.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1FineTune, 99.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 24,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.lanes().gates_array().iter().any(|gate| *gate))
            .unwrap();
        let lane = block
            .lanes()
            .gates_array()
            .iter()
            .position(|gate| *gate)
            .unwrap();
        assert!(
            (block.oscillators().params().osc1.frequency_semitones - 120.0).abs() < f32::EPSILON,
            "osc1 frequency param should clamp to 120"
        );
        assert!(
            (block.oscillators().params().osc1.fine_tune_cents - 50.0).abs() < f32::EPSILON,
            "osc1 fine tune should clamp to +50 cents"
        );
        let expected = crate::midi_to_hz(120) * 2.0f32.powf(50.0 / 1200.0);
        let osc1_freq = block.oscillators().osc1_frequency_hz().to_array()[lane];

        assert!(
            (osc1_freq - expected).abs() < 0.5,
            "note 24 + clamped freq 120 should be MIDI 120 +50c, got {osc1_freq}, expected {expected}"
        );
    }

    #[test]
    fn osc_mix_is_canonical_balance_control() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.25));

        let params = voices[0].oscillators.params();
        assert_eq!(params.osc_mix, 0.25);
    }

    #[test]
    fn osc_slop_zero_is_stable_and_full_slop_offsets_lanes() {
        let mut stable = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72] {
            stable.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let stable_block = &stable[0];
        for lane in 0..WideF32::LANES {
            let expected = crate::midi_to_hz(stable_block.lanes().notes_array()[lane]);
            let freq = stable_block.oscillators().osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "slop 0 should not detune lane {lane}, got {freq}, expected {expected}"
            );
        }

        let mut sloppy = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        sloppy.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            sloppy.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let sloppy_block = &sloppy[0];
        let offsets: [f32; WideF32::LANES] = core::array::from_fn(|lane| {
            let expected = crate::midi_to_hz(sloppy_block.lanes().notes_array()[lane]);
            sloppy_block.oscillators().osc1_frequency_hz().to_array()[lane] - expected
        });

        assert!(
            offsets.iter().any(|offset| offset.abs() > 0.01),
            "full slop should offset at least one lane, offsets {offsets:?}"
        );
    }

    #[test]
    fn clearing_osc_slop_restores_intended_frequency() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 0.0));
        let block = &voices[0];
        for lane in 0..WideF32::LANES {
            let expected = crate::midi_to_hz(block.lanes().notes_array()[lane]);
            let freq = block.oscillators().osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "clearing slop should restore lane {lane}, got {freq}, expected {expected}"
            );
        }
    }

    #[test]
    fn note_reset_flags_are_routed_to_oscillators() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1NoteReset, 0.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2NoteReset, 1.0));

        let params = voices[0].oscillators.params();
        assert!(!params.osc1.note_reset);
        assert!(params.osc2.note_reset);
    }

    #[test]
    fn hard_sync_param_is_routed_to_oscillators() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));

        assert!(voices[0].oscillators.params().sync);
    }

    #[test]
    fn aux_envelope_to_oscillator_frequency_modulates_pitch() {
        let mut voices = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(
            ParamId::AuxEgDestination,
            ModDestination::Osc1Frequency.index() as f32,
        ));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.0005));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 5.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        process_frames(&mut voices, 1_000);

        let block = &voices[0];
        let freq = block.oscillators().osc1_frequency_hz().to_array()[0];
        let expected = crate::midi_to_hz(72);
        assert!(
            (freq - expected).abs() < 5.0,
            "full positive aux pitch modulation should raise osc1 by about one octave, got {freq}, expected {expected}"
        );
    }

    #[test]
    fn aux_repeat_keeps_envelope_cycling_while_held() {
        let mut repeating = VoiceManager::<{ crate::VOICE_PACKS }>::new(1_000.0);
        repeating.handle_control(ControlMessage::SetParam(
            ParamId::AuxEgDestination,
            ModDestination::FilterCutoff.index() as f32,
        ));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgAmount, 1.0));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgAttack, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgDecay, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgSustain, 0.5));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgRelease, 0.001));
        repeating.handle_control(ControlMessage::SetParam(ParamId::AuxEgLoop, 1.0));
        repeating.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let first = repeating[0].aux_mut().envelope_next().to_array()[0];
        let reset = repeating[0].aux_mut().envelope_next().to_array()[0];
        let second = repeating[0].aux_mut().envelope_next().to_array()[0];

        assert!(first > 0.9);
        assert_eq!(reset, 0.0);
        assert!(second > 0.9);

        repeating.handle_control(ControlMessage::NoteOff { note: 60 });
        assert_eq!(repeating[0].aux_mut().envelope_next().to_array()[0], 0.0);
        assert!(repeating[0].aux().envelope_is_idle_lane(0));
    }

    #[test]
    fn vca_initial_level_at_one_ignores_amp_envelope_amount() {
        let mut drone = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        drone.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 1.0));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.0));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.001));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
        drone.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut drone, 4_096);
        let (drone_left, _) = stereo_rms(&mut drone, 4_096);
        assert!(
            drone_left > 0.05,
            "full VCA level should bypass amp envelope amount, RMS {drone_left}"
        );

        let mut gated = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        gated.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 0.0));
        gated.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 0.0));
        gated.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.001));
        gated.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
        gated.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut gated, 4_096);
        let (gated_left, _) = stereo_rms(&mut gated, 4_096);
        assert!(
            gated_left < 0.001,
            "zero VCA level and envelope amount should be silent, RMS {gated_left}"
        );

        let mut enveloped = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        enveloped.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 0.0));
        enveloped.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 1.0));
        enveloped.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.001));
        enveloped.handle_control(ControlMessage::SetParam(ParamId::AmpEgSustain, 1.0));
        enveloped.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut enveloped, 4_096);
        let (enveloped_left, _) = stereo_rms(&mut enveloped, 4_096);
        assert!(
            enveloped_left > 0.05,
            "zero VCA level with full envelope amount should still gate audibly, RMS {enveloped_left}"
        );
    }

    #[test]
    fn vca_initial_level_keeps_drone_rendering_after_amp_release() {
        let mut drone = VoiceManager::<{ crate::VOICE_PACKS }>::new(44_100.0);
        drone.handle_control(ControlMessage::SetParam(ParamId::VcaInitialLevel, 0.25));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEnvAmount, 1.0));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
        drone.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));
        drone.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut drone, 512);
        drone.handle_control(ControlMessage::NoteOff { note: 60 });
        process_frames(&mut drone, 512);

        assert!(drone[0].amplifier().is_idle_lane(0));
        assert!(!drone[0].is_lane_silent(0));
        let (left, _) = stereo_rms(&mut drone, 4_096);
        assert!(
            left > 0.01,
            "VCA bias should remain audible after the amp envelope release, RMS {left}"
        );
    }

    #[test]
    fn voice_reuse_fades_vca_bias_out_and_back_in() {
        let mut patch = Patch::default();
        patch.amplifier.initial_level = 1.0;
        patch.amplifier.env_amount = 0.0;
        let (mut block, modulation) = test_block(44_100.0, &patch);
        block.note_on(0, 60, 1.0, false);
        for _ in 0..221 {
            voice_block_next(&mut block, &modulation);
        }
        let gains = block.lanes().lifecycle_gains_array();
        assert_eq!(gains[0], 1.0);
        for gain in gains.iter().skip(1) {
            assert_eq!(*gain, 0.0);
        }

        block.schedule_note_on(0, 64, 1.0, false);
        for _ in 0..221 {
            voice_block_next(&mut block, &modulation);
        }
        assert!(block.has_pending_note(0));
        assert_eq!(block.lanes().lifecycle_gains_array()[0], 0.0);

        voice_block_next(&mut block, &modulation);
        assert!(!block.has_pending_note(0));
        let fade_in_start = block.lanes().lifecycle_gains_array()[0];
        assert!(fade_in_start > 0.0 && fade_in_start < 0.001);
        for _ in 1..221 {
            voice_block_next(&mut block, &modulation);
        }
        assert_eq!(block.lanes().lifecycle_gains_array()[0], 1.0);
    }

    #[test]
    fn lifecycle_fade_on_wide_pulse_does_not_create_dc_click() {
        let sample_rate = 44_100.0;
        let mut patch = Patch::default();
        patch.osc1.waveform = 3;
        patch.osc1.enabled = true;
        patch.osc1.shape_mod = 0.67;
        patch.osc2.enabled = false;
        patch.osc_mix = 0.0;
        patch.sub_osc_level = 0.0;
        patch.noise_level = 0.0;
        patch.filter.cutoff = 8_000.0;
        patch.filter.resonance = 0.0;
        patch.amplifier.initial_level = 1.0;
        patch.amplifier.env_amount = 0.0;
        let (mut block, modulation) = test_block(sample_rate, &patch);
        block.note_on(0, 96, 1.0, false);

        let settle = (sample_rate * 0.25) as usize;
        for _ in 0..settle {
            voice_block_next(&mut block, &modulation);
        }

        block.schedule_note_on(0, 100, 1.0, false);
        let fade_samples = block.lifecycle_shutdown_samples() as usize;
        let mut fade_sum = 0.0;
        for _ in 0..fade_samples {
            let (left, right) = voice_block_next(&mut block, &modulation);
            fade_sum += left + right;
        }
        let fade_mean = fade_sum / (2.0 * fade_samples as f32);
        assert!(
            fade_mean.abs() < 0.05,
            "lifecycle fade of a DC-blocked signal should not invent a DC click, mean={fade_mean}"
        );
    }

    #[test]
    fn amp_gain_step_on_wide_pulse_does_not_create_dc_click() {
        let sample_rate = 44_100.0;
        let mut patch = Patch::default();
        patch.osc1.waveform = 3;
        patch.osc1.enabled = true;
        patch.osc1.shape_mod = 0.67;
        patch.osc2.enabled = false;
        patch.osc_mix = 0.0;
        patch.sub_osc_level = 0.0;
        patch.noise_level = 0.0;
        patch.filter.cutoff = 8_000.0;
        patch.filter.resonance = 0.0;
        patch.amplifier.env_amount = 1.0;
        patch.amplifier.eg_attack = 0.0005;
        patch.amplifier.eg_decay = 0.0005;
        patch.amplifier.eg_sustain = 1.0;
        let (mut block, modulation) = test_block(sample_rate, &patch);
        block.note_on(0, 96, 1.0, false);

        let settle = (sample_rate * 0.25) as usize;
        for _ in 0..settle {
            voice_block_next(&mut block, &modulation);
        }

        block.set_param(ParamId::AmpEgSustain, 0.25);
        let measure = (sample_rate * 0.005) as usize;
        let mut sum = 0.0;
        for _ in 0..measure {
            let (left, right) = voice_block_next(&mut block, &modulation);
            sum += left + right;
        }
        let mean = sum / (2.0 * measure as f32);
        assert!(
            mean.abs() < 0.05,
            "amp sustain steps after DC blocking should not invent a DC click, mean={mean}"
        );
    }

    #[test]
    fn live_amp_env_amount_step_ramps_gain_instead_of_jumping() {
        let sample_rate = 44_100.0;
        let mut patch = Patch::default();
        patch.osc1.enabled = true;
        patch.osc2.enabled = false;
        patch.amplifier.env_amount = 0.0;
        patch.amplifier.eg_attack = 0.0005;
        patch.amplifier.eg_decay = 0.0005;
        patch.amplifier.eg_sustain = 1.0;
        let (mut block, modulation) = test_block(sample_rate, &patch);
        block.note_on(0, 60, 1.0, false);

        let settle = (sample_rate * 0.05) as usize;
        for _ in 0..settle {
            voice_block_next(&mut block, &modulation);
        }

        block.set_param(ParamId::AmpEnvAmount, 1.0);
        let (left_before, _) = voice_block_next(&mut block, &modulation);
        let (left_after, _) = voice_block_next(&mut block, &modulation);
        let step = (left_after - left_before).abs();
        assert!(
            step < 0.2,
            "live amp env amount step should ramp gradually, per-sample step={step}"
        );
    }

    #[test]
    fn patch_load_snaps_amp_env_amount_without_ramp() {
        let sample_rate = 44_100.0;
        let mut patch = Patch::default();
        patch.osc1.enabled = true;
        patch.osc2.enabled = false;
        patch.amplifier.env_amount = 0.0;
        patch.amplifier.eg_attack = 0.0005;
        patch.amplifier.eg_decay = 0.0005;
        patch.amplifier.eg_sustain = 1.0;
        let (mut block, modulation) = test_block(sample_rate, &patch);
        block.note_on(0, 60, 1.0, false);

        let settle = (sample_rate * 0.05) as usize;
        for _ in 0..settle {
            voice_block_next(&mut block, &modulation);
        }

        block.set_param(ParamId::AmpEnvAmount, 1.0);
        for _ in 0..64 {
            voice_block_next(&mut block, &modulation);
        }

        patch.amplifier.env_amount = 0.25;
        block.apply_voice_patch(&patch);
        let (snapped, _) = voice_block_next(&mut block, &modulation);
        let (next, _) = voice_block_next(&mut block, &modulation);
        let step = (next - snapped).abs();
        assert!(
            step < snapped.abs() * 0.2 + 1e-4,
            "patch load should snap amp env amount instead of continuing the prior ramp, snapped={snapped}, next={next}"
        );
    }

    #[test]
    fn filter_envelope_amount_opens_closed_filter() {
        let mut with_env = VoiceBlock::new(44_100.0);
        with_env.set_param(ParamId::FilterCutoff, 112.0);
        with_env.set_param(ParamId::FilterEnvAmount, 1.0);
        with_env.set_param(ParamId::FilterEgAttack, 0.0005);
        with_env.set_param(ParamId::FilterEgDecay, 5.0);
        with_env.set_param(ParamId::FilterEgSustain, 1.0);
        with_env.set_param(ParamId::AmpEnvAmount, 1.0);
        with_env.set_param(ParamId::AmpEgAttack, 0.0005);
        with_env.set_param(ParamId::AmpEgDecay, 0.0005);
        with_env.set_param(ParamId::AmpEgSustain, 1.0);
        with_env.set_param(ParamId::OscMix, 0.0);
        with_env.set_param(ParamId::NoiseLevel, 0.0);
        with_env.set_param(ParamId::SubOscLevel, 0.0);
        assert!((with_env.filter().env_amount() - 1.0).abs() < 1e-6);

        let mut without_env = VoiceBlock::new(44_100.0);
        without_env.set_param(ParamId::FilterCutoff, 112.0);
        without_env.set_param(ParamId::FilterEnvAmount, 0.0);
        without_env.set_param(ParamId::AmpEnvAmount, 1.0);
        without_env.set_param(ParamId::AmpEgAttack, 0.0005);
        without_env.set_param(ParamId::AmpEgDecay, 0.0005);
        without_env.set_param(ParamId::AmpEgSustain, 1.0);
        without_env.set_param(ParamId::OscMix, 0.0);
        without_env.set_param(ParamId::NoiseLevel, 0.0);
        without_env.set_param(ParamId::SubOscLevel, 0.0);

        with_env.note_on(0, 60, 1.0, false);
        without_env.note_on(0, 60, 1.0, false);
        let modulation = PatchModulation::default();
        let mut with_sum = 0.0f32;
        let mut without_sum = 0.0f32;
        for _ in 0..2048 {
            with_sum += voice_block_next(&mut with_env, &modulation).0.powi(2);
            without_sum += voice_block_next(&mut without_env, &modulation).0.powi(2);
        }
        let with_rms = (with_sum / 2048.0).sqrt();
        let without_rms = (without_sum / 2048.0).sqrt();
        assert!(
            with_rms > without_rms * 1.5,
            "with_env {with_rms}, without_env {without_rms}"
        );
    }
}
