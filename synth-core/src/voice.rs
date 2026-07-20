//! Single voice-block signal chain (four SIMD lanes).

use crate::f32x4;

use crate::analog_oscillators::{OscillatorModulation, Oscillators};
use crate::effects::EffectModulation;
use crate::patch::{
    DedicatedModSlot, DedicatedModSource, ModDestination, ModMatrixSlot, ModRoute, PanModMode,
    Patch,
};
#[cfg(feature = "profiling")]
use crate::profiling::{NoopProfiler, RenderProfiler, RenderStage};
use crate::{
    DadsrEnvelope, Filter, FilterOversampling, FilterType, LANES, Lfo, LfoWaveform,
    MIN_LFO_RATE_HZ, ModSource, ModulationParam, ParamId, Waveform, midi_to_hz,
};

const LFO_PITCH_DEPTH_SEMITONES: f32 = 12.0;
const LFO_CUTOFF_DEPTH_SEMITONES: f32 = 48.0;
const MAX_COMPILED_MOD_ROUTES: usize = 18;
/// Short smooth release used before replacing an audible voice (SynthLab precedent).
const VOICE_STEAL_SHUTDOWN_SECONDS: f32 = 0.005;

/// Provisional Rev2-16 physical-voice pan pattern.
///
/// Sequential documents a deterministic alternating pattern whose voices move
/// progressively toward center. These coefficients are isolated so measured
/// hardware values can replace the estimate without changing pan semantics.
pub const REV2_VOICE_PAN_POSITIONS: [f32; 16] = [
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

#[derive(Clone, Copy)]
struct PendingNote {
    note: u8,
    velocity: f32,
    reset_key_synced_lfos: bool,
}

/// Four-lane subtractive voice: oscillators → filter → amplifier.
///
/// Each lane can represent a separate note. Envelopes, LFOs, and modulation are
/// evaluated per lane each sample step.
pub struct VoiceBlock {
    notes: [u8; LANES],
    velocities: [f32; LANES],
    gates: [bool; LANES],
    ages: [u64; LANES],
    pending_notes: [Option<PendingNote>; LANES],
    pending_note_mask: u8,

    oscillators: Oscillators,
    amp_env: DadsrEnvelope,
    filter_env: DadsrEnvelope,
    filter: Filter,
    aux_env: DadsrEnvelope,
    aux_env_destination: ModDestination,
    aux_env_amount: f32,
    aux_env_velocity_amount: f32,
    lfos: [Lfo; 4],
    lfo_destinations: [ModDestination; 4],
    lfo_clock_sync: [bool; 4],
    lfo_base_rates_hz: [f32; 4],
    lfo_base_depths: [f32; 4],
    last_lfo_outputs: [f32x4; 4],
    last_effect_modulation: EffectModulation,
    mod_matrix_slots: [ModMatrixSlot; 8],
    dedicated_mod_slots: [DedicatedModSlot; 5],
    modulation_plan: ModulationExecutionPlan,
    defer_modulation_plan_rebuild: bool,
    vca_initial_level: f32,
    amp_env_amount: f32,
    amp_velocity_amount: f32,
    pan_spread: f32,
    pan_positions: [f32; LANES],
    pan_mod_mode: PanModMode,
    centered_pan_sin: f32x4,
    centered_pan_cos: f32x4,
    pitch_bend_range: f32,

    sample_rate: f32,
}

impl VoiceBlock {
    pub fn new(sample_rate: f32, patch: &Patch) -> Self {
        let (centered_pan_sin, centered_pan_cos) =
            f32x4::splat(core::f32::consts::FRAC_PI_4).sin_cos();
        let mut block = Self {
            notes: [60; LANES],
            velocities: [1.0; LANES],
            gates: [false; LANES],
            ages: [0; LANES],
            pending_notes: [None; LANES],
            pending_note_mask: 0,
            oscillators: Oscillators::new(sample_rate),
            amp_env: DadsrEnvelope::analog(sample_rate),
            filter_env: DadsrEnvelope::analog(sample_rate),
            filter: Filter::default(),
            aux_env: DadsrEnvelope::analog(sample_rate),
            aux_env_destination: ModDestination::Off,
            aux_env_amount: 0.0,
            aux_env_velocity_amount: 0.0,
            lfos: core::array::from_fn(|_| Lfo::new(sample_rate)),
            lfo_destinations: [ModDestination::Off; 4],
            lfo_clock_sync: [false; 4],
            lfo_base_rates_hz: [MIN_LFO_RATE_HZ; 4],
            lfo_base_depths: [0.0; 4],
            last_lfo_outputs: [f32x4::splat(0.0); 4],
            last_effect_modulation: EffectModulation::default(),
            mod_matrix_slots: [ModMatrixSlot::default(); 8],
            dedicated_mod_slots: [DedicatedModSlot::default(); 5],
            modulation_plan: ModulationExecutionPlan::default(),
            defer_modulation_plan_rebuild: false,
            vca_initial_level: 0.0,
            amp_env_amount: 0.0,
            amp_velocity_amount: 0.0,
            pan_spread: 0.0,
            pan_positions: core::array::from_fn(|lane| voice_pan_position(lane, LANES)),
            pan_mod_mode: PanModMode::Alternate,
            centered_pan_sin,
            centered_pan_cos,
            pitch_bend_range: 0.0,
            sample_rate,
        };
        block.apply_patch(patch);
        block
    }

    pub fn note_on(&mut self, lane: usize, note: u8, velocity: f32, reset_key_synced_lfos: bool) {
        self.pending_notes[lane] = None;
        self.pending_note_mask &= !(1 << lane);
        self.notes[lane] = note;
        self.velocities[lane] = velocity;
        self.gates[lane] = true;
        self.ages[lane] = 0;
        self.amp_env.trigger_lane(lane);
        self.filter_env.trigger_lane(lane);
        self.aux_env.trigger_lane(lane);

        if reset_key_synced_lfos {
            for lfo in &mut self.lfos {
                if lfo.key_sync() {
                    lfo.reset_all();
                }
            }
        }
        self.oscillators.note_on(lane, self.note_frequencies_hz());
        self.filter.reset_lane(lane);
    }

    pub(crate) fn schedule_note_on(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        reset_key_synced_lfos: bool,
    ) {
        let shutdown_in_progress = self.pending_notes[lane].is_some();
        self.pending_notes[lane] = Some(PendingNote {
            note,
            velocity,
            reset_key_synced_lfos,
        });
        self.pending_note_mask |= 1 << lane;
        if shutdown_in_progress {
            return;
        }
        self.gates[lane] = false;
        self.amp_env
            .shutdown_lane(lane, VOICE_STEAL_SHUTDOWN_SECONDS);
        self.filter_env.release_lane(lane);
        self.aux_env.release_lane(lane);
    }

    pub fn note_off(&mut self, note: u8) {
        for lane in 0..LANES {
            if self.active_note(lane) == Some(note) {
                self.note_off_lane(lane);
            }
        }
    }

    pub fn note_off_lane(&mut self, lane: usize) {
        self.pending_notes[lane] = None;
        self.pending_note_mask &= !(1 << lane);
        self.gates[lane] = false;
        self.amp_env.release_lane(lane);
        self.filter_env.release_lane(lane);
        self.aux_env.release_lane(lane);
    }

    pub fn all_notes_off(&mut self) {
        self.pending_notes = [None; LANES];
        self.pending_note_mask = 0;
        self.gates = [false; LANES];
        self.amp_env.release_all();
        self.filter_env.release_all();
        self.aux_env.release_all();
    }

    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.filter.set_oversampling(oversampling);
    }

    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.filter.set_filter_type(filter_type);
    }

    pub fn next(&mut self, performance: PerformanceModulation) -> (f32, f32) {
        #[cfg(feature = "profiling")]
        {
            return self.next_inner(performance, &mut NoopProfiler);
        }
        #[cfg(not(feature = "profiling"))]
        self.next_inner(performance)
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn next_profiled(
        &mut self,
        performance: PerformanceModulation,
        profiler: &mut impl RenderProfiler,
    ) -> (f32, f32) {
        self.next_inner(performance, profiler)
    }

    #[cfg(not(feature = "profiling"))]
    pub(crate) fn next_block(&mut self, performance: PerformanceModulation) -> (f32, f32) {
        self.next_inner(performance)
    }

    fn next_inner(
        &mut self,
        performance: PerformanceModulation,
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) -> (f32, f32) {
        self.start_pending_notes();
        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::EnvelopesAndModulation);
        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::EnvelopeAdvance);
        let velocities = f32x4::new(self.velocities);
        let aux_env = self.aux_env.next();
        let aux_velocity_scale = f32x4::splat(1.0 - self.aux_env_velocity_amount)
            + velocities * f32x4::splat(self.aux_env_velocity_amount);
        let aux_signal = aux_env * f32x4::splat(self.aux_env_amount) * aux_velocity_scale;
        let filter_env = self.filter_env.next();
        let amp = self.amp_env.next();
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::EnvelopeAdvance);

        let context = ModSignalContext {
            performance,
            velocities,
            filter_env,
            amp_env: amp,
            aux_env,
            aux_signal,
        };
        let pitch_bend = f32x4::splat(performance.pitch_bend * self.pitch_bend_range);
        let mut lfo_modulation = LfoModulation::default();
        lfo_modulation.oscillators.osc1_frequency_semitones = pitch_bend;
        lfo_modulation.oscillators.osc2_frequency_semitones = pitch_bend;
        if self.modulation_plan.any_modulation {
            #[cfg(feature = "profiling")]
            profiler.begin(RenderStage::LfoControlRouting);
            let lfo_control = if self.modulation_plan.control_count == 0 {
                LfoControlModulation::default()
            } else {
                self.evaluate_lfo_control_routes(context)
            };
            #[cfg(feature = "profiling")]
            profiler.end(RenderStage::LfoControlRouting);

            #[cfg(feature = "profiling")]
            profiler.begin(RenderStage::LfoGeneration);
            self.advance_lfos(lfo_control);
            #[cfg(feature = "profiling")]
            profiler.end(RenderStage::LfoGeneration);

            #[cfg(feature = "profiling")]
            profiler.begin(RenderStage::AudioModulationRouting);
            if let Some(route) = self.modulation_plan.single_pwm_route {
                lfo_modulation.osc1_shape =
                    average_lanes(self.last_lfo_outputs[route.lfo_index as usize]) * route.amount;
            } else if let Some(route) = self.modulation_plan.single_filter_cutoff_route {
                let index = route.lfo_index as usize;
                let scale = route.amount * LFO_CUTOFF_DEPTH_SEMITONES;
                if self.lfos[index].output_is_uniform() {
                    lfo_modulation
                        .filter_cutoff
                        .set_uniform(self.last_lfo_outputs[index].to_array()[0] * scale);
                } else {
                    lfo_modulation
                        .filter_cutoff
                        .add(self.last_lfo_outputs[index] * f32x4::splat(scale));
                }
            } else {
                self.apply_audio_modulation_routes(&mut lfo_modulation, context);
            }
            #[cfg(feature = "profiling")]
            profiler.end(RenderStage::AudioModulationRouting);
        }
        self.last_effect_modulation = lfo_modulation.effects;
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::EnvelopesAndModulation);

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::Oscillators);
        #[cfg(feature = "profiling")]
        let osc = self.oscillators.next_prepared_profiled(
            lfo_modulation.oscillators,
            [lfo_modulation.osc1_shape, lfo_modulation.osc2_shape],
            profiler,
        );
        #[cfg(not(feature = "profiling"))]
        let osc = self.oscillators.next_prepared(
            lfo_modulation.oscillators,
            [lfo_modulation.osc1_shape, lfo_modulation.osc2_shape],
        );
        let mix = osc.audio;
        self.filter
            .set_self_oscillation_color_enabled(!osc.audio_source_active);
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::Oscillators);

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::Filter);
        let notes = f32x4::new(self.notes.map(|note| note as f32));
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
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::Filter);

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::AmplifierAndPan);
        let velocity_gain =
            f32x4::splat(1.0 - self.amp_velocity_amount) + velocities * self.amp_velocity_amount;
        let env_gain = f32x4::splat(self.vca_initial_level)
            + (f32x4::splat(1.0 - self.vca_initial_level) * amp * self.amp_env_amount);
        let amp_lfo_gain = (f32x4::splat(1.0) + lfo_modulation.amp_gain)
            .clamp(f32x4::splat(0.0), f32x4::splat(2.0));
        let output = filtered * velocity_gain * env_gain * amp_lfo_gain;

        let stereo = self.pan_lanes(output, lfo_modulation.pan);
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::AmplifierAndPan);
        stereo
    }

    fn evaluate_lfo_control_routes(&self, context: ModSignalContext) -> LfoControlModulation {
        let mut lfo_control = LfoControlModulation::default();
        for route in self.modulation_plan.control_routes() {
            let signal = route.signal(self, context);
            lfo_control.apply(route.destination, average_lanes(signal));
        }
        lfo_control
    }

    fn advance_lfos(&mut self, lfo_control: LfoControlModulation) {
        if self.modulation_plan.rate_target_mask == 0 && self.modulation_plan.depth_target_mask == 0
        {
            for (index, lfo) in self.lfos.iter_mut().enumerate() {
                if self.modulation_plan.active_lfo_mask & (1 << index) != 0 {
                    self.last_lfo_outputs[index] = lfo.next();
                } else {
                    lfo.advance_silent();
                    self.last_lfo_outputs[index] = f32x4::ZERO;
                }
            }
            return;
        }

        let rates = if lfo_control.rate_mod == [0.0; 4] {
            self.lfo_base_rates_hz
        } else {
            (f32x4::new(self.lfo_base_rates_hz)
                * (f32x4::new(lfo_control.rate_mod) * f32x4::splat(4.0)).exp2())
            .to_array()
        };
        let depths = if lfo_control.depth_mod == [0.0; 4] {
            self.lfo_base_depths
        } else {
            (f32x4::new(self.lfo_base_depths) + f32x4::new(lfo_control.depth_mod))
                .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
                .to_array()
        };

        for (index, lfo) in self.lfos.iter_mut().enumerate() {
            let bit = 1 << index;
            if self.modulation_plan.rate_target_mask & bit != 0 {
                lfo.set_rate_hz(rates[index]);
            }
            if self.modulation_plan.depth_target_mask & bit != 0 {
                lfo.set_depth(depths[index]);
            }
            if self.modulation_plan.active_lfo_mask & bit != 0 {
                self.last_lfo_outputs[index] = lfo.next();
            } else {
                lfo.advance_silent();
                self.last_lfo_outputs[index] = f32x4::ZERO;
            }
        }
    }

    fn note_frequencies_hz(&self) -> f32x4 {
        f32x4::new(self.notes.map(midi_to_hz))
    }

    fn pan_lanes(&self, lanes: f32x4, pan_mod: f32x4) -> (f32, f32) {
        if self.pan_spread == 0.0
            && (self.pan_mod_mode == PanModMode::Alternate || pan_mod == f32x4::ZERO)
        {
            return (
                (lanes * self.centered_pan_cos).reduce_add(),
                (lanes * self.centered_pan_sin).reduce_add(),
            );
        }

        let voice_position = f32x4::new(self.pan_positions);
        let position = match self.pan_mod_mode {
            PanModMode::Alternate => {
                let spread =
                    (f32x4::splat(self.pan_spread) + pan_mod).clamp(f32x4::ZERO, f32x4::splat(1.0));
                voice_position * spread
            }
            PanModMode::Fixed => (voice_position * f32x4::splat(self.pan_spread) + pan_mod)
                .clamp(f32x4::splat(-1.0), f32x4::splat(1.0)),
        };
        let angle = (position + f32x4::splat(1.0)) * f32x4::splat(core::f32::consts::FRAC_PI_4);
        let (sin, cos) = angle.sin_cos();

        ((lanes * cos).reduce_add(), (lanes * sin).reduce_add())
    }

    pub fn is_lane_silent(&self, lane: usize) -> bool {
        self.pending_notes[lane].is_none() && !self.gates[lane] && self.amp_env.is_idle_lane(lane)
    }

    pub fn is_lane_released(&self, lane: usize) -> bool {
        self.pending_notes[lane].is_none() && !self.gates[lane]
    }

    pub fn for_each_active_note(&self, mut f: impl FnMut(u8)) {
        for lane in 0..LANES {
            if let Some(pending) = self.pending_notes[lane] {
                f(pending.note);
            } else if self.gates[lane] {
                f(self.notes[lane]);
            }
        }
    }

    pub(crate) fn active_note(&self, lane: usize) -> Option<u8> {
        self.pending_notes[lane]
            .map(|pending| pending.note)
            .or_else(|| self.gates[lane].then_some(self.notes[lane]))
    }

    pub(crate) fn has_pending_note(&self, lane: usize) -> bool {
        self.pending_notes[lane].is_some()
    }

    fn start_pending_notes(&mut self) {
        if self.pending_note_mask == 0 {
            return;
        }
        for lane in 0..LANES {
            if self.pending_note_mask & (1 << lane) == 0 {
                continue;
            }
            if !self.amp_env.is_idle_lane(lane) {
                continue;
            }
            let Some(pending) = self.pending_notes[lane].take() else {
                continue;
            };

            self.amp_env.reset_lane(lane);
            self.filter_env.reset_lane(lane);
            self.aux_env.reset_lane(lane);
            self.note_on(
                lane,
                pending.note,
                pending.velocity,
                pending.reset_key_synced_lfos,
            );
        }
    }

    pub fn active_lane_count(&self) -> usize {
        (0..LANES)
            .filter(|&lane| !self.is_lane_silent(lane))
            .count()
    }

    pub fn age_active_lanes(&mut self) {
        for lane in 0..LANES {
            if self.gates[lane] {
                self.ages[lane] += 1;
            }
        }
    }

    pub fn oldest_lane(&self) -> usize {
        self.ages
            .iter()
            .enumerate()
            .max_by_key(|(_, age)| *age)
            .map(|(lane, _)| lane)
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

    pub fn set_pitch_bend_range(&mut self, semitones: f32) {
        self.pitch_bend_range = semitones.clamp(0.0, 12.0);
    }

    pub fn set_amp_attack(&mut self, seconds: f32) {
        self.amp_env.set_attack_seconds(seconds);
    }

    pub fn set_amp_delay(&mut self, seconds: f32) {
        self.amp_env.set_delay_seconds(seconds);
    }

    pub fn set_amp_decay(&mut self, seconds: f32) {
        self.amp_env.set_decay_seconds(seconds);
    }

    pub fn set_amp_sustain(&mut self, sustain: f32) {
        self.amp_env.set_sustain_level(sustain);
    }

    pub fn set_amp_release(&mut self, seconds: f32) {
        self.amp_env.set_release_seconds(seconds);
    }

    pub fn set_vca_initial_level(&mut self, level: f32) {
        self.vca_initial_level = level.clamp(0.0, 1.0);
    }

    pub fn set_amp_env_amount(&mut self, amount: f32) {
        self.amp_env_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_amp_velocity_amount(&mut self, amount: f32) {
        self.amp_velocity_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_pan_spread(&mut self, spread: f32) {
        self.pan_spread = spread.clamp(0.0, 1.0);
    }

    pub fn set_pan_mod_mode(&mut self, mode: PanModMode) {
        self.pan_mod_mode = mode;
    }

    pub(crate) fn set_pan_positions(&mut self, positions: [f32; LANES]) {
        self.pan_positions = positions.map(|position| position.clamp(-1.0, 1.0));
    }

    pub fn set_lfo_rate_hz(&mut self, index: usize, rate_hz: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            self.lfo_base_rates_hz[index] = rate_hz;
            lfo.set_rate_hz(rate_hz);
        }
    }

    pub fn set_lfo_depth(&mut self, index: usize, depth: f32) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            self.lfo_base_depths[index] = depth.clamp(0.0, 1.0);
            lfo.set_depth(depth);
            self.refresh_modulation_plan();
        }
    }

    pub fn set_lfo_waveform(&mut self, index: usize, waveform: LfoWaveform) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_waveform(waveform);
        }
    }

    pub fn set_lfo_destination(&mut self, index: usize, destination: ModDestination) {
        if let Some(slot) = self.lfo_destinations.get_mut(index) {
            *slot = destination;
            self.refresh_modulation_plan();
        }
    }

    pub fn set_lfo_clock_sync(&mut self, index: usize, clock_sync: bool) {
        if let Some(slot) = self.lfo_clock_sync.get_mut(index) {
            *slot = clock_sync;
        }
    }

    pub fn set_lfo_key_sync(&mut self, index: usize, key_sync: bool) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.set_key_sync(key_sync);
        }
    }

    pub fn set_filter_delay(&mut self, seconds: f32) {
        self.filter_env.set_delay_seconds(seconds);
    }

    pub fn set_filter_attack(&mut self, seconds: f32) {
        self.filter_env.set_attack_seconds(seconds);
    }

    pub fn set_filter_decay(&mut self, seconds: f32) {
        self.filter_env.set_decay_seconds(seconds);
    }

    pub fn set_filter_sustain(&mut self, sustain: f32) {
        self.filter_env.set_sustain_level(sustain);
    }

    pub fn set_filter_release(&mut self, seconds: f32) {
        self.filter_env.set_release_seconds(seconds);
    }

    pub fn set_aux_destination(&mut self, destination: ModDestination) {
        self.aux_env_destination = destination;
        self.refresh_modulation_plan();
    }

    pub fn set_aux_amount(&mut self, amount: f32) {
        self.aux_env_amount = amount.clamp(-1.0, 1.0);
        self.refresh_modulation_plan();
    }

    pub fn set_aux_velocity_amount(&mut self, amount: f32) {
        self.aux_env_velocity_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_aux_delay(&mut self, seconds: f32) {
        self.aux_env.set_delay_seconds(seconds);
    }

    pub fn set_aux_attack(&mut self, seconds: f32) {
        self.aux_env.set_attack_seconds(seconds);
    }

    pub fn set_aux_decay(&mut self, seconds: f32) {
        self.aux_env.set_decay_seconds(seconds);
    }

    pub fn set_aux_sustain(&mut self, sustain: f32) {
        self.aux_env.set_sustain_level(sustain);
    }

    pub fn set_aux_release(&mut self, seconds: f32) {
        self.aux_env.set_release_seconds(seconds);
    }

    pub fn set_aux_repeat(&mut self, repeat: bool) {
        self.aux_env.set_loop_enabled(repeat);
    }

    pub fn set_mod_route(
        &mut self,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = self.mod_matrix_slots.get_mut(index) {
                    *slot = ModMatrixSlot {
                        enabled,
                        source,
                        destination,
                        amount: amount.clamp(-1.0, 1.0),
                    };
                }
            }
            ModRoute::Dedicated(source) => {
                let index = source.index();
                if let Some(slot) = self.dedicated_mod_slots.get_mut(index) {
                    *slot = DedicatedModSlot {
                        enabled,
                        destination,
                        amount: amount.clamp(-1.0, 1.0),
                    };
                }
            }
        }
        self.refresh_modulation_plan();
    }

    pub fn set_mod_route_param(&mut self, route: ModRoute, parameter: ModulationParam) {
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = self.mod_matrix_slots.get_mut(index) {
                    match parameter {
                        ModulationParam::Source(source) => slot.source = source,
                        ModulationParam::Destination(destination) => {
                            slot.destination = destination;
                        }
                        ModulationParam::Amount(amount) => {
                            slot.amount = amount.clamp(-1.0, 1.0);
                        }
                    }
                    if !matches!(parameter, ModulationParam::Amount(_)) {
                        slot.enabled = slot.source != ModSource::Off
                            && slot.destination != ModDestination::Off;
                    }
                }
            }
            ModRoute::Dedicated(source) => {
                let index = source.index();
                if let Some(slot) = self.dedicated_mod_slots.get_mut(index) {
                    match parameter {
                        ModulationParam::Destination(destination) => {
                            slot.destination = destination;
                            slot.enabled = destination != ModDestination::Off;
                        }
                        ModulationParam::Amount(amount) => {
                            slot.amount = amount.clamp(-1.0, 1.0);
                        }
                        ModulationParam::Source(_) => {}
                    }
                }
            }
        }
        self.refresh_modulation_plan();
    }

    fn apply_audio_modulation_routes(
        &self,
        modulation: &mut LfoModulation,
        context: ModSignalContext,
    ) {
        for route in self.modulation_plan.audio_routes() {
            modulation.apply_destination(route.destination, route.signal(self, context));
        }
    }

    fn rebuild_modulation_plan(&mut self) {
        self.modulation_plan = ModulationExecutionPlan::compile(
            self.lfo_base_depths,
            self.lfo_destinations,
            self.aux_env_destination,
            self.aux_env_amount,
            self.mod_matrix_slots,
            self.dedicated_mod_slots,
        );
        for index in 0..self.lfos.len() {
            self.lfos[index].set_rate_hz(self.lfo_base_rates_hz[index]);
            self.lfos[index].set_depth(self.lfo_base_depths[index]);
        }
    }

    fn refresh_modulation_plan(&mut self) {
        if !self.defer_modulation_plan_rebuild {
            self.rebuild_modulation_plan();
        }
    }

    pub(crate) fn take_effect_modulation(&mut self) -> EffectModulation {
        core::mem::take(&mut self.last_effect_modulation)
    }

    pub(crate) fn begin_patch_update(&mut self) {
        self.defer_modulation_plan_rebuild = true;
    }

    pub(crate) fn finish_patch_update(&mut self) {
        self.defer_modulation_plan_rebuild = false;
        self.rebuild_modulation_plan();
    }

    pub(crate) fn apply_patch(&mut self, patch: &Patch) {
        self.begin_patch_update();
        patch.for_each_param(|id, value| self.set_param(id, value));
        patch.for_each_modulation(|route, slot| {
            self.set_mod_route(
                route,
                slot.enabled,
                slot.source,
                slot.destination,
                slot.amount,
            );
        });
        self.finish_patch_update();
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) {
        match id {
            ParamId::Osc1Waveform => self.set_osc1_waveform(Waveform::from_index(value as usize)),
            ParamId::Osc1Enabled => self.set_osc1_enabled(value >= 0.5),
            ParamId::Osc2Waveform => self.set_osc2_waveform(Waveform::from_index(value as usize)),
            ParamId::Osc2Enabled => self.set_osc2_enabled(value >= 0.5),
            ParamId::Osc1Frequency => self.set_osc1_note_param(value),
            ParamId::Osc2Frequency => self.set_osc2_note_param(value),
            ParamId::Osc1FineTune => self.set_osc1_fine(value),
            ParamId::Osc2FineTune => self.set_osc2_fine(value),
            ParamId::Osc1ShapeMod => self.set_osc1_shape_mod(value),
            ParamId::Osc2ShapeMod => self.set_osc2_shape_mod(value),
            ParamId::Osc1Level => {
                if value <= 0.0 {
                    self.set_osc_mix(1.0);
                }
            }
            ParamId::Osc2Level => {
                if value <= 0.0 {
                    self.set_osc_mix(0.0);
                }
            }
            ParamId::OscMix => self.set_osc_mix(value),
            ParamId::SubOscLevel => self.set_sub_osc_level(value),
            ParamId::NoiseLevel => self.set_noise_level(value),
            ParamId::HardSync => self.set_hard_sync(value >= 0.5),
            ParamId::OscSlop | ParamId::AnalogDrift => self.set_osc_slop(value),
            ParamId::Osc1NoteReset => self.set_osc1_note_reset(value >= 0.5),
            ParamId::Osc2NoteReset => self.set_osc2_note_reset(value >= 0.5),
            ParamId::Osc1KeyboardOn => self.set_osc1_keyboard_on(value >= 0.5),
            ParamId::Osc2KeyboardOn => self.set_osc2_keyboard_on(value >= 0.5),
            ParamId::FilterCutoff => self.set_filter_cutoff(value),
            ParamId::FilterResonance => self.set_filter_resonance(value),
            ParamId::FilterPoles => self.set_filter_poles(if value < 0.5 { 2 } else { 4 }),
            ParamId::FilterKeyTrack => self.set_filter_key_track(value),
            ParamId::FilterEnvAmount => self.set_filter_env_amount(value),
            ParamId::FilterVelocity => self.set_filter_velocity(value),
            ParamId::FilterAudioMod => self.set_filter_audio_mod(value),
            ParamId::FilterEgDelay => self.set_filter_delay(value),
            ParamId::FilterEgAttack => self.set_filter_attack(value),
            ParamId::FilterEgDecay => self.set_filter_decay(value),
            ParamId::FilterEgSustain => self.set_filter_sustain(value),
            ParamId::FilterEgRelease => self.set_filter_release(value),
            ParamId::VcaInitialLevel => self.set_vca_initial_level(value),
            ParamId::AmpEnvAmount => self.set_amp_env_amount(value),
            ParamId::AmpVelocity => self.set_amp_velocity_amount(value),
            ParamId::AmpEgDelay => self.set_amp_delay(value),
            ParamId::AmpEgAttack => self.set_amp_attack(value),
            ParamId::AmpEgDecay => self.set_amp_decay(value),
            ParamId::AmpEgSustain => self.set_amp_sustain(value),
            ParamId::AmpEgRelease => self.set_amp_release(value),
            ParamId::AuxEgDestination => {
                self.set_aux_destination(ModDestination::from_index(value as usize));
            }
            ParamId::AuxEgAmount => self.set_aux_amount(value),
            ParamId::AuxEgVelocity => self.set_aux_velocity_amount(value),
            ParamId::AuxEgDelay => self.set_aux_delay(value),
            ParamId::AuxEgAttack => self.set_aux_attack(value),
            ParamId::AuxEgDecay => self.set_aux_decay(value),
            ParamId::AuxEgSustain => self.set_aux_sustain(value),
            ParamId::AuxEgRelease => self.set_aux_release(value),
            ParamId::AuxEgLoop => self.set_aux_repeat(value >= 0.5),
            ParamId::PanSpread => self.set_pan_spread(value),
            ParamId::PanModMode => self.set_pan_mod_mode(PanModMode::from_param(value)),
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
            ParamId::Lfo1KeySync => self.set_lfo_key_sync(0, value >= 0.5),
            ParamId::Lfo2KeySync => self.set_lfo_key_sync(1, value >= 0.5),
            ParamId::Lfo3KeySync => self.set_lfo_key_sync(2, value >= 0.5),
            ParamId::Lfo4KeySync => self.set_lfo_key_sync(3, value >= 0.5),
            ParamId::PitchBendRange => self.set_pitch_bend_range(value),
            ParamId::MasterVolume
            | ParamId::Osc1Glide
            | ParamId::Osc2Glide
            | ParamId::GlideTime
            | ParamId::GlideMode
            | ParamId::GlideEnabled
            | ParamId::KeyMode
            | ParamId::UnisonEnabled
            | ParamId::UnisonMode
            | ParamId::UnisonDetune
            | ParamId::Bpm
            | ParamId::ClockDivide => {}
            _ => {}
        }
    }

    #[cfg(test)]
    fn for_each_modulation_route(
        &self,
        context: ModSignalContext,
        mut apply: impl FnMut(ModDestination, f32x4),
    ) {
        for (output, destination) in self.last_lfo_outputs.iter().zip(self.lfo_destinations) {
            apply(destination, *output);
        }

        apply(self.aux_env_destination, context.aux_signal);

        for slot in self.mod_matrix_slots {
            if !slot.enabled {
                continue;
            }

            let signal = self.mod_source_signal(slot.source, context) * f32x4::splat(slot.amount);
            apply(slot.destination, signal);
        }

        for (index, slot) in self.dedicated_mod_slots.iter().copied().enumerate() {
            if !slot.enabled {
                continue;
            }

            let dedicated_source = DedicatedModSource::ALL[index].source();
            let signal =
                self.mod_source_signal(dedicated_source, context) * f32x4::splat(slot.amount);
            apply(slot.destination, signal);
        }
    }

    fn mod_source_signal(&self, source: ModSource, context: ModSignalContext) -> f32x4 {
        match source {
            ModSource::Off
            | ModSource::Seq1
            | ModSource::Seq2
            | ModSource::Seq3
            | ModSource::Seq4
            | ModSource::Noise
            | ModSource::AudioOut => f32x4::splat(0.0),
            ModSource::Lfo1 => self.last_lfo_outputs[0],
            ModSource::Lfo2 => self.last_lfo_outputs[1],
            ModSource::Lfo3 => self.last_lfo_outputs[2],
            ModSource::Lfo4 => self.last_lfo_outputs[3],
            ModSource::EnvLpf => context.filter_env,
            ModSource::EnvVca => context.amp_env,
            ModSource::Env3 => context.aux_env,
            ModSource::PitchBend => f32x4::splat(context.performance.pitch_bend),
            ModSource::ModWheel => f32x4::splat(context.performance.mod_wheel),
            ModSource::Pressure => f32x4::splat(context.performance.pressure),
            ModSource::Breath => f32x4::splat(context.performance.breath),
            ModSource::FootPedal => f32x4::splat(context.performance.foot),
            ModSource::ExpressionPedal => f32x4::splat(context.performance.expression),
            ModSource::Velocity => context.velocities,
            ModSource::NoteNumber => f32x4::new(self.notes.map(|note| note as f32 / 127.0)),
            ModSource::Dc => f32x4::splat(1.0),
        }
    }
}

#[cfg(test)]
impl VoiceBlock {
    pub(crate) fn test_gates(&self) -> [bool; LANES] {
        self.gates
    }

    pub(crate) fn test_gate(&self, lane: usize) -> bool {
        self.gates[lane]
    }

    pub(crate) fn test_notes(&self) -> [u8; LANES] {
        self.notes
    }

    pub(crate) fn test_note(&self, lane: usize) -> u8 {
        self.notes[lane]
    }

    pub(crate) fn test_pan_positions(&self) -> [f32; LANES] {
        self.pan_positions
    }

    pub(crate) fn test_lfo_output(&self, index: usize) -> f32 {
        self.last_lfo_outputs[index].to_array()[0]
    }

    pub(crate) fn test_osc1_frequency_hz(&self) -> f32x4 {
        self.oscillators.osc1_frequency_hz()
    }

    pub(crate) fn test_filter_cutoff(&self) -> f32 {
        self.filter.cutoff()
    }

    pub(crate) fn test_filter_resonance(&self) -> f32 {
        self.filter.resonance()
    }

    pub(crate) fn test_mod_matrix_slot(&self, index: usize) -> ModMatrixSlot {
        self.mod_matrix_slots[index]
    }
}

#[derive(Clone, Copy)]
struct ModSignalContext {
    performance: PerformanceModulation,
    velocities: f32x4,
    filter_env: f32x4,
    amp_env: f32x4,
    aux_env: f32x4,
    aux_signal: f32x4,
}

#[derive(Default)]
struct LfoControlModulation {
    rate_mod: [f32; 4],
    depth_mod: [f32; 4],
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

#[derive(Clone, Copy)]
struct PreparedCutoffModulation {
    lanes: f32x4,
    uniform: Option<f32>,
}

impl Default for PreparedCutoffModulation {
    fn default() -> Self {
        Self {
            lanes: f32x4::ZERO,
            uniform: Some(0.0),
        }
    }
}

impl PreparedCutoffModulation {
    #[inline(always)]
    fn set_uniform(&mut self, value: f32) {
        self.lanes = f32x4::splat(value);
        self.uniform = Some(value);
    }

    #[inline(always)]
    fn add(&mut self, contribution: f32x4) {
        self.lanes += contribution;
        self.uniform = self.uniform.and_then(|value| {
            Self::uniform_lane_value(contribution).map(|contribution| value + contribution)
        });
    }

    #[inline(always)]
    fn uniform_lane_value(value: f32x4) -> Option<f32> {
        let lanes = value.to_array();
        lanes[1..]
            .iter()
            .all(|lane| lane.to_bits() == lanes[0].to_bits())
            .then_some(lanes[0])
    }
}

#[derive(Default)]
struct LfoModulation {
    oscillators: OscillatorModulation,
    osc1_shape: f32,
    osc2_shape: f32,
    filter_cutoff: PreparedCutoffModulation,
    filter_resonance: f32x4,
    filter_audio_mod: f32x4,
    amp_gain: f32x4,
    pan: f32x4,
    effects: EffectModulation,
}

impl LfoModulation {
    fn apply_destination(&mut self, destination: ModDestination, signal: f32x4) {
        match destination {
            ModDestination::Off => {}
            ModDestination::Osc1Frequency => {
                self.oscillators.osc1_frequency_semitones +=
                    signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
            }
            ModDestination::Osc2Frequency => {
                self.oscillators.osc2_frequency_semitones +=
                    signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
            }
            ModDestination::OscAllFrequency => {
                let pitch = signal * f32x4::splat(LFO_PITCH_DEPTH_SEMITONES);
                self.oscillators.osc1_frequency_semitones += pitch;
                self.oscillators.osc2_frequency_semitones += pitch;
            }
            ModDestination::OscMix => self.oscillators.mix += signal,
            ModDestination::NoiseLevel => self.oscillators.noise_level += signal,
            ModDestination::SubOscLevel => self.oscillators.sub_level += signal,
            ModDestination::Osc1ShapeMod => self.osc1_shape += average_lanes(signal),
            ModDestination::Osc2ShapeMod => self.osc2_shape += average_lanes(signal),
            ModDestination::OscAllShapeMod => {
                let shape = average_lanes(signal);
                self.osc1_shape += shape;
                self.osc2_shape += shape;
            }
            ModDestination::FilterCutoff => {
                self.filter_cutoff
                    .add(signal * f32x4::splat(LFO_CUTOFF_DEPTH_SEMITONES));
            }
            ModDestination::FilterResonance => self.filter_resonance += signal,
            ModDestination::FilterAudioMod => self.filter_audio_mod += signal,
            ModDestination::Vca => self.amp_gain += signal,
            ModDestination::Pan => self.pan += signal,
            ModDestination::FxMix => self.effects.mix += average_lanes(signal),
            ModDestination::FxParam1 => self.effects.param1 += average_lanes(signal),
            ModDestination::FxParam2 => self.effects.param2 += average_lanes(signal),
            _ => {}
        }
    }
}

fn average_lanes(value: f32x4) -> f32 {
    value.reduce_add() / LANES as f32
}

#[derive(Clone, Copy)]
enum CompiledModSource {
    Standard(ModSource),
    AuxSignal,
}

#[derive(Clone, Copy)]
struct CompiledModRoute {
    source: CompiledModSource,
    destination: ModDestination,
    amount: f32,
}

impl CompiledModRoute {
    const EMPTY: Self = Self {
        source: CompiledModSource::Standard(ModSource::Off),
        destination: ModDestination::Off,
        amount: 0.0,
    };

    fn signal(self, block: &VoiceBlock, context: ModSignalContext) -> f32x4 {
        let signal = match self.source {
            CompiledModSource::Standard(source) => block.mod_source_signal(source, context),
            CompiledModSource::AuxSignal => context.aux_signal,
        };
        signal * f32x4::splat(self.amount)
    }
}

#[derive(Clone, Copy)]
struct SinglePwmRoute {
    lfo_index: u8,
    amount: f32,
}

#[derive(Clone, Copy)]
struct SingleFilterCutoffRoute {
    lfo_index: u8,
    amount: f32,
}

#[derive(Clone, Copy)]
struct ModulationExecutionPlan {
    control_routes: [CompiledModRoute; MAX_COMPILED_MOD_ROUTES],
    audio_routes: [CompiledModRoute; MAX_COMPILED_MOD_ROUTES],
    control_count: u8,
    audio_count: u8,
    active_lfo_mask: u8,
    rate_target_mask: u8,
    depth_target_mask: u8,
    total_route_count: u8,
    single_pwm_route: Option<SinglePwmRoute>,
    single_filter_cutoff_route: Option<SingleFilterCutoffRoute>,
    any_modulation: bool,
}

impl Default for ModulationExecutionPlan {
    fn default() -> Self {
        Self {
            control_routes: [CompiledModRoute::EMPTY; MAX_COMPILED_MOD_ROUTES],
            audio_routes: [CompiledModRoute::EMPTY; MAX_COMPILED_MOD_ROUTES],
            control_count: 0,
            audio_count: 0,
            active_lfo_mask: 0,
            rate_target_mask: 0,
            depth_target_mask: 0,
            total_route_count: 0,
            single_pwm_route: None,
            single_filter_cutoff_route: None,
            any_modulation: false,
        }
    }
}

impl ModulationExecutionPlan {
    fn compile(
        lfo_base_depths: [f32; 4],
        lfo_destinations: [ModDestination; 4],
        aux_destination: ModDestination,
        aux_amount: f32,
        matrix_slots: [ModMatrixSlot; 8],
        dedicated_slots: [DedicatedModSlot; 5],
    ) -> Self {
        let mut plan = Self::default();

        // A non-zero-depth output is observable through `last_lfo_outputs`
        // and may become routed on the next control update, so keep generating
        // it while phase-only advancement is reserved for zero-depth LFOs.
        for (index, depth) in lfo_base_depths.iter().enumerate() {
            if *depth != 0.0 {
                plan.active_lfo_mask |= 1 << index;
            }
        }

        let lfo_sources = [
            ModSource::Lfo1,
            ModSource::Lfo2,
            ModSource::Lfo3,
            ModSource::Lfo4,
        ];
        for (index, destination) in lfo_destinations.iter().copied().enumerate() {
            if destination != ModDestination::Off && lfo_base_depths[index] != 0.0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(lfo_sources[index]),
                    destination,
                    amount: 1.0,
                });
            }
        }
        if aux_destination != ModDestination::Off && aux_amount != 0.0 {
            plan.add_route(CompiledModRoute {
                source: CompiledModSource::AuxSignal,
                destination: aux_destination,
                amount: 1.0,
            });
        }
        for slot in matrix_slots {
            if slot.enabled && slot.amount != 0.0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(slot.source),
                    destination: slot.destination,
                    amount: slot.amount,
                });
            }
        }
        for (index, slot) in dedicated_slots.iter().copied().enumerate() {
            if slot.enabled && slot.amount != 0.0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(DedicatedModSource::ALL[index].source()),
                    destination: slot.destination,
                    amount: slot.amount,
                });
            }
        }
        plan.single_pwm_route = plan.detect_single_pwm_route();
        plan.single_filter_cutoff_route = plan.detect_single_filter_cutoff_route();
        plan.any_modulation = plan.total_route_count > 0 || plan.active_lfo_mask != 0;
        plan
    }

    fn add_route(&mut self, route: CompiledModRoute) {
        self.total_route_count += 1;
        if let CompiledModSource::Standard(source) = route.source {
            self.active_lfo_mask |= Self::lfo_source_mask(source);
        }
        self.rate_target_mask |= Self::lfo_rate_target_mask(route.destination);
        self.depth_target_mask |= Self::lfo_depth_target_mask(route.destination);

        if Self::is_lfo_control_destination(route.destination) {
            let index = self.control_count as usize;
            self.control_routes[index] = route;
            self.control_count += 1;
        } else if Self::is_audio_destination(route.destination) {
            let index = self.audio_count as usize;
            self.audio_routes[index] = route;
            self.audio_count += 1;
        }
    }

    fn control_routes(&self) -> &[CompiledModRoute] {
        &self.control_routes[..self.control_count as usize]
    }

    fn audio_routes(&self) -> &[CompiledModRoute] {
        &self.audio_routes[..self.audio_count as usize]
    }

    fn detect_single_pwm_route(&self) -> Option<SinglePwmRoute> {
        if self.total_route_count != 1 || self.control_count != 0 || self.audio_count != 1 {
            return None;
        }
        let route = self.audio_routes[0];
        if route.destination != ModDestination::Osc1ShapeMod {
            return None;
        }
        let CompiledModSource::Standard(source) = route.source else {
            return None;
        };
        let lfo_index = match source {
            ModSource::Lfo1 => 0,
            ModSource::Lfo2 => 1,
            ModSource::Lfo3 => 2,
            ModSource::Lfo4 => 3,
            _ => return None,
        };
        Some(SinglePwmRoute {
            lfo_index,
            amount: route.amount,
        })
    }

    fn detect_single_filter_cutoff_route(&self) -> Option<SingleFilterCutoffRoute> {
        if self.total_route_count != 1 || self.control_count != 0 || self.audio_count != 1 {
            return None;
        }
        let route = self.audio_routes[0];
        if route.destination != ModDestination::FilterCutoff {
            return None;
        }
        let CompiledModSource::Standard(source) = route.source else {
            return None;
        };
        let lfo_index = match source {
            ModSource::Lfo1 => 0,
            ModSource::Lfo2 => 1,
            ModSource::Lfo3 => 2,
            ModSource::Lfo4 => 3,
            _ => return None,
        };
        Some(SingleFilterCutoffRoute {
            lfo_index,
            amount: route.amount,
        })
    }

    fn lfo_source_mask(source: ModSource) -> u8 {
        match source {
            ModSource::Lfo1 => 1 << 0,
            ModSource::Lfo2 => 1 << 1,
            ModSource::Lfo3 => 1 << 2,
            ModSource::Lfo4 => 1 << 3,
            _ => 0,
        }
    }

    fn lfo_rate_target_mask(destination: ModDestination) -> u8 {
        match destination {
            ModDestination::Lfo1Frequency => 1 << 0,
            ModDestination::Lfo2Frequency => 1 << 1,
            ModDestination::Lfo3Frequency => 1 << 2,
            ModDestination::Lfo4Frequency => 1 << 3,
            ModDestination::LfoAllFrequency => 0b1111,
            _ => 0,
        }
    }

    fn lfo_depth_target_mask(destination: ModDestination) -> u8 {
        match destination {
            ModDestination::Lfo1Amount => 1 << 0,
            ModDestination::Lfo2Amount => 1 << 1,
            ModDestination::Lfo3Amount => 1 << 2,
            ModDestination::Lfo4Amount => 1 << 3,
            ModDestination::LfoAllAmount => 0b1111,
            _ => 0,
        }
    }

    fn is_lfo_control_destination(destination: ModDestination) -> bool {
        Self::lfo_rate_target_mask(destination) != 0 || Self::lfo_depth_target_mask(destination) != 0
    }

    fn is_audio_destination(destination: ModDestination) -> bool {
        matches!(
            destination,
            ModDestination::Osc1Frequency
                | ModDestination::Osc2Frequency
                | ModDestination::OscAllFrequency
                | ModDestination::OscMix
                | ModDestination::NoiseLevel
                | ModDestination::SubOscLevel
                | ModDestination::Osc1ShapeMod
                | ModDestination::Osc2ShapeMod
                | ModDestination::OscAllShapeMod
                | ModDestination::FilterCutoff
                | ModDestination::FilterResonance
                | ModDestination::FilterAudioMod
                | ModDestination::Vca
                | ModDestination::Pan
                | ModDestination::FxMix
                | ModDestination::FxParam1
                | ModDestination::FxParam2
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voices::Voices;
    use crate::{ControlMessage, ParamId};

    fn stereo_rms(voices: &mut Voices, frames: usize) -> (f32, f32) {
        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        (
            (left_sum / frames as f32).sqrt(),
            (right_sum / frames as f32).sqrt(),
        )
    }

    fn process_frames(voices: &mut Voices, frames: usize) {
        for _ in 0..frames {
            voices.next();
        }
    }

    fn voice_block_next(block: &mut VoiceBlock) -> (f32, f32) {
        block.next(PerformanceModulation::default())
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
            velocities: f32x4::new([0.2, 0.4, 0.6, 0.8]),
            filter_env: f32x4::splat(ramp),
            amp_env: f32x4::splat(1.0 - ramp),
            aux_env: f32x4::splat(0.5),
            aux_signal: f32x4::splat(0.3),
        }
    }

    fn modulation_step_compiled(
        block: &mut VoiceBlock,
        context: ModSignalContext,
    ) -> LfoModulation {
        let control = block.evaluate_lfo_control_routes(context);
        block.advance_lfos(control);
        let mut modulation = LfoModulation::default();
        block.apply_audio_modulation_routes(&mut modulation, context);
        modulation
    }

    fn modulation_step_reference(
        block: &mut VoiceBlock,
        context: ModSignalContext,
    ) -> LfoModulation {
        let mut control = LfoControlModulation::default();
        block.for_each_modulation_route(context, |destination, signal| {
            control.apply(destination, average_lanes(signal));
        });
        let rates = (f32x4::new(block.lfo_base_rates_hz)
            * (f32x4::new(control.rate_mod) * f32x4::splat(4.0)).exp2())
        .to_array();
        let depths = (f32x4::new(block.lfo_base_depths) + f32x4::new(control.depth_mod))
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0))
            .to_array();
        for (index, lfo) in block.lfos.iter_mut().enumerate() {
            lfo.set_rate_hz(rates[index]);
            lfo.set_depth(depths[index]);
            block.last_lfo_outputs[index] = lfo.next();
        }
        let mut modulation = LfoModulation::default();
        block.for_each_modulation_route(context, |destination, signal| {
            apply_destination_modulation_reference(&mut modulation, destination, signal);
        });
        modulation
    }

    fn apply_destination_modulation_reference(
        modulation: &mut LfoModulation,
        destination: ModDestination,
        signal: f32x4,
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

    fn assert_lanes_equal(actual: f32x4, expected: f32x4) {
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
        let expected_osc1_shape = average_lanes(expected.oscillators.osc1_shape);
        let expected_osc2_shape = average_lanes(expected.oscillators.osc2_shape);
        assert!((actual.osc1_shape - expected_osc1_shape).abs() <= 2e-6);
        assert!((actual.osc2_shape - expected_osc2_shape).abs() <= 2e-6);
        assert_lanes_equal(actual.filter_cutoff.lanes, expected.filter_cutoff.lanes);
        assert_lanes_equal(actual.amp_gain, expected.amp_gain);
        assert_eq!(actual.effects.mix.to_bits(), expected.effects.mix.to_bits());
    }

    fn configure_compiled_reference_case(block: &mut VoiceBlock) {
        block.set_lfo_rate_hz(0, 3.0);
        block.set_lfo_depth(0, 0.8);
        block.set_lfo_waveform(0, LfoWaveform::Triangle);
        block.set_lfo_destination(0, ModDestination::Osc1ShapeMod);
        block.set_lfo_rate_hz(1, 0.7);
        block.set_lfo_depth(1, 0.6);
        block.set_lfo_waveform(1, LfoWaveform::Saw);
        block.set_aux_destination(ModDestination::Lfo2Frequency);
        block.set_aux_amount(0.3);
        block.set_mod_route(
            ModRoute::Free(0),
            true,
            ModSource::Lfo2,
            ModDestination::FilterCutoff,
            -0.7,
        );
        block.set_mod_route(
            ModRoute::Free(1),
            true,
            ModSource::Velocity,
            ModDestination::Osc2ShapeMod,
            0.4,
        );
        block.set_mod_route(
            ModRoute::Free(2),
            false,
            ModSource::Lfo1,
            ModDestination::Vca,
            1.0,
        );
        block.set_mod_route(
            ModRoute::Dedicated(DedicatedModSource::ModWheel),
            true,
            ModSource::Off,
            ModDestination::FxMix,
            0.25,
        );
    }

    #[test]
    fn compiled_modulation_matches_two_pass_reference_for_4096_samples() {
        let mut compiled = VoiceBlock::new(48_000.0, &Patch::default());
        let mut reference = VoiceBlock::new(48_000.0, &Patch::default());
        configure_compiled_reference_case(&mut compiled);
        configure_compiled_reference_case(&mut reference);

        for sample in 0..4096 {
            let context = modulation_context(sample);
            let actual = modulation_step_compiled(&mut compiled, context);
            let expected = modulation_step_reference(&mut reference, context);
            for index in 0..4 {
                assert_lanes_equal(
                    compiled.last_lfo_outputs[index],
                    reference.last_lfo_outputs[index],
                );
            }
            assert_modulation_equal(&actual, &expected);
        }
    }

    #[test]
    fn compiled_route_changes_apply_on_next_sample_and_preserve_sample_hold_phase() {
        let mut compiled = VoiceBlock::new(100.0, &Patch::default());
        let mut reference = VoiceBlock::new(100.0, &Patch::default());
        for block in [&mut compiled, &mut reference] {
            block.set_lfo_rate_hz(0, 10.0);
            block.set_lfo_depth(0, 1.0);
            block.set_lfo_waveform(0, LfoWaveform::Triangle);
            block.set_lfo_rate_hz(1, 7.0);
            block.set_lfo_waveform(1, LfoWaveform::SampleAndHold);
        }

        for sample in 0..64 {
            let context = modulation_context(sample);
            let _ = modulation_step_compiled(&mut compiled, context);
            let _ = modulation_step_reference(&mut reference, context);
        }
        compiled.set_lfo_depth(1, 1.0);
        reference.set_lfo_depth(1, 1.0);
        compiled.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        reference.set_lfo_destination(1, ModDestination::Osc1ShapeMod);
        assert_eq!(compiled.modulation_plan.audio_count, 1);
        assert_eq!(compiled.modulation_plan.active_lfo_mask & 0b11, 0b11);

        let context = modulation_context(64);
        let actual = modulation_step_compiled(&mut compiled, context);
        let expected = modulation_step_reference(&mut reference, context);
        assert_lanes_equal(compiled.last_lfo_outputs[1], reference.last_lfo_outputs[1]);
        assert_modulation_equal(&actual, &expected);
    }

    #[test]
    fn single_pwm_fast_path_requires_exactly_one_lfo_to_osc1_shape_route() {
        let mut block = VoiceBlock::new(48_000.0, &Patch::default());
        block.set_lfo_depth(0, 1.0);
        block.set_mod_route(
            ModRoute::Free(0),
            true,
            ModSource::Lfo1,
            ModDestination::Osc1ShapeMod,
            0.49,
        );
        let route = block
            .modulation_plan
            .single_pwm_route
            .expect("single PWM route should compile to the fast path");
        assert_eq!(route.lfo_index, 0);
        assert_eq!(route.amount.to_bits(), 0.49f32.to_bits());

        block.set_mod_route(
            ModRoute::Free(1),
            true,
            ModSource::ModWheel,
            ModDestination::FilterCutoff,
            0.5,
        );
        assert!(block.modulation_plan.single_pwm_route.is_none());
    }

    fn pan_lanes_reference(block: &VoiceBlock, lanes: f32x4, pan_mod: f32x4) -> (f32, f32) {
        let voice_position = f32x4::new(block.pan_positions);
        let position = match block.pan_mod_mode {
            PanModMode::Alternate => {
                let spread = (f32x4::splat(block.pan_spread) + pan_mod)
                    .clamp(f32x4::ZERO, f32x4::splat(1.0));
                voice_position * spread
            }
            PanModMode::Fixed => (voice_position * f32x4::splat(block.pan_spread) + pan_mod)
                .clamp(f32x4::splat(-1.0), f32x4::splat(1.0)),
        };
        let angle = (position + f32x4::splat(1.0)) * f32x4::splat(core::f32::consts::FRAC_PI_4);
        let (sin, cos) = angle.sin_cos();

        ((lanes * cos).reduce_add(), (lanes * sin).reduce_add())
    }

    #[test]
    fn pan_lanes_matches_rev2_mode_equations() {
        let lanes = f32x4::new([0.75, -0.25, 0.125, -0.0625]);
        let mut block = VoiceBlock::new(44_100.0, &Patch::default());

        block.set_pan_spread(1.0);
        block.note_on(0, 60, 1.0, false);
        let expected = pan_lanes_reference(&block, lanes, f32x4::splat(0.75));
        let actual = block.pan_lanes(lanes, f32x4::splat(0.75));
        assert_eq!(actual.0.to_bits(), expected.0.to_bits());
        assert_eq!(actual.1.to_bits(), expected.1.to_bits());

        block.set_pan_mod_mode(PanModMode::Fixed);
        let expected = pan_lanes_reference(&block, lanes, f32x4::splat(-0.25));
        let actual = block.pan_lanes(lanes, f32x4::splat(-0.25));
        assert_eq!(actual.0.to_bits(), expected.0.to_bits());
        assert_eq!(actual.1.to_bits(), expected.1.to_bits());
    }

    #[test]
    fn a_single_voice_keeps_its_physical_pan_position() {
        let lanes = f32x4::new([1.0, 0.0, 0.0, 0.0]);
        let mut block = VoiceBlock::new(44_100.0, &Patch::default());
        block.set_pan_spread(1.0);

        let (left, right) = block.pan_lanes(lanes, f32x4::ZERO);

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
    fn alternate_modulation_changes_width_and_fixed_modulation_translates() {
        let lanes = f32x4::new([1.0, 1.0, 0.0, 0.0]);
        let mut block = VoiceBlock::new(44_100.0, &Patch::default());
        block.set_pan_spread(0.5);

        let alternate = block.pan_lanes(lanes, f32x4::splat(0.25));
        assert!((alternate.0 - alternate.1).abs() < 1.0e-6);

        block.set_pan_mod_mode(PanModMode::Fixed);
        let fixed = block.pan_lanes(lanes, f32x4::splat(0.25));
        assert!(
            fixed.1 > fixed.0,
            "positive Fixed modulation should move the program right"
        );
    }

    #[test]
    fn matrix_route_can_modulate_lfo_frequency_before_lfo_outputs_are_used() {
        fn lfo2_peak(matrix_enabled: bool) -> f32 {
            let mut block = VoiceBlock::new(100.0, &Patch::default());
            block.set_lfo_rate_hz(0, 1.0);
            block.set_lfo_depth(0, 1.0);
            block.set_lfo_waveform(0, LfoWaveform::Square);
            block.set_lfo_rate_hz(1, 0.1);
            block.set_lfo_depth(1, 1.0);
            block.set_lfo_waveform(1, LfoWaveform::Saw);
            block.set_mod_route(
                ModRoute::Free(0),
                matrix_enabled,
                ModSource::Lfo1,
                ModDestination::Lfo2Frequency,
                1.0,
            );

            let mut peak = 0.0f32;
            for _ in 0..64 {
                voice_block_next(&mut block);
                peak = peak.max(block.last_lfo_outputs[1].to_array()[0]);
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
    fn aux_envelope_route_can_modulate_lfo_frequency_as_internal_route() {
        fn lfo2_peak(aux_enabled: bool) -> f32 {
            let mut block = VoiceBlock::new(100.0, &Patch::default());
            block.set_lfo_rate_hz(1, 0.1);
            block.set_lfo_depth(1, 1.0);
            block.set_lfo_waveform(1, LfoWaveform::Saw);
            block.set_aux_destination(if aux_enabled {
                ModDestination::Lfo2Frequency
            } else {
                ModDestination::Off
            });
            block.set_aux_amount(1.0);
            block.set_aux_attack(0.0005);
            block.set_aux_decay(5.0);
            block.set_aux_sustain(1.0);
            block.note_on(0, 60, 1.0, false);

            let mut peak = 0.0f32;
            for _ in 0..64 {
                voice_block_next(&mut block);
                peak = peak.max(block.last_lfo_outputs[1].to_array()[0]);
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        for _ in 0..frames {
            let (left, right) = voices.next();
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let frames = 4096;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();

        assert!(
            right > left * 100.0,
            "physical voice 1 should pan right at full spread, left {left}, right {right}"
        );
    }

    #[test]
    fn pan_lfo_modulates_spread_width_instead_of_offset() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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

        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let frames = 4096;
        for _ in 0..frames {
            let (left, right) = voices.next();
            left_sum += left * left;
            right_sum += right * right;
        }
        let left = (left_sum / frames as f32).sqrt();
        let right = (right_sum / frames as f32).sqrt();

        assert!(
            (left - right).abs() < left.max(right) * 0.1,
            "positive pan modulation should widen alternating voices symmetrically, left {left}, right {right}"
        );
    }

    #[test]
    fn repeated_notes_advance_through_physical_voice_pan_positions() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 72.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.gates.iter().any(|gate| *gate))
            .unwrap();
        let lane = block.gates.iter().position(|gate| *gate).unwrap();
        let expected = crate::midi_to_hz(76);
        let osc1_freq = block.oscillators.osc1_frequency_hz().to_array()[lane];
        assert_eq!(block.notes[lane], 64);
        assert!(
            (osc1_freq - expected).abs() < 0.1,
            "osc1 should track MIDI note + tuning offset, got {} expected {expected}",
            osc1_freq
        );
    }

    #[test]
    fn oscillator_frequency_and_fine_tune_use_natural_units_and_clamp() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Frequency, 240.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1FineTune, 99.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let block = voices
            .iter()
            .find(|block| block.gates.iter().any(|gate| *gate))
            .unwrap();
        let lane = block.gates.iter().position(|gate| *gate).unwrap();
        let expected = crate::midi_to_hz(120) * 2.0f32.powf(50.0 / 1200.0);
        let osc1_freq = block.oscillators.osc1_frequency_hz().to_array()[lane];

        assert!(
            (osc1_freq - expected).abs() < 0.5,
            "osc1 frequency should clamp to 120 semitones and +50 cents, got {osc1_freq}, expected {expected}"
        );
    }

    #[test]
    fn osc_mix_is_canonical_balance_control() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2Enabled, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::OscMix, 0.25));

        let params = voices[0].oscillators.params();
        assert_eq!(params.osc_mix, 0.25);
    }

    #[test]
    fn osc_slop_zero_is_stable_and_full_slop_offsets_lanes() {
        let mut stable = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72] {
            stable.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let stable_block = &stable[0];
        for lane in 0..LANES {
            let expected = crate::midi_to_hz(stable_block.notes[lane]);
            let freq = stable_block.oscillators.osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "slop 0 should not detune lane {lane}, got {freq}, expected {expected}"
            );
        }

        let mut sloppy = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        sloppy.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            sloppy.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        let sloppy_block = &sloppy[0];
        let offsets: [f32; LANES] = core::array::from_fn(|lane| {
            let expected = crate::midi_to_hz(sloppy_block.notes[lane]);
            sloppy_block.oscillators.osc1_frequency_hz().to_array()[lane] - expected
        });

        assert!(
            offsets.iter().any(|offset| offset.abs() > 0.01),
            "full slop should offset at least one lane, offsets {offsets:?}"
        );
    }

    #[test]
    fn clearing_osc_slop_restores_intended_frequency() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 1.0));
        for note in [60, 64, 67, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        voices.handle_control(ControlMessage::SetParam(ParamId::OscSlop, 0.0));
        let block = &voices[0];
        for lane in 0..LANES {
            let expected = crate::midi_to_hz(block.notes[lane]);
            let freq = block.oscillators.osc1_frequency_hz().to_array()[lane];
            assert!(
                (freq - expected).abs() < 0.1,
                "clearing slop should restore lane {lane}, got {freq}, expected {expected}"
            );
        }
    }

    #[test]
    fn note_reset_flags_are_routed_to_oscillators() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc1NoteReset, 0.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Osc2NoteReset, 1.0));

        let params = voices[0].oscillators.params();
        assert!(!params.osc1.note_reset);
        assert!(params.osc2.note_reset);
    }

    #[test]
    fn hard_sync_param_is_routed_to_oscillators() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::HardSync, 1.0));

        assert!(voices[0].oscillators.params().sync);
    }

    #[test]
    fn aux_envelope_to_oscillator_frequency_modulates_pitch() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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

        process_frames(&mut voices, 32);

        let block = &voices[0];
        let freq = block.oscillators.osc1_frequency_hz().to_array()[0];
        let expected = crate::midi_to_hz(72);
        assert!(
            (freq - expected).abs() < 1.0,
            "full positive aux pitch modulation should raise osc1 by about one octave, got {freq}, expected {expected}"
        );
    }

    #[test]
    fn aux_repeat_keeps_envelope_cycling_while_held() {
        let mut repeating = Voices::<{ crate::VOICE_PACKS }>::new(1_000.0);
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

        let first = repeating[0].aux_env.next().to_array()[0];
        let reset = repeating[0].aux_env.next().to_array()[0];
        let second = repeating[0].aux_env.next().to_array()[0];

        assert!(first > 0.9);
        assert_eq!(reset, 0.0);
        assert!(second > 0.9);

        repeating.handle_control(ControlMessage::NoteOff { note: 60 });
        assert_eq!(repeating[0].aux_env.next().to_array()[0], 0.0);
        assert!(repeating[0].aux_env.is_idle_lane(0));
    }

    #[test]
    fn vca_initial_level_at_one_ignores_amp_envelope_amount() {
        let mut drone = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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

        let mut gated = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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

        let mut enveloped = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
}

