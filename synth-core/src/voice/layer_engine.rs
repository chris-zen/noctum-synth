//! Polyphonic voice allocation and mixing.

#[cfg(test)]
use crate::dsp::filter::MAX_CUTOFF_HZ;
use crate::{
    ChordMemory, ClockDivision, ControlMessage, GatedSequencerMode, GlideMode, KeyMode, LayerPatch,
    ModDestination, ParamId, PolyStep, SequenceUpdate, SequencerRecordCommand, SequencerType,
    UnisonMode, VOICE_PACKS,
    arp::{ArpEngine, ArpEvent},
    dsp::{
        DEFAULT_PARAMETER_SMOOTHING_SECONDS, FilterOversampling, FilterType,
        parameter_smoother::ParameterSmoother,
    },
    effects::{EffectModulation, EffectsState},
    fixed_index_list::FixedIndexList,
    math::{F32, WideF32},
    midi::prophet::cutoff_raw_to_hz,
    pressed_keys::PressedKeys,
    profiling::RenderContext,
    sequencer::model::SequencerTransportCommand,
    sequencer::{
        gated::{GatedEvent, GatedSequencer},
        poly::{PolyEvent, PolySequencer, PolyStepEvent},
        recorder::{RecorderEvent, StepRecorder},
    },
    voice::{
        IdleAdvance, NoteGlide, PatchModulation, PerformanceModulation, VoiceBlock,
        voice_pan_position,
    },
};

const MIDI_CC_FILTER_RESONANCE: u8 = 71;
const MIDI_CC_FILTER_CUTOFF: u8 = 74;

/// Provisional guide-level detune curve, isolated for future Rev2 calibration.
pub fn unison_detune_cents(voice_index: usize, voice_count: usize, amount: f32) -> f32 {
    if voice_count <= 1 {
        return 0.0;
    }
    let position = 2.0 * voice_index as f32 / (voice_count - 1) as f32 - 1.0;
    position * amount.clamp(0.0, 16.0)
}

/// Physical SIMD voice storage shared by logical layer engines.
pub struct VoicePool<const PACKS: usize = VOICE_PACKS> {
    blocks: [VoiceBlock; PACKS],
    sample_rate: f32,
}

impl<const PACKS: usize> VoicePool<PACKS> {
    const VALID_PACK_COUNT: () = assert!(PACKS > 0, "voice pool must contain at least one pack");

    /// Creates physical voice storage for a non-zero compile-time pack count.
    ///
    /// ```compile_fail
    /// use synth_core::voice::VoicePool;
    ///
    /// let _ = VoicePool::<0>::new(48_000.0);
    /// ```
    pub fn new(sample_rate: f32) -> Self {
        let () = Self::VALID_PACK_COUNT;
        let mut blocks = Self::new_blocks(sample_rate);
        for block in &mut blocks {
            block.refresh_lfo_engines();
        }
        Self {
            blocks,
            sample_rate,
        }
    }

    fn new_blocks(sample_rate: f32) -> [VoiceBlock; PACKS] {
        let patch = LayerPatch::default();
        core::array::from_fn(|block_index| {
            let mut block = VoiceBlock::new(sample_rate);
            block.apply_voice_patch(&patch);
            block.set_pan_positions(core::array::from_fn(|lane| {
                voice_pan_position(block_index * WideF32::LANES + lane, PACKS * WideF32::LANES)
            }));
            block
        })
    }

    pub(crate) fn reset(&mut self) {
        let patch = LayerPatch::default();
        let voice_count = PACKS * WideF32::LANES;
        for (block_index, block) in self.blocks.iter_mut().enumerate() {
            let mut replacement = VoiceBlock::new(self.sample_rate);
            replacement.apply_voice_patch(&patch);
            replacement.set_pan_positions(core::array::from_fn(|lane| {
                voice_pan_position(block_index * WideF32::LANES + lane, voice_count)
            }));
            *block = replacement;
        }
    }

    /// Stops current notes, then reconstructs every research oscillator source.
    #[cfg(feature = "experimental-oscillators")]
    pub fn set_experimental_oscillator_model(
        &mut self,
        model: crate::dsp::ExperimentalOscillatorModel,
    ) {
        for block in &mut self.blocks {
            block.all_notes_off();
            block.set_experimental_oscillator_model(model);
        }
    }

    #[cfg(feature = "experimental-oscillators")]
    pub fn set_measured_wavetable_bank(&mut self, bank: crate::dsp::MeasuredWavetableBank) {
        for block in &mut self.blocks {
            block.set_measured_wavetable_bank(bank);
        }
    }

    fn set_region_pan_positions(&mut self, region: VoiceRegion) {
        let voice_count = region.voice_capacity();
        for (block_index, block) in self.region_mut(region).iter_mut().enumerate() {
            block.set_pan_positions(core::array::from_fn(|lane| {
                voice_pan_position(block_index * WideF32::LANES + lane, voice_count)
            }));
        }
    }

    fn region(&self, region: VoiceRegion) -> &[VoiceBlock] {
        &self.blocks[region.start_pack..region.end_pack()]
    }

    fn region_mut(&mut self, region: VoiceRegion) -> &mut [VoiceBlock] {
        &mut self.blocks[region.start_pack..region.end_pack()]
    }
}

/// A block-aligned subregion of a [`VoicePool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceRegion {
    start_pack: usize,
    pack_count: usize,
}

impl VoiceRegion {
    pub(crate) const fn all<const PACKS: usize>() -> Self {
        Self {
            start_pack: 0,
            pack_count: PACKS,
        }
    }

    pub(crate) const fn from_packs(start_pack: usize, pack_count: usize) -> Self {
        Self {
            start_pack,
            pack_count,
        }
    }

    pub const fn start_pack(self) -> usize {
        self.start_pack
    }

    pub const fn pack_count(self) -> usize {
        self.pack_count
    }

    pub const fn voice_capacity(self) -> usize {
        self.pack_count * WideF32::LANES
    }

    fn end_pack(self) -> usize {
        self.start_pack + self.pack_count
    }
}

/// Logical state and processing for one synth layer.
pub struct LayerEngine<const PACKS: usize = VOICE_PACKS> {
    region: VoiceRegion,
    allocated: AllocatedVoices<PACKS>,
    pressed_keys: PressedKeys,
    last_played_note: Option<u8>,
    glide: GlideSettings,
    unison: UnisonSettings,
    key_mode: KeyMode,
    performance: PerformanceModulation,
    modulation: PatchModulation,
    last_effect_modulation: EffectModulation,
    arp: ArpEngine,
    gated: GatedSequencer,
    poly: PolySequencer,
    recorder: StepRecorder,
    effects: EffectsState,
    local_tempo_bpm: f32,
    tempo_bpm: f32,
    clock_division: ClockDivision,
    program_volume: ParameterSmoother,
}

impl<const PACKS: usize> LayerEngine<PACKS> {
    const _VALID_PACK_COUNT: () = assert!(PACKS > 0, "layer must contain at least one voice pack");

    pub fn new(sample_rate: f32) -> Self {
        let () = Self::_VALID_PACK_COUNT;
        let patch = LayerPatch::default();
        let mut effects = EffectsState::new(sample_rate);
        effects.set_tempo_bpm(crate::DEFAULT_TEMPO_BPM);
        Self {
            region: VoiceRegion::all::<PACKS>(),
            allocated: AllocatedVoices::new(),
            pressed_keys: PressedKeys::default(),
            last_played_note: None,
            glide: GlideSettings::default(),
            unison: UnisonSettings::default(),
            key_mode: KeyMode::default(),
            performance: PerformanceModulation::default(),
            modulation: PatchModulation::new(&patch),
            last_effect_modulation: EffectModulation::default(),
            arp: ArpEngine::new(sample_rate),
            gated: GatedSequencer::new(sample_rate),
            poly: PolySequencer::new(sample_rate),
            recorder: StepRecorder::default(),
            effects,
            local_tempo_bpm: crate::DEFAULT_TEMPO_BPM,
            tempo_bpm: crate::DEFAULT_TEMPO_BPM,
            clock_division: ClockDivision::default(),
            program_volume: ParameterSmoother::new(
                0.8,
                sample_rate,
                DEFAULT_PARAMETER_SMOOTHING_SECONDS,
            ),
        }
    }

    fn sync_lfo_engines(&mut self, pool: &mut VoicePool<PACKS>) {
        for block in pool.region_mut(self.region) {
            block.refresh_lfo_engines();
        }
    }

    pub(crate) fn apply_patch(&mut self, pool: &mut VoicePool<PACKS>, patch: &LayerPatch) {
        let was_poly_active = self.poly.is_active();
        self.recorder.cancel_pending();
        let release = self.poly.apply_sequence(&patch.sequence);
        self.release_sequence_lanes(pool, release);
        self.modulation.apply_patch(patch);
        pool.set_region_pan_positions(self.region);
        for block in pool.region_mut(self.region) {
            block.set_tempo_bpm(patch.bpm);
            block.set_clock_division(patch.clock_divide);
            block.apply_voice_patch(patch);
        }
        self.sync_lfo_engines(pool);
        self.key_mode = patch.key_mode;
        self.glide.enabled = patch.glide_enabled;
        self.glide.mode = patch.glide_mode;
        self.unison.enabled = patch.unison_enabled;
        self.unison.mode = patch.unison_mode;
        self.unison.detune = patch.unison_detune.clamp(0.0, 16.0);
        self.unison.chord = patch.unison_chord;
        self.arp.set_params(&patch.arp);
        self.gated.apply_sequence(&patch.sequence);
        self.performance.sequence = self.gated.outputs();
        self.gated.set_tempo_bpm(patch.bpm);
        self.gated.set_clock_division(patch.clock_divide);
        self.poly.set_tempo_bpm(patch.bpm);
        self.poly.set_clock_division(patch.clock_divide);
        self.effects.set_params(patch.effects);
        self.local_tempo_bpm = patch.bpm.clamp(30.0, 250.0);
        self.clock_division = patch.clock_divide;
        self.program_volume
            .snap(patch.program_volume.clamp(0.0, 1.0));
        self.rebuild_sounding_notes(pool);
        self.update_poly_routing_transition(pool, was_poly_active);
    }

    pub(crate) fn assign_region(&mut self, region: VoiceRegion) {
        self.region = region;
    }

    pub(crate) fn clear_note_state(&mut self) {
        self.allocated.clear_occupancy();
        self.allocated.sustain_pressed = false;
        self.pressed_keys.clear();
        self.last_played_note = None;
        self.arp.all_notes_off();
        self.arp.set_sustain_pedal(false);
        self.gated.note_off_all();
        let _ = self.poly.stop();
        self.recorder.cancel_pending();
    }

    pub(crate) fn stop_recording(&mut self) -> bool {
        if !self.recorder.is_recording() {
            self.recorder.cancel_pending();
            return false;
        }
        self.recorder.command(SequencerRecordCommand::Stop);
        true
    }

    fn voice_count(&self) -> usize {
        self.region.voice_capacity()
    }

    pub fn handle_control(&mut self, pool: &mut VoicePool<PACKS>, msg: ControlMessage) {
        match msg {
            ControlMessage::NoteOn { note, velocity } => {
                if velocity <= 0.0 {
                    self.handle_note_off(pool, note);
                    return;
                }
                self.handle_note_on(pool, note, velocity.clamp(0.0, 1.0));
            }
            ControlMessage::NoteOff { note } => {
                self.handle_note_off(pool, note);
            }
            ControlMessage::AllNotesOff => {
                for block in pool.region_mut(self.region) {
                    block.all_notes_off();
                }
                self.allocated.clear_occupancy();
                self.pressed_keys.clear();
                self.arp.all_notes_off();
                self.gated.note_off_all();
                let _ = self.poly.stop();
                self.recorder.cancel_pending();
            }
            ControlMessage::SetUnisonChord { chord, .. } => {
                self.unison.chord = chord;
                if self.unison.enabled && self.unison.mode == UnisonMode::Chord {
                    self.rebuild_sounding_notes(pool);
                }
            }
            ControlMessage::SetParam { param, value, .. } => self.set_param(pool, param, value),
            ControlMessage::SetFilterType(filter_type) => self.set_filter_type(pool, filter_type),
            ControlMessage::SetMidiClockMode(_)
            | ControlMessage::SetMasterVolume(_)
            | ControlMessage::MidiRealtime(_) => {}
            #[cfg(feature = "experimental-oscillators")]
            ControlMessage::SetExperimentalOscillatorModel(_) => {}
            ControlMessage::SetModulation {
                route,
                enabled,
                source,
                destination,
                amount,
                ..
            } => {
                self.modulation
                    .set_mod_route(route, enabled, source, destination, amount);
                self.sync_lfo_engines(pool);
            }
            ControlMessage::SetModulationParam {
                route, parameter, ..
            } => {
                self.modulation.set_mod_route_param(route, parameter);
                self.sync_lfo_engines(pool);
            }
            ControlMessage::SetSequence { update, .. } => {
                self.apply_sequence_update(pool, update);
            }
            ControlMessage::SetSequencerTransport { command, .. } => {
                self.set_poly_transport(pool, command);
            }
            ControlMessage::SetSequencerRunning { running, .. } => {
                self.set_poly_transport(
                    pool,
                    if running {
                        SequencerTransportCommand::Start
                    } else {
                        SequencerTransportCommand::Stop
                    },
                );
            }
            ControlMessage::SequencerRecord { command, .. } => {
                if self.poly.is_selected() || command == SequencerRecordCommand::Stop {
                    self.recorder.command(command);
                }
            }
            ControlMessage::PitchBend { value } => {
                self.performance.pitch_bend = value.clamp(-1.0, 1.0);
            }
            ControlMessage::ModWheel { value } => {
                self.performance.mod_wheel = value.clamp(0.0, 1.0);
            }
            ControlMessage::Pressure { value } => {
                self.performance.pressure = value.clamp(0.0, 1.0);
            }
            ControlMessage::SustainPedal { pressed } => {
                self.set_sustain_pedal(pool, pressed);
            }
            ControlMessage::ControlChange { controller, value } => {
                let value = value.clamp(0.0, 1.0);
                match controller {
                    2 => self.performance.breath = value,
                    4 => self.performance.foot = value,
                    11 => self.performance.expression = value,
                    MIDI_CC_FILTER_RESONANCE => {
                        for block in pool.region_mut(self.region) {
                            block.set_filter_resonance(value);
                        }
                    }
                    MIDI_CC_FILTER_CUTOFF => {
                        let cutoff = midi_filter_cutoff_hz(value);
                        for block in pool.region_mut(self.region) {
                            block.set_filter_cutoff(cutoff);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_note_on(&mut self, pool: &mut VoicePool<PACKS>, note: u8, velocity: f32) {
        if self.poly.is_active() && self.recorder.is_recording() {
            // Rev2 live notes play alongside a running poly sequence.
            // Transposition is the explicit Record+key gesture.
            self.poly.set_transpose_note(note);
        } else {
            self.recorder.note_on(note, velocity);
        }
        self.restore_note_on(pool, note, velocity);
    }

    pub(crate) fn restore_note_on(&mut self, pool: &mut VoicePool<PACKS>, note: u8, velocity: f32) {
        let first_held_note = self.pressed_keys.is_empty();
        self.handle_note_on_voice(pool, note, velocity);
        if let Some(event) = self.gated.note_on(first_held_note) {
            self.handle_gated_event(pool, event, false);
        }
    }

    fn handle_note_on_voice(&mut self, pool: &mut VoicePool<PACKS>, note: u8, velocity: f32) {
        if note >= 128 {
            return;
        }
        if self.arp.params().enabled && !self.poly.is_active() {
            let was_empty = self.pressed_keys.is_empty();
            self.pressed_keys.press(note, velocity);
            self.last_played_note = Some(note);
            if was_empty {
                self.reset_key_synced_lfos(pool);
            }
            self.arp.note_on(note, velocity);
            return;
        }
        let is_first_key = self.pressed_keys.is_empty();
        let glide_start = self.last_played_note;
        let should_glide = self.glide.enabled
            && glide_start.is_some()
            && (!self.glide.mode.is_auto() || !is_first_key);
        self.pressed_keys.press(note, velocity);
        self.last_played_note = Some(note);
        if is_first_key {
            self.reset_key_synced_lfos(pool);
        }
        if !self.unison.enabled {
            self.allocated.poly_note_on(
                pool.region_mut(self.region),
                note,
                velocity,
                glide_start,
                should_glide,
            );
            return;
        }
        let Some((selected, selected_velocity)) = self.pressed_keys.selected(self.key_mode) else {
            return;
        };
        self.update_unison_group(
            pool,
            selected,
            selected_velocity,
            is_first_key || key_mode_retriggers(self.key_mode),
            glide_start,
            should_glide,
        );
    }

    fn handle_note_off(&mut self, pool: &mut VoicePool<PACKS>, note: u8) {
        self.recorder.note_off(note);
        self.handle_note_off_voice(pool, note);
        if self.pressed_keys.is_empty() {
            self.gated.note_off_all();
        }
    }

    fn handle_note_off_voice(&mut self, pool: &mut VoicePool<PACKS>, note: u8) {
        if note >= 128 {
            return;
        }
        self.pressed_keys.release(note);
        if self.arp.params().enabled && !self.poly.is_active() {
            self.arp.note_off(note);
            return;
        }
        if !self.unison.enabled {
            self.allocated
                .poly_note_off(pool.region_mut(self.region), note);
            return;
        }
        if let Some((selected, velocity)) = self.pressed_keys.selected(self.key_mode) {
            self.update_unison_group(pool, selected, velocity, false, None, self.glide.enabled);
        } else if !self.allocated.sustain_pressed {
            self.release_unison_group(pool);
        }
    }

    fn unison_targets(&self, root: u8) -> ([u8; 16], usize) {
        let mut targets = [root; 16];
        if let Some(count) = self.unison.mode.voice_count() {
            return (targets, count.min(self.voice_count()).min(targets.len()));
        }

        let mut len = 0;
        let intervals = self.unison.chord.intervals();
        if intervals.is_empty() {
            targets[0] = root;
            return (targets, 1);
        }
        for interval in intervals.iter().copied() {
            let Some(note) = root.checked_add(interval) else {
                continue;
            };
            if note < 128 && len < self.voice_count().min(targets.len()) {
                targets[len] = note;
                len += 1;
            }
        }
        if len == 0 {
            targets[0] = root;
            len = 1;
        }
        (targets, len)
    }

    fn update_unison_group(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        root: u8,
        velocity: f32,
        retrigger: bool,
        glide_start: Option<u8>,
        should_glide: bool,
    ) {
        let (targets, target_len) = self.unison_targets(root);
        let voice_count = self.voice_count();
        let blocks = pool.region_mut(self.region);

        for voice_idx in target_len..voice_count {
            if !self.allocated.held.contains(voice_idx) {
                continue;
            }
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = AllocatedVoices::<PACKS>::voice_location(voice_idx);
            blocks[block_idx].note_off_lane(lane);
            self.allocated.held.remove(voice_idx);
            self.allocated.sustained.remove(voice_idx);
        }

        for (voice_idx, note) in targets[..target_len].iter().copied().enumerate() {
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = AllocatedVoices::<PACKS>::voice_location(voice_idx);
            self.allocated.sustained.remove(voice_idx);
            let was_active = self.allocated.held.contains(voice_idx)
                && blocks[block_idx].active_note(lane).is_some();
            let lane_glide_start = if was_active { None } else { glide_start };
            let tuning_cents = core::array::from_fn(|block_lane| {
                let index = block_idx * WideF32::LANES + block_lane;
                if index < target_len {
                    unison_detune_cents(index, target_len, self.unison.detune)
                } else {
                    0.0
                }
            });
            let block = &mut blocks[block_idx];
            if was_active {
                if retrigger {
                    // This lane already belongs to the unison group. Retrigger it
                    // in place instead of treating the transition as a voice
                    // steal: the latter inserts a 5 ms shutdown and allows rapid
                    // key transitions to overwrite the pending performance note.
                    block.retrigger_sounding_with_glide(
                        lane,
                        note,
                        velocity,
                        tuning_cents,
                        should_glide,
                    );
                } else {
                    block.retune_lane(lane, note, velocity, tuning_cents, should_glide);
                }
            } else if block.is_lane_silent(lane) {
                block.note_on_tuned_with_glide(
                    lane,
                    note,
                    velocity,
                    false,
                    tuning_cents,
                    NoteGlide {
                        start_note: lane_glide_start,
                        enabled: should_glide,
                    },
                );
            } else {
                block.schedule_note_on_tuned_with_glide(
                    lane,
                    note,
                    velocity,
                    false,
                    tuning_cents,
                    NoteGlide {
                        start_note: lane_glide_start,
                        enabled: should_glide,
                    },
                );
            }
            self.allocated.mark_held(voice_idx);
        }
    }

    fn refresh_unison_detune(&mut self, pool: &mut VoicePool<PACKS>) {
        let count = (0..self.voice_count())
            .take_while(|voice_idx| self.allocated.held.contains(*voice_idx))
            .count();
        for (block_idx, block) in pool.region_mut(self.region).iter_mut().enumerate() {
            let tuning_cents = core::array::from_fn(|lane| {
                let voice_idx = block_idx * WideF32::LANES + lane;
                if voice_idx < count {
                    unison_detune_cents(voice_idx, count, self.unison.detune)
                } else {
                    0.0
                }
            });
            block.set_tuning_cents(tuning_cents);
        }
    }

    fn release_unison_group(&mut self, pool: &mut VoicePool<PACKS>) {
        let blocks = pool.region_mut(self.region);
        for voice_idx in 0..self.region.voice_capacity() {
            if !self.allocated.held.contains(voice_idx) {
                continue;
            }
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = AllocatedVoices::<PACKS>::voice_location(voice_idx);
            blocks[block_idx].note_off_lane(lane);
            self.allocated.held.remove(voice_idx);
            self.allocated.sustained.remove(voice_idx);
        }
    }

    fn reset_key_synced_lfos(&mut self, pool: &mut VoicePool<PACKS>) {
        for block in pool.region_mut(self.region) {
            block.reset_key_synced_lfos();
        }
    }

    fn rebuild_sounding_notes(&mut self, pool: &mut VoicePool<PACKS>) {
        for block in pool.region_mut(self.region) {
            block.all_notes_off();
        }
        self.allocated.clear_occupancy();

        if self.arp.params().enabled {
            return;
        }
        if self.unison.enabled {
            if let Some((note, velocity)) = self.pressed_keys.selected(self.key_mode) {
                self.update_unison_group(pool, note, velocity, true, None, false);
            }
        } else {
            let pressed = self.pressed_keys.clone();
            for (note, velocity) in pressed.iter() {
                self.allocated.poly_note_on(
                    pool.region_mut(self.region),
                    note,
                    velocity,
                    None,
                    false,
                );
            }
        }
    }

    fn set_sustain_pedal(&mut self, pool: &mut VoicePool<PACKS>, pressed: bool) {
        if self.arp.params().enabled {
            self.arp.set_sustain_pedal(pressed);
            if !self.arp.sustain_forward() {
                return;
            }
        }
        if self.unison.enabled {
            if self.allocated.sustain_pressed && !pressed && self.pressed_keys.is_empty() {
                self.release_unison_group(pool);
            }
            self.allocated.sustain_pressed = pressed;
            return;
        }
        self.allocated
            .set_sustain_pedal(pool.region_mut(self.region), pressed);
    }

    fn set_param(&mut self, pool: &mut VoicePool<PACKS>, id: ParamId, value: f32) {
        match id {
            ParamId::UnisonEnabled => {
                let enabled = value >= 0.5;
                if enabled != self.unison.enabled {
                    self.unison.enabled = enabled;
                    self.rebuild_sounding_notes(pool);
                }
                return;
            }
            ParamId::UnisonMode => {
                let mode = UnisonMode::from_index(value as usize);
                if mode != self.unison.mode {
                    self.unison.mode = mode;
                    if self.unison.enabled {
                        self.rebuild_sounding_notes(pool);
                    }
                }
                return;
            }
            ParamId::UnisonDetune => {
                self.unison.detune = value.clamp(0.0, 16.0);
                if self.unison.enabled {
                    self.refresh_unison_detune(pool);
                }
                return;
            }
            ParamId::KeyMode => {
                self.key_mode = KeyMode::from_index(value as usize);
                if self.unison.enabled {
                    if let Some((note, velocity)) = self.pressed_keys.selected(self.key_mode) {
                        self.update_unison_group(pool, note, velocity, false, None, false);
                    }
                }
                return;
            }
            ParamId::GlideEnabled => self.glide.enabled = value >= 0.5,
            ParamId::GlideMode => self.glide.mode = GlideMode::from_index(value as usize),
            ParamId::Lfo1Depth => self.modulation.set_lfo_depth(0, value),
            ParamId::Lfo2Depth => self.modulation.set_lfo_depth(1, value),
            ParamId::Lfo3Depth => self.modulation.set_lfo_depth(2, value),
            ParamId::Lfo4Depth => self.modulation.set_lfo_depth(3, value),
            ParamId::Lfo1Destination => self
                .modulation
                .set_lfo_destination(0, ModDestination::from_index(value as usize)),
            ParamId::Lfo2Destination => self
                .modulation
                .set_lfo_destination(1, ModDestination::from_index(value as usize)),
            ParamId::Lfo3Destination => self
                .modulation
                .set_lfo_destination(2, ModDestination::from_index(value as usize)),
            ParamId::Lfo4Destination => self
                .modulation
                .set_lfo_destination(3, ModDestination::from_index(value as usize)),
            ParamId::AuxEgDestination => self
                .modulation
                .set_aux_destination(ModDestination::from_index(value as usize)),
            ParamId::AuxEgAmount => self.modulation.set_aux_amount(value),
            ParamId::ArpEnabled => {
                let became_enabled = value >= 0.5 && !self.arp.params().enabled;
                self.arp.set_enabled(value >= 0.5);
                if became_enabled {
                    // Repopulate arp held_notes from currently pressed keys, since
                    // the arp's clear() on disable wiped its internal state.
                    for (note, velocity) in self.pressed_keys.iter() {
                        self.arp.note_on(note, velocity);
                    }
                }
                self.rebuild_sounding_notes(pool);
                return;
            }
            ParamId::ArpMode => {
                self.arp.set_params(&crate::patch::ArpParams {
                    mode: crate::patch::ArpMode::from_index(value as usize),
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpRange => {
                self.arp.set_params(&crate::patch::ArpParams {
                    range: (value as u8).clamp(0, 2) + 1,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpRepeats => {
                self.arp.set_params(&crate::patch::ArpParams {
                    repeats: (value as u8).clamp(0, 2) + 1,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpRelatch => {
                self.arp.set_params(&crate::patch::ArpParams {
                    relatch: value >= 0.5,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpHold => {
                self.arp.set_params(&crate::patch::ArpParams {
                    hold: value >= 0.5,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpBeatSync => {
                self.arp.set_params(&crate::patch::ArpParams {
                    beat_sync: value >= 0.5,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::ArpSustainMode => {
                let mode = match value as usize {
                    0 => crate::patch::ArpSustainMode::ArpHold,
                    2 => crate::patch::ArpSustainMode::ArpHoldMom,
                    _ => crate::patch::ArpSustainMode::Sustain,
                };
                self.arp.set_params(&crate::patch::ArpParams {
                    sustain_mode: mode,
                    ..self.arp.params().clone()
                });
                return;
            }
            ParamId::SequencerType => {
                let update = SequenceUpdate::Type(SequencerType::from_index(value as usize));
                self.apply_sequence_update(pool, update);
                return;
            }
            ParamId::GatedSequencerMode => {
                self.gated
                    .apply_update(SequenceUpdate::GatedMode(GatedSequencerMode::from_index(
                        value as usize,
                    )));
                return;
            }
            _ => {}
        }
        for block in pool.region_mut(self.region) {
            block.set_param(id, value);
        }
        if matches!(
            id,
            ParamId::Lfo1Depth
                | ParamId::Lfo2Depth
                | ParamId::Lfo3Depth
                | ParamId::Lfo4Depth
                | ParamId::Lfo1Destination
                | ParamId::Lfo2Destination
                | ParamId::Lfo3Destination
                | ParamId::Lfo4Destination
        ) {
            self.sync_lfo_engines(pool);
        }
    }

    pub(crate) fn set_tempo_bpm(&mut self, pool: &mut VoicePool<PACKS>, bpm: f32) {
        self.tempo_bpm = bpm.clamp(30.0, 250.0);
        self.effects.set_tempo_bpm(self.tempo_bpm);
        for block in pool.region_mut(self.region) {
            block.set_tempo_bpm(self.tempo_bpm);
        }
        self.arp.set_tempo_bpm(self.tempo_bpm);
        self.gated.set_tempo_bpm(self.tempo_bpm);
        self.poly.set_tempo_bpm(self.tempo_bpm);
    }

    pub(crate) fn set_clock_division(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        division: ClockDivision,
    ) {
        self.clock_division = division;
        for block in pool.region_mut(self.region) {
            block.set_clock_division(division);
        }
        self.arp.set_clock_division(division);
        self.gated.set_clock_division(division);
        self.poly.set_clock_division(division);
    }

    pub(crate) fn set_external_clock(&mut self, external: bool) {
        self.arp.set_external_clock(external);
        self.gated.set_external_clock(external);
        self.poly.set_external_clock(external);
    }

    pub(crate) fn midi_clock_tick(&mut self) {
        self.arp.midi_clock_tick();
        self.gated.midi_clock_tick();
        self.poly.midi_clock_tick();
    }

    fn release_sequence_lanes(&mut self, pool: &mut VoicePool<PACKS>, lanes: [bool; 6]) {
        for (lane, release) in lanes.iter().copied().enumerate() {
            if release {
                self.allocated
                    .sequence_note_off(pool.region_mut(self.region), lane as u8);
            }
        }
    }

    fn apply_poly_step(&mut self, pool: &mut VoicePool<PACKS>, event: PolyStepEvent) {
        self.release_sequence_lanes(pool, event.note_off);
        for note in event.note_on.into_iter().flatten() {
            self.allocated.sequence_note_on(
                pool.region_mut(self.region),
                note.lane,
                note.note,
                note.velocity,
            );
        }
    }

    fn apply_sequence_update(&mut self, pool: &mut VoicePool<PACKS>, update: SequenceUpdate) {
        let was_poly_active = self.poly.is_active();
        let was_gated_envelope = self.gated.envelope_gating();
        self.gated.apply_update(update);
        if let SequenceUpdate::Type(sequencer_type) = update {
            self.performance.sequence = self.gated.outputs();
            self.modulation
                .set_gated_enabled(sequencer_type == SequencerType::Gated);
            self.sync_lfo_engines(pool);
        }
        let release = self.poly.apply_update(update);
        self.release_sequence_lanes(pool, release);
        self.update_poly_routing_transition(pool, was_poly_active);
        if matches!(update, SequenceUpdate::Type(SequencerType::Polyphonic)) && was_gated_envelope {
            self.rebuild_sounding_notes(pool);
        }
        if matches!(update, SequenceUpdate::Type(SequencerType::Gated)) {
            self.recorder.cancel_pending();
        }
        if let SequenceUpdate::GatedDestination { track, destination } = update {
            self.modulation
                .set_gated_destination(usize::from(track), destination);
            self.sync_lfo_engines(pool);
        }
    }

    fn set_poly_transport(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        command: SequencerTransportCommand,
    ) {
        match command {
            SequencerTransportCommand::Start => {
                if self.poly.is_active() {
                    let release = self.poly.stop();
                    self.release_sequence_lanes(pool, release);
                }
                if !self.poly.start() {
                    return;
                }
                self.enter_poly_routing(pool);
            }
            SequencerTransportCommand::Continue => {
                let was_active = self.poly.is_active();
                if !self.poly.continue_playback() {
                    return;
                }
                if !was_active {
                    self.enter_poly_routing(pool);
                }
            }
            SequencerTransportCommand::Stop => {
                let was_active = self.poly.is_active();
                let release = self.poly.stop();
                self.release_sequence_lanes(pool, release);
                if was_active {
                    self.restore_arp_routing(pool);
                }
            }
        }
    }

    fn enter_poly_routing(&mut self, pool: &mut VoicePool<PACKS>) {
        if !self.arp.params().enabled {
            return;
        }
        if let Some(note) = self.arp.current_note() {
            self.allocated
                .poly_note_off(pool.region_mut(self.region), note);
        }
        self.arp.all_notes_off();
        let pressed = self.pressed_keys.clone();
        for (note, velocity) in pressed.iter() {
            self.allocated
                .poly_note_on(pool.region_mut(self.region), note, velocity, None, false);
        }
    }

    fn restore_arp_routing(&mut self, pool: &mut VoicePool<PACKS>) {
        if !self.arp.params().enabled {
            return;
        }
        let pressed = self.pressed_keys.clone();
        for (note, _) in pressed.iter() {
            self.allocated
                .poly_note_off(pool.region_mut(self.region), note);
        }
        self.arp.all_notes_off();
        for (note, velocity) in pressed.iter() {
            self.arp.note_on(note, velocity);
        }
    }

    fn update_poly_routing_transition(&mut self, pool: &mut VoicePool<PACKS>, was_active: bool) {
        match (was_active, self.poly.is_active()) {
            (true, false) => self.restore_arp_routing(pool),
            (false, true) => self.enter_poly_routing(pool),
            _ => {}
        }
    }

    fn poly_advance(&mut self, pool: &mut VoicePool<PACKS>) {
        match self.poly.advance() {
            Some(PolyEvent::Step(event)) => self.apply_poly_step(pool, event),
            Some(PolyEvent::GateOff(release)) => self.release_sequence_lanes(pool, release),
            None => {}
        }
    }

    fn release_gated_notes(&mut self, pool: &mut VoicePool<PACKS>) {
        if self.unison.enabled {
            self.release_unison_group(pool);
        } else {
            // The gated sequencer is the envelope gate source; a held sustain
            // pedal must not turn its half-step gate-off into a sustained note.
            let sustain_pressed = self.allocated.sustain_pressed;
            self.allocated.sustain_pressed = false;
            let pressed = self.pressed_keys.clone();
            for (note, _) in pressed.iter() {
                self.allocated
                    .poly_note_off(pool.region_mut(self.region), note);
            }
            self.allocated.sustain_pressed = sustain_pressed;
        }
    }

    fn handle_gated_event(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        event: GatedEvent,
        retrigger: bool,
    ) {
        if !self.gated.envelope_gating() {
            return;
        }
        match event {
            GatedEvent::Boundary { gate: true } if retrigger => self.rebuild_sounding_notes(pool),
            GatedEvent::Boundary { gate: false } | GatedEvent::GateOff => {
                self.release_gated_notes(pool);
            }
            GatedEvent::Boundary { gate: true } => {}
        }
    }

    fn gated_advance(&mut self, pool: &mut VoicePool<PACKS>) {
        let held_note = !self.pressed_keys.is_empty();
        if let Some(event) = self.gated.advance(held_note) {
            self.handle_gated_event(pool, event, true);
        }
        self.performance.sequence = self.gated.outputs();
    }

    fn arp_advance(&mut self, pool: &mut VoicePool<PACKS>) {
        if !self.arp.params().enabled || self.poly.is_active() {
            return;
        }
        let prev_note = self.arp.current_note();
        match self.arp.advance(1) {
            Some(ArpEvent::Release(_)) => {
                if let Some(prev) = prev_note {
                    self.allocated
                        .poly_note_off(pool.region_mut(self.region), prev);
                }
            }
            Some(ArpEvent::Step(note)) => {
                if let Some(prev) = prev_note {
                    self.allocated
                        .poly_note_off(pool.region_mut(self.region), prev);
                }
                self.allocated.poly_note_on(
                    pool.region_mut(self.region),
                    note,
                    self.arp.current_velocity(),
                    prev_note.filter(|_| self.glide.enabled),
                    self.glide.enabled,
                );
            }
            None => {}
        }
    }

    pub(crate) fn next(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        ctx: &mut RenderContext<'_>,
    ) -> (f32, f32) {
        if self.poly.is_active() {
            self.poly_advance(pool);
        } else {
            if self.gated.is_active(!self.pressed_keys.is_empty()) {
                self.gated_advance(pool);
            }
            self.arp_advance(pool);
        }
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let mut effects = EffectModulation::default();
        let next_poly_block = if self.unison.enabled {
            None
        } else {
            let voice = self.allocated.allocate(pool.region(self.region));
            Some(AllocatedVoices::<PACKS>::voice_location(voice).block_index)
        };
        let unison_warm_blocks = if self.unison.enabled {
            let voices = self
                .unison
                .mode
                .voice_count()
                .unwrap_or_else(|| self.unison.chord.intervals().len().max(1))
                .min(self.voice_count());
            voices.div_ceil(WideF32::LANES)
        } else {
            0
        };

        for (block_index, block) in pool.region_mut(self.region).iter_mut().enumerate() {
            let block_voice_count = block.active_lane_count();
            if block_voice_count == 0 {
                let mode =
                    if next_poly_block == Some(block_index) || block_index < unison_warm_blocks {
                        IdleAdvance::Warm
                    } else {
                        IdleAdvance::Cold
                    };
                block.advance_idle(mode, self.performance, &self.modulation, ctx);
                continue;
            }
            let (block_left, block_right) = block.next(self.performance, &self.modulation, ctx);
            left += block_left;
            right += block_right;
            effects.add(block.take_effect_modulation());
        }

        self.last_effect_modulation = effects.scale(1.0 / self.region.pack_count() as f32);
        (left, right)
    }

    pub fn effect_modulation(&self) -> EffectModulation {
        self.last_effect_modulation
    }

    pub fn active_notes(&self, pool: &VoicePool<PACKS>) -> ActiveNotes<PACKS> {
        let mut notes = ActiveNotes::<PACKS>::new();
        self.for_each_active_note(pool, |note| {
            notes.push(note);
        });
        notes
    }

    pub fn active_notes_into(&self, pool: &VoicePool<PACKS>, out: &mut [u8]) -> usize {
        let mut len = 0;
        self.for_each_active_note(pool, |note| {
            if len < out.len() {
                out[len] = note;
                len += 1;
            }
        });
        len
    }

    pub fn for_each_active_note(&self, pool: &VoicePool<PACKS>, mut f: impl FnMut(u8)) {
        for block in pool.region(self.region) {
            block.for_each_active_note(&mut f);
        }
    }

    pub fn active_voice_count(&self, pool: &VoicePool<PACKS>) -> usize {
        pool.region(self.region)
            .iter()
            .map(|block| block.active_lane_count())
            .sum()
    }

    pub fn lowest_active_note(&self, pool: &VoicePool<PACKS>) -> Option<u8> {
        let mut lowest = None;
        self.for_each_active_note(pool, |note| {
            lowest = Some(lowest.map_or(note, |current: u8| current.min(note)));
        });
        lowest
    }

    pub fn set_filter_oversampling(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        oversampling: FilterOversampling,
    ) {
        for block in pool.region_mut(self.region) {
            block.set_filter_oversampling(oversampling);
        }
    }

    pub fn set_filter_type(&mut self, pool: &mut VoicePool<PACKS>, filter_type: FilterType) {
        for block in pool.region_mut(self.region) {
            block.set_filter_type(filter_type);
        }
    }
    pub(crate) fn effects_mut(&mut self) -> &mut EffectsState {
        &mut self.effects
    }

    pub(crate) fn effects_and_volume(&mut self) -> (&mut EffectsState, f32) {
        let program_volume = self.program_volume.next();
        (&mut self.effects, program_volume)
    }

    pub(crate) fn set_program_volume(&mut self, volume: f32) {
        self.program_volume.set_target(volume.clamp(0.0, 1.0));
    }

    #[cfg(test)]
    pub(crate) fn program_volume(&self) -> f32 {
        self.program_volume.target()
    }

    #[cfg(test)]
    pub(crate) fn program_volume_current(&self) -> f32 {
        self.program_volume.value()
    }

    #[cfg(test)]
    pub(crate) fn has_pressed_key(&self, note: u8) -> bool {
        self.pressed_keys.iter().any(|(pressed, _)| pressed == note)
    }

    pub(crate) fn tempo_bpm(&self) -> f32 {
        self.tempo_bpm
    }

    pub(crate) fn local_tempo_bpm(&self) -> f32 {
        self.local_tempo_bpm
    }

    pub(crate) const fn poly_sequencer_active(&self) -> bool {
        self.poly.is_active()
    }

    pub(crate) fn sequencer_active_step(&self) -> Option<u8> {
        if self.poly.is_active() {
            self.poly.last_step()
        } else {
            self.gated.last_step()
        }
    }

    pub(crate) fn pop_recorder_event(&mut self) -> Option<RecorderEvent> {
        self.recorder.pop_event()
    }

    pub(crate) fn apply_recorded_step(&mut self, step: u8, value: PolyStep) {
        self.poly.replace_step(step, value);
    }

    pub(crate) fn replace_sequence(
        &mut self,
        pool: &mut VoicePool<PACKS>,
        sequence: &crate::LayerSequence,
    ) {
        let was_poly_active = self.poly.is_active();
        self.gated.apply_sequence(sequence);
        self.performance.sequence = self.gated.outputs();
        let release = self.poly.apply_sequence(sequence);
        self.release_sequence_lanes(pool, release);
        for track in 0..crate::sequencer::model::GATED_TRACK_COUNT {
            self.modulation
                .set_gated_destination(track, sequence.gated.tracks[track].destination);
        }
        self.modulation
            .set_gated_enabled(sequence.sequencer_type == SequencerType::Gated);
        self.update_poly_routing_transition(pool, was_poly_active);
        self.sync_lfo_engines(pool);
    }

    pub(crate) fn set_local_tempo_bpm(&mut self, bpm: f32) {
        self.local_tempo_bpm = bpm.clamp(30.0, 250.0);
    }
}

#[cfg(test)]
pub(crate) use test_support::TestLayerEngine;

#[cfg(test)]
mod test_support {
    use crate::{
        ClockDivision, ControlMessage, LayerPatch, VOICE_PACKS, profiling::RenderContext,
        voice::VoiceBlock,
    };

    use super::{ActiveNotes, LayerEngine, VoicePool};

    pub(crate) struct TestLayerEngine<const PACKS: usize = VOICE_PACKS> {
        pool: VoicePool<PACKS>,
        layer: LayerEngine<PACKS>,
    }

    impl<const PACKS: usize> TestLayerEngine<PACKS> {
        pub(crate) fn new(sample_rate: f32) -> Self {
            Self {
                pool: VoicePool::new(sample_rate),
                layer: LayerEngine::new(sample_rate),
            }
        }

        pub(crate) fn handle_control(&mut self, message: ControlMessage) {
            self.layer.handle_control(&mut self.pool, message);
        }

        pub(crate) fn apply_patch(&mut self, patch: &LayerPatch) {
            self.layer.apply_patch(&mut self.pool, patch);
        }

        pub(crate) fn next(&mut self, ctx: &mut RenderContext<'_>) -> (f32, f32) {
            self.layer.next(&mut self.pool, ctx)
        }

        pub(crate) fn active_notes(&self) -> ActiveNotes<PACKS> {
            self.layer.active_notes(&self.pool)
        }

        pub(crate) fn active_voice_count(&self) -> usize {
            self.layer.active_voice_count(&self.pool)
        }

        pub(crate) fn sequence_outputs(&self) -> [f32; crate::sequencer::model::GATED_TRACK_COUNT] {
            self.layer.performance.sequence
        }

        pub(crate) fn set_clock_division(&mut self, division: ClockDivision) {
            self.layer.set_clock_division(&mut self.pool, division);
        }

        pub(crate) fn iter(&self) -> core::slice::Iter<'_, VoiceBlock> {
            self.pool.region(self.layer.region).iter()
        }
    }

    impl<const PACKS: usize> core::ops::Deref for TestLayerEngine<PACKS> {
        type Target = LayerEngine<PACKS>;

        fn deref(&self) -> &Self::Target {
            &self.layer
        }
    }

    impl<const PACKS: usize> core::ops::DerefMut for TestLayerEngine<PACKS> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.layer
        }
    }

    impl<const PACKS: usize> core::ops::Index<usize> for TestLayerEngine<PACKS> {
        type Output = VoiceBlock;

        fn index(&self, index: usize) -> &Self::Output {
            &self.pool.region(self.layer.region)[index]
        }
    }

    impl<const PACKS: usize> core::ops::IndexMut<usize> for TestLayerEngine<PACKS> {
        fn index_mut(&mut self, index: usize) -> &mut Self::Output {
            &mut self.pool.region_mut(self.layer.region)[index]
        }
    }
}

/// Snapshot of MIDI notes currently sounding across all voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveNotes<const PACKS: usize = VOICE_PACKS> {
    notes: [[u8; WideF32::LANES]; PACKS],
    len: usize,
}

impl<const PACKS: usize> Default for ActiveNotes<PACKS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const PACKS: usize> ActiveNotes<PACKS> {
    pub const fn new() -> Self {
        Self {
            notes: [[0; WideF32::LANES]; PACKS],
            len: 0,
        }
    }

    pub fn push(&mut self, note: u8) -> bool {
        if self.len >= self.capacity() {
            return false;
        }

        self.notes[self.len / WideF32::LANES][self.len % WideF32::LANES] = note;
        self.len += 1;
        true
    }

    pub const fn capacity(&self) -> usize {
        PACKS * WideF32::LANES
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, note: &u8) -> bool {
        self.iter().any(|held| held == *note)
    }

    pub fn iter(&self) -> ActiveNotesIter<'_, PACKS> {
        ActiveNotesIter {
            notes: self,
            index: 0,
        }
    }
}

pub struct ActiveNotesIter<'a, const PACKS: usize> {
    notes: &'a ActiveNotes<PACKS>,
    index: usize,
}

impl<const PACKS: usize> Iterator for ActiveNotesIter<'_, PACKS> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.notes.len {
            return None;
        }

        let note = self.notes.notes[self.index / WideF32::LANES][self.index % WideF32::LANES];
        self.index += 1;
        Some(note)
    }
}

#[derive(Clone, Copy, Default)]
struct SustainedVoices {
    bits: u32,
}

impl SustainedVoices {
    fn clear(&mut self) {
        self.bits = 0;
    }

    fn contains(self, voice_idx: usize) -> bool {
        debug_assert!(voice_idx < 32);
        (self.bits & (1 << voice_idx)) != 0
    }

    fn insert(&mut self, voice_idx: usize) {
        debug_assert!(voice_idx < 32);
        self.bits |= 1 << voice_idx;
    }

    fn remove(&mut self, voice_idx: usize) {
        debug_assert!(voice_idx < 32);
        self.bits &= !(1 << voice_idx);
    }

    fn is_empty(self) -> bool {
        self.bits == 0
    }

    fn iter(self) -> SustainedVoicesIter {
        SustainedVoicesIter { bits: self.bits }
    }
}

struct SustainedVoicesIter {
    bits: u32,
}

impl Iterator for SustainedVoicesIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bits == 0 {
            return None;
        }
        let voice_idx = self.bits.trailing_zeros() as usize;
        self.bits &= self.bits - 1;
        Some(voice_idx)
    }
}

struct VoiceLocation {
    block_index: usize,
    lane: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VoiceOwner {
    None,
    Live,
    Sequence(u8),
}

struct AllocatedVoices<const PACKS: usize> {
    held: FixedIndexList<PACKS, { WideF32::LANES }>,
    sustained: SustainedVoices,
    sustain_pressed: bool,
    next_voice: usize,
    owners: [VoiceOwner; 32],
}

impl<const PACKS: usize> AllocatedVoices<PACKS> {
    const _VOICE_CAPACITY: () = assert!(PACKS * WideF32::LANES <= 32);
    const VOICE_COUNT: usize = PACKS * WideF32::LANES;

    const fn new() -> Self {
        Self {
            held: FixedIndexList::new(),
            sustained: SustainedVoices { bits: 0 },
            sustain_pressed: false,
            next_voice: 0,
            owners: [VoiceOwner::None; 32],
        }
    }

    fn clear_occupancy(&mut self) {
        self.held.clear();
        self.sustained.clear();
        self.owners.fill(VoiceOwner::None);
    }

    fn voice_location(voice_idx: usize) -> VoiceLocation {
        VoiceLocation {
            block_index: voice_idx / WideF32::LANES,
            lane: voice_idx % WideF32::LANES,
        }
    }

    fn mark_held(&mut self, voice_idx: usize) {
        if self.held.contains(voice_idx) {
            self.held.move_to_back(voice_idx);
        } else {
            self.held.push_back(voice_idx);
        }
    }

    fn find_active_voice(&self, blocks: &[VoiceBlock], note: u8) -> Option<usize> {
        for voice_idx in self.held.iter() {
            if self.owners[voice_idx] != VoiceOwner::Live {
                continue;
            }
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(voice_idx);
            if blocks[block_idx].active_note(lane) == Some(note) {
                return Some(voice_idx);
            }
        }
        None
    }

    fn allocate(&self, blocks: &[VoiceBlock]) -> usize {
        let voice_count = blocks.len() * WideF32::LANES;
        debug_assert!(voice_count > 0 && voice_count <= Self::VOICE_COUNT);
        for offset in 0..voice_count {
            let idx = (self.next_voice + offset) % voice_count;
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(idx);
            if blocks[block_idx].is_lane_silent(lane) {
                return idx;
            }
        }

        for offset in 0..voice_count {
            let idx = (self.next_voice + offset) % voice_count;
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(idx);
            if blocks[block_idx].is_lane_released(lane) {
                return idx;
            }
        }

        for offset in 0..voice_count {
            let idx = (self.next_voice + offset) % voice_count;
            if self.sustained.contains(idx) {
                return idx;
            }
        }

        self.held.front().unwrap_or(0)
    }

    fn poly_note_on(
        &mut self,
        blocks: &mut [VoiceBlock],
        note: u8,
        velocity: f32,
        glide_start: Option<u8>,
        should_glide: bool,
    ) {
        let active_voice_idx = self.find_active_voice(blocks, note);
        let voice_idx = active_voice_idx.unwrap_or_else(|| self.allocate(blocks));
        let VoiceLocation {
            block_index: block_idx,
            lane,
        } = Self::voice_location(voice_idx);
        self.sustained.remove(voice_idx);
        let block = &mut blocks[block_idx];
        let silent = block.is_lane_silent(lane);
        if silent || (active_voice_idx.is_some() && !block.has_pending_note(lane)) {
            if silent {
                block.note_on_tuned_with_glide(
                    lane,
                    note,
                    velocity,
                    false,
                    [0.0; WideF32::LANES],
                    NoteGlide {
                        start_note: glide_start,
                        enabled: should_glide,
                    },
                );
            } else {
                block.retrigger_sounding_with_glide(
                    lane,
                    note,
                    velocity,
                    [0.0; WideF32::LANES],
                    should_glide,
                );
            }
        } else {
            block.schedule_note_on_tuned_with_glide(
                lane,
                note,
                velocity,
                false,
                [0.0; WideF32::LANES],
                NoteGlide {
                    start_note: glide_start,
                    enabled: should_glide,
                },
            );
        }
        self.mark_held(voice_idx);
        self.owners[voice_idx] = VoiceOwner::Live;
        self.next_voice = (voice_idx + 1) % (blocks.len() * WideF32::LANES);
    }

    fn sequence_note_on(
        &mut self,
        blocks: &mut [VoiceBlock],
        lane_owner: u8,
        note: u8,
        velocity: f32,
    ) {
        self.sequence_note_off(blocks, lane_owner);
        let voice_idx = self.allocate(blocks);
        let VoiceLocation {
            block_index: block_idx,
            lane,
        } = Self::voice_location(voice_idx);
        self.held.remove(voice_idx);
        self.sustained.remove(voice_idx);
        let block = &mut blocks[block_idx];
        if block.is_lane_silent(lane) {
            block.note_on(lane, note, velocity, false);
        } else {
            block.schedule_note_on_tuned_with_glide(
                lane,
                note,
                velocity,
                false,
                [0.0; WideF32::LANES],
                NoteGlide::default(),
            );
        }
        self.mark_held(voice_idx);
        self.owners[voice_idx] = VoiceOwner::Sequence(lane_owner);
        self.next_voice = (voice_idx + 1) % (blocks.len() * WideF32::LANES);
    }

    fn sequence_note_off(&mut self, blocks: &mut [VoiceBlock], lane_owner: u8) {
        for voice_idx in 0..blocks.len() * WideF32::LANES {
            if self.owners[voice_idx] != VoiceOwner::Sequence(lane_owner) {
                continue;
            }
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(voice_idx);
            blocks[block_idx].note_off_lane(lane);
            self.held.remove(voice_idx);
            self.sustained.remove(voice_idx);
            self.owners[voice_idx] = VoiceOwner::None;
        }
    }

    fn poly_note_off(&mut self, blocks: &mut [VoiceBlock], note: u8) {
        while let Some(voice_idx) = self.find_active_voice(blocks, note) {
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(voice_idx);
            self.held.remove(voice_idx);
            if self.sustain_pressed {
                self.sustained.insert(voice_idx);
            } else {
                self.sustained.remove(voice_idx);
                let block = &mut blocks[block_idx];
                if block.active_note(lane) == Some(note) {
                    block.note_off_lane(lane);
                }
                self.owners[voice_idx] = VoiceOwner::None;
            }
        }
    }

    fn release_sustained(&mut self, blocks: &mut [VoiceBlock]) {
        while !self.sustained.is_empty() {
            let voice_idx = self.sustained.iter().next().expect("non-empty");
            let VoiceLocation {
                block_index: block_idx,
                lane,
            } = Self::voice_location(voice_idx);
            blocks[block_idx].note_off_lane(lane);
            self.sustained.remove(voice_idx);
            self.owners[voice_idx] = VoiceOwner::None;
        }
    }

    fn set_sustain_pedal(&mut self, blocks: &mut [VoiceBlock], pressed: bool) {
        if self.sustain_pressed && !pressed {
            self.release_sustained(blocks);
        }
        self.sustain_pressed = pressed;
    }
}

#[derive(Clone, Copy, Default)]
struct GlideSettings {
    enabled: bool,
    mode: GlideMode,
}

#[derive(Clone, Copy, Default)]
struct UnisonSettings {
    enabled: bool,
    mode: UnisonMode,
    detune: f32,
    chord: ChordMemory,
}

fn key_mode_retriggers(mode: KeyMode) -> bool {
    matches!(
        mode,
        KeyMode::LowRetrigger | KeyMode::HighRetrigger | KeyMode::LastRetrigger
    )
}

fn midi_filter_cutoff_hz(value: f32) -> f32 {
    let raw = F32(value.clamp(0.0, 1.0) * 127.0).round().as_f32() as u16;
    cutoff_raw_to_hz(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GatedSequencerMode, GatedStep, LayerTarget, ModDestination, ModRoute, ModSource,
        ModulationParam, ParamId, PolyLaneStep, PolyNote, PolyVelocity, SequenceUpdate,
        SequencerType, VOICE_COUNT, voice::REV2_VOICE_PAN_POSITIONS,
    };

    fn process_frames<const PACKS: usize>(voices: &mut TestLayerEngine<PACKS>, frames: usize) {
        let mut ctx = crate::create_render_context!();
        for _ in 0..frames {
            voices.next(&mut ctx);
        }
    }

    fn render_once<const PACKS: usize>(voices: &mut TestLayerEngine<PACKS>) {
        process_frames(voices, 1);
    }

    fn find_gated_note<const PACKS: usize>(
        voices: &TestLayerEngine<PACKS>,
        note: u8,
    ) -> Option<(usize, usize)> {
        for (block_idx, block) in voices.iter().enumerate() {
            for lane in 0..WideF32::LANES {
                if block.lanes().gate(lane) && block.lanes().note(lane) == note {
                    return Some((block_idx, lane));
                }
            }
        }
        None
    }

    fn enable_unison<const PACKS: usize>(voices: &mut TestLayerEngine<PACKS>, mode: UnisonMode) {
        voices.handle_control(ControlMessage::edit_param(
            ParamId::UnisonMode,
            mode.index() as f32,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonEnabled, 1.0));
    }

    #[test]
    fn empty_default_sequence_does_not_gate_normal_notes() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&LayerPatch::default());
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        render_once(&mut voices);
        assert!(find_gated_note(&voices, 60).is_some());
    }

    #[test]
    fn key_step_rest_closes_gate_even_with_sustain_pressed() {
        let mut patch = LayerPatch::default();
        patch.sequence.sequencer_type = SequencerType::Gated;
        patch.sequence.gated_mode = GatedSequencerMode::KeyStep;
        patch.sequence.gated.tracks[0].steps[0] = GatedStep::Rest;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        assert!(find_gated_note(&voices, 60).is_none());
    }

    #[test]
    fn live_gated_edits_reach_only_the_target_layer_runtime() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        let mut patch = LayerPatch::default();
        patch.sequence.sequencer_type = SequencerType::Gated;
        patch.sequence.gated_mode = GatedSequencerMode::KeyStep;
        patch.sequence.gated.tracks[0].steps[0] = GatedStep::Value(25);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        render_once(&mut voices);
        assert!((voices.sequence_outputs()[0] - 0.2).abs() < 1.0e-6);
        voices.handle_control(ControlMessage::SetSequence {
            target: LayerTarget::Edit,
            update: SequenceUpdate::GatedStep {
                track: 0,
                step: 1,
                value: GatedStep::Value(75),
            },
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 62,
            velocity: 1.0,
        });
        render_once(&mut voices);
        assert!((voices.sequence_outputs()[0] - 0.6).abs() < 1.0e-6);
    }

    fn poly_patch(notes: &[u8]) -> LayerPatch {
        let mut patch = LayerPatch::default();
        patch.sequence.sequencer_type = SequencerType::Polyphonic;
        patch.sequence.poly.steps[0].lanes.fill(PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Rest,
        });
        for (lane, note) in notes.iter().copied().take(6).enumerate() {
            patch.sequence.poly.steps[0].lanes[lane] = PolyLaneStep {
                note: PolyNote::Note(note),
                velocity: PolyVelocity::Velocity(127),
            };
        }
        patch
    }

    #[test]
    fn poly_tie_keeps_the_actual_voice_gate_open_through_the_tie_step() {
        let mut patch = poly_patch(&[60]);
        patch.bpm = 120.0;
        patch.clock_divide = ClockDivision::Quarter;
        patch.sequence.poly.steps[1].lanes.fill(PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Rest,
        });
        patch.sequence.poly.steps[1].lanes[0] = PolyLaneStep {
            note: PolyNote::Tie,
            velocity: PolyVelocity::Velocity(127),
        };
        patch.sequence.poly.steps[2].lanes.fill(PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Rest,
        });

        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(100.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });

        render_once(&mut voices); // Note boundary.
        assert!(find_gated_note(&voices, 60).is_some());
        process_frames(&mut voices, 49);
        assert!(find_gated_note(&voices, 60).is_some());

        render_once(&mut voices); // Tie boundary.
        assert!(find_gated_note(&voices, 60).is_some());
        process_frames(&mut voices, 49);
        assert!(find_gated_note(&voices, 60).is_some());

        render_once(&mut voices); // Rest boundary.
        assert!(find_gated_note(&voices, 60).is_none());
    }

    #[test]
    fn stopping_poly_sequence_releases_only_sequence_owned_duplicate_pitch() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&poly_patch(&[60]));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });
        render_once(&mut voices);
        assert_eq!(
            voices
                .active_notes()
                .iter()
                .filter(|note| *note == 60)
                .count(),
            2
        );
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: false,
        });
        assert_eq!(
            voices
                .active_notes()
                .iter()
                .filter(|note| *note == 60)
                .count(),
            1
        );
    }

    #[test]
    fn live_notes_do_not_transpose_a_running_poly_sequence() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(100.0);
        voices.apply_patch(&poly_patch(&[60]));
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });
        render_once(&mut voices);
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });

        process_frames(&mut voices, 50);

        assert_eq!(
            voices
                .active_notes()
                .iter()
                .filter(|note| *note == 60)
                .count(),
            1,
            "the sequence must keep its recorded pitch"
        );
        assert_eq!(
            voices
                .active_notes()
                .iter()
                .filter(|note| *note == 72)
                .count(),
            1,
            "the live note must play alongside the sequence"
        );
    }

    #[test]
    fn record_modifier_transposes_a_running_poly_sequence() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(100.0);
        voices.apply_patch(&poly_patch(&[60]));
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });
        render_once(&mut voices);
        voices.handle_control(ControlMessage::SequencerRecord {
            target: LayerTarget::Edit,
            command: SequencerRecordCommand::Start,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });

        process_frames(&mut voices, 50);

        assert_eq!(
            voices
                .active_notes()
                .iter()
                .filter(|note| *note == 72)
                .count(),
            2,
            "Record+key must transpose the sequence while also sounding the live note"
        );
        assert!(!voices.active_notes().contains(&60));
    }

    #[test]
    fn six_lane_chord_and_voice_stealing_leave_no_sequence_owned_notes() {
        for mut voices in [
            TestLayerEngine::<1>::new(48_000.0),
            TestLayerEngine::<1>::new(48_000.0),
        ] {
            voices.apply_patch(&poly_patch(&[60, 61, 62, 63, 64, 65]));
            voices.handle_control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Edit,
                running: true,
            });
            render_once(&mut voices);
            assert_eq!(voices.active_notes().len(), 6.min(WideF32::LANES));
            voices.handle_control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Edit,
                running: false,
            });
            assert!(voices.active_notes().is_empty());
        }
    }

    #[test]
    fn sequence_stop_bypasses_sustain_and_arp_resumes_after_transport() {
        let mut patch = poly_patch(&[67]);
        patch.arp.enabled = true;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });
        render_once(&mut voices);
        assert!(voices.active_notes().contains(&67));
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: false,
        });
        assert!(!voices.active_notes().contains(&67));
        render_once(&mut voices);
        assert!(voices.active_notes().contains(&60));
    }

    #[test]
    fn repeated_play_stop_and_patch_swaps_never_orphan_sequence_voices() {
        let mut voices = TestLayerEngine::<2>::new(48_000.0);
        let mut first = poly_patch(&[60, 64, 67]);
        let second = poly_patch(&[61, 65, 68]);
        voices.apply_patch(&first);
        for iteration in 0..256 {
            voices.handle_control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Edit,
                running: true,
            });
            render_once(&mut voices);
            if iteration % 3 == 0 {
                first = if first.sequence.poly.steps[0].lanes[0].note == PolyNote::Note(60) {
                    second.clone()
                } else {
                    poly_patch(&[60, 64, 67])
                };
                voices.apply_patch(&first);
            }
            voices.handle_control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Edit,
                running: false,
            });
            process_frames(&mut voices, 8);
            assert!(voices.active_notes().is_empty(), "iteration {iteration}");
        }
    }

    #[test]
    fn live_type_changes_restore_and_resuspend_arp_without_stuck_notes() {
        let mut patch = poly_patch(&[67]);
        patch.arp.enabled = true;
        let mut voices = TestLayerEngine::<2>::new(100.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Edit,
            running: true,
        });
        render_once(&mut voices);
        assert!(voices.poly_sequencer_active());
        voices.handle_control(ControlMessage::SetSequence {
            target: LayerTarget::Edit,
            update: SequenceUpdate::Type(SequencerType::Gated),
        });
        assert!(!voices.poly_sequencer_active());
        render_once(&mut voices);
        assert!(voices.active_notes().contains(&60));
        assert!(!voices.active_notes().contains(&67));

        voices.handle_control(ControlMessage::SetSequence {
            target: LayerTarget::Edit,
            update: SequenceUpdate::Type(SequencerType::Polyphonic),
        });
        assert!(!voices.poly_sequencer_active());
        voices.handle_control(ControlMessage::SetSequencerTransport {
            target: LayerTarget::Edit,
            command: SequencerTransportCommand::Start,
        });
        assert!(voices.poly_sequencer_active());
        process_frames(&mut voices, 60);
        assert!(voices.active_notes().contains(&67));
        voices.handle_control(ControlMessage::AllNotesOff);
        assert!(voices.active_notes().is_empty());
    }

    fn configure_glide<const PACKS: usize>(voices: &mut TestLayerEngine<PACKS>, mode: GlideMode) {
        voices.handle_control(ControlMessage::edit_param(ParamId::Osc1Glide, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::GlideMode,
            mode.index() as f32,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::GlideEnabled, 1.0));
    }

    fn gated_note_frequency<const PACKS: usize>(voices: &TestLayerEngine<PACKS>, note: u8) -> f32 {
        let (block, lane) = find_gated_note(voices, note).expect("note should be gated");
        voices[block].oscillators().osc1_frequency_hz().to_array()[lane]
    }

    #[test]
    fn allocation_and_stealing_stay_inside_the_assigned_region() {
        let mut pool = VoicePool::<4>::new(44_100.0);
        let mut layer = LayerEngine::<4>::new(44_100.0);
        layer.region = VoiceRegion {
            start_pack: 1,
            pack_count: 2,
        };

        for note in 36..60 {
            layer.handle_control(
                &mut pool,
                ControlMessage::NoteOn {
                    note,
                    velocity: 1.0,
                },
            );
        }
        let mut ctx = crate::create_render_context!();
        for _ in 0..512 {
            layer.next(&mut pool, &mut ctx);
        }

        assert_eq!(pool.blocks[0].active_lane_count(), 0);
        assert_eq!(pool.blocks[3].active_lane_count(), 0);
        assert_eq!(
            layer.active_voice_count(&pool),
            layer.region.voice_capacity()
        );
        assert!(
            layer
                .active_notes(&pool)
                .iter()
                .all(|note| (36..60).contains(&note))
        );
    }

    #[test]
    fn auto_glide_requires_a_physically_held_key() {
        let mut staccato = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        configure_glide(&mut staccato, GlideMode::FixedTimeAuto);
        staccato.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        staccato.handle_control(ControlMessage::NoteOff { note: 60 });
        staccato.handle_control(ControlMessage::SustainPedal { pressed: true });
        staccato.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        assert!((gated_note_frequency(&staccato, 72) - crate::tuning::midi_to_hz(72)).abs() < 0.1);

        let mut legato = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        configure_glide(&mut legato, GlideMode::FixedTimeAuto);
        legato.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        legato.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        assert!((gated_note_frequency(&legato, 72) - crate::tuning::midi_to_hz(60)).abs() < 0.1);
    }

    #[test]
    fn non_auto_glide_uses_the_previous_staccato_note() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        configure_glide(&mut voices, GlideMode::FixedTime);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });

        assert!((gated_note_frequency(&voices, 72) - crate::tuning::midi_to_hz(60)).abs() < 0.1);
    }

    #[test]
    fn polyphonic_note_on_does_not_cancel_a_sibling_lane_glide() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        configure_glide(&mut voices, GlideMode::FixedTime);
        for note in [60, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        process_frames(&mut voices, 1_000);
        let before = gated_note_frequency(&voices, 72);
        assert!(before > crate::tuning::midi_to_hz(60));
        assert!(before < crate::tuning::midi_to_hz(72));

        voices.handle_control(ControlMessage::NoteOn {
            note: 76,
            velocity: 1.0,
        });

        let after = gated_note_frequency(&voices, 72);
        assert!((after - before).abs() < 0.001);
        process_frames(&mut voices, 1);
        assert!(gated_note_frequency(&voices, 72) > after);
    }

    #[test]
    fn every_unison_stack_size_uses_the_requested_voice_count() {
        for (index, mode) in UnisonMode::ALL[..16].iter().copied().enumerate() {
            let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
            enable_unison(&mut voices, mode);
            voices.handle_control(ControlMessage::NoteOn {
                note: 60,
                velocity: 1.0,
            });
            assert_eq!(voices.active_voice_count(), index + 1, "{}", mode.name());
            assert!(voices.active_notes().iter().all(|note| note == 60));
        }
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn unison_detune_is_symmetric_and_centered() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonDetune, 16.0));
        enable_unison(&mut voices, UnisonMode::V4);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        let tuning: [f32; 4] =
            core::array::from_fn(|voice| unison_detune_cents(voice, 4, voices.unison.detune));
        assert_eq!(tuning[0], -16.0);
        assert!((tuning[1] + 16.0 / 3.0).abs() < 0.001);
        assert!((tuning[2] - 16.0 / 3.0).abs() < 0.001);
        assert_eq!(tuning[3], 16.0);
        assert!((tuning.iter().sum::<f32>()).abs() < 0.001);
        let frequencies = voices[0].oscillators().osc1_frequency_hz().to_array();
        let center = crate::tuning::midi_to_hz(60);
        assert!(frequencies[0] < center);
        assert!(frequencies[3] > center);
        assert!((frequencies[0] * frequencies[3] - center * center).abs() < 1.0);
    }

    #[test]
    fn low_priority_retunes_legato_and_falls_back_on_release() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        enable_unison(&mut voices, UnisonMode::V2);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 8);
        let age = voices[0].lanes().age(0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 0.8,
        });
        assert!(voices.active_notes().iter().all(|note| note == 60));
        assert_eq!(
            voices[0].lanes().age(0),
            age,
            "legato note must not retrigger"
        );

        voices.handle_control(ControlMessage::NoteOn {
            note: 55,
            velocity: 0.7,
        });
        assert!(voices.active_notes().iter().all(|note| note == 55));
        assert_eq!(voices[0].lanes().age(0), age);
        voices.handle_control(ControlMessage::NoteOff { note: 55 });
        assert!(voices.active_notes().iter().all(|note| note == 60));
    }

    #[test]
    fn retrigger_key_modes_restart_unison_group_in_place() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(
            ParamId::KeyMode,
            KeyMode::LowRetrigger.index() as f32,
        ));
        enable_unison(&mut voices, UnisonMode::V2);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 8);
        assert!(voices[0].lanes().age(0) > 0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 0.5,
        });
        assert!(!voices[0].has_pending_note(0));
        assert!(voices.active_notes().iter().all(|note| note == 60));
        assert_eq!(voices[0].lanes().age(0), 0);
    }

    #[test]
    fn last_retrigger_unison_glides_without_a_pending_voice_steal() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.handle_control(ControlMessage::edit_param(
            ParamId::KeyMode,
            KeyMode::LastRetrigger.index() as f32,
        ));
        configure_glide(&mut voices, GlideMode::FixedTime);
        enable_unison(&mut voices, UnisonMode::V4);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });

        assert!(!voices[0].has_pending_note(0));
        assert!(voices.active_notes().iter().all(|note| note == 72));
        let start = gated_note_frequency(&voices, 72);
        assert!((start - crate::tuning::midi_to_hz(60)).abs() < 0.1);
        process_frames(&mut voices, 1_000);
        let progressed = gated_note_frequency(&voices, 72);
        assert!(progressed > start);
        assert!(progressed < crate::tuning::midi_to_hz(72));
    }

    #[test]
    fn unison_last_retrigger_legato_does_not_click_from_dsp_reset() {
        let sample_rate = 44_100.0;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);
        let mut patch = LayerPatch::default();
        patch.osc1.waveform = 3;
        patch.osc1.enabled = true;
        patch.osc1.note_reset = false;
        patch.osc1.shape_mod = 0.67;
        patch.osc1.frequency = 24.0;
        patch.osc2.enabled = false;
        patch.osc_mix = 0.0;
        patch.sub_osc_level = 0.9;
        patch.noise_level = 0.0;
        patch.filter.cutoff = 211.0;
        patch.filter.resonance = 0.26;
        patch.filter.key_track = 0.386;
        patch.filter.env_amount = 0.118;
        patch.filter.eg_attack = 0.0005;
        patch.filter.eg_decay = 2.0;
        patch.filter.eg_sustain = 0.575;
        patch.amplifier.eg_attack = 0.0005;
        patch.amplifier.eg_decay = 0.0005;
        patch.amplifier.eg_sustain = 1.0;
        patch.amplifier.env_amount = 1.0;
        patch.key_mode = KeyMode::LastRetrigger;
        patch.unison_enabled = true;
        patch.unison_mode = UnisonMode::V7;
        patch.unison_detune = 4.0;
        patch.glide_enabled = true;
        patch.glide_mode = GlideMode::FixedRateAuto;
        patch.osc1.glide = 0.9;
        patch.osc2.glide = 0.9;
        voices.apply_patch(&patch);

        voices.handle_control(ControlMessage::NoteOn {
            note: 36,
            velocity: 1.0,
        });

        let mut ctx = crate::create_render_context!();
        let settle = (sample_rate * 0.05) as usize;
        let mut prev = 0.0f32;
        for _ in 0..settle {
            let (left, right) = voices.next(&mut ctx);
            prev = left + right;
        }

        let mut baseline_max_delta = 0.0f32;
        for _ in 0..128 {
            let (left, right) = voices.next(&mut ctx);
            let sample = left + right;
            baseline_max_delta = baseline_max_delta.max((sample - prev).abs());
            prev = sample;
        }

        voices.handle_control(ControlMessage::NoteOn {
            note: 43,
            velocity: 1.0,
        });

        let mut retrigger_max_delta = 0.0f32;
        for _ in 0..16 {
            let (left, right) = voices.next(&mut ctx);
            let sample = left + right;
            retrigger_max_delta = retrigger_max_delta.max((sample - prev).abs());
            prev = sample;
        }

        assert!(
            retrigger_max_delta < baseline_max_delta.max(0.02) * 3.0 + 0.05,
            "in-place unison retrigger must not invent a click from DSP reset, key-track snap, or velocity jump, \
             baseline_max_delta={baseline_max_delta} retrigger_max_delta={retrigger_max_delta}"
        );
    }

    #[test]
    fn legato_velocity_change_does_not_click_amp_gain() {
        let sample_rate = 44_100.0;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);
        let mut patch = LayerPatch::default();
        patch.osc1.waveform = 3;
        patch.osc1.enabled = true;
        patch.osc1.note_reset = false;
        patch.osc1.shape_mod = 0.67;
        patch.osc1.frequency = 24.0;
        patch.osc2.enabled = false;
        patch.osc_mix = 0.0;
        patch.sub_osc_level = 0.9;
        patch.filter.cutoff = 211.0;
        patch.filter.resonance = 0.26;
        patch.filter.key_track = 0.386;
        patch.filter.env_amount = 0.0;
        patch.amplifier.velocity = 0.441;
        patch.amplifier.eg_attack = 0.0005;
        patch.amplifier.eg_decay = 0.0005;
        patch.amplifier.eg_sustain = 1.0;
        patch.amplifier.env_amount = 1.0;
        patch.key_mode = KeyMode::LastRetrigger;
        patch.unison_enabled = true;
        patch.unison_mode = UnisonMode::V7;
        patch.unison_detune = 4.0;
        patch.glide_enabled = true;
        patch.glide_mode = GlideMode::FixedRateAuto;
        patch.osc1.glide = 0.9;
        voices.apply_patch(&patch);

        voices.handle_control(ControlMessage::NoteOn {
            note: 36,
            velocity: 1.0,
        });
        let mut ctx = crate::create_render_context!();
        let settle = (sample_rate * 0.05) as usize;
        let mut prev = 0.0f32;
        for _ in 0..settle {
            let (left, right) = voices.next(&mut ctx);
            prev = left + right;
        }
        let mut baseline_max_delta = 0.0f32;
        for _ in 0..128 {
            let (left, right) = voices.next(&mut ctx);
            let sample = left + right;
            baseline_max_delta = baseline_max_delta.max((sample - prev).abs());
            prev = sample;
        }

        voices.handle_control(ControlMessage::NoteOn {
            note: 43,
            velocity: 0.7,
        });
        let mut retrigger_max_delta = 0.0f32;
        for _ in 0..256 {
            let (left, right) = voices.next(&mut ctx);
            let sample = left + right;
            retrigger_max_delta = retrigger_max_delta.max((sample - prev).abs());
            prev = sample;
        }

        assert!(
            retrigger_max_delta < baseline_max_delta.max(0.02) * 3.0 + 0.05,
            "legato velocity changes must ramp amp gain instead of clicking, \
             baseline_max_delta={baseline_max_delta} retrigger_max_delta={retrigger_max_delta}"
        );
    }

    #[cfg(not(feature = "wide-1"))]
    #[test]
    fn repeated_unison_note_waits_for_click_safe_shutdown_and_keeps_detune() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonDetune, 12.0));
        enable_unison(&mut voices, UnisonMode::V4);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 8);
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });

        for lane in 0..4 {
            assert!(voices[0].has_pending_note(lane));
        }

        process_frames(&mut voices, 256);
        for lane in 0..4 {
            assert!(!voices[0].has_pending_note(lane));
        }
        let frequencies = voices[0].oscillators().osc1_frequency_hz().to_array();
        let center = crate::tuning::midi_to_hz(60);
        assert!(frequencies[0] < center);
        assert!(frequencies[3] > center);
    }

    #[test]
    fn high_and_last_priority_follow_the_documented_selection_rules() {
        for (mode, expected, fallback) in [(KeyMode::High, 67, 60), (KeyMode::Last, 55, 67)] {
            let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
            voices.handle_control(ControlMessage::edit_param(
                ParamId::KeyMode,
                mode.index() as f32,
            ));
            enable_unison(&mut voices, UnisonMode::V2);
            for note in [60, 67, 55] {
                voices.handle_control(ControlMessage::NoteOn {
                    note,
                    velocity: 1.0,
                });
            }
            assert!(voices.active_notes().iter().all(|note| note == expected));
            voices.handle_control(ControlMessage::NoteOff { note: expected });
            assert!(voices.active_notes().iter().all(|note| note == fallback));
        }
    }

    #[cfg(not(feature = "wide-1"))]
    #[test]
    fn live_detune_update_changes_pitch_without_retriggering() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        enable_unison(&mut voices, UnisonMode::V2);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 8);
        let age = voices[0].lanes().age(0);
        let before = voices[0].oscillators().osc1_frequency_hz().to_array();
        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonDetune, 12.0));
        let after = voices[0].oscillators().osc1_frequency_hz().to_array();
        assert_eq!(voices[0].lanes().age(0), age);
        assert!(after[0] < before[0]);
        assert!(after[1] > before[1]);
    }

    #[test]
    fn chord_memory_transposes_voicing_and_omits_overflow() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        let chord = ChordMemory::from_notes([60, 64, 67]);
        voices.handle_control(ControlMessage::edit_unison_chord(chord));
        enable_unison(&mut voices, UnisonMode::Chord);
        voices.handle_control(ControlMessage::NoteOn {
            note: 62,
            velocity: 1.0,
        });
        let notes: heapless::Vec<u8, 16> = voices.active_notes().iter().collect();
        assert_eq!(notes.as_slice(), &[62, 66, 69]);

        voices.handle_control(ControlMessage::NoteOff { note: 62 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 124,
            velocity: 1.0,
        });
        let notes: heapless::Vec<u8, 16> = voices.active_notes().iter().collect();
        assert_eq!(notes.as_slice(), &[124]);
    }

    #[test]
    fn unison_sustain_holds_group_until_pedal_release() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        enable_unison(&mut voices, UnisonMode::V4);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        assert_eq!(voices.active_notes().len(), 4);
        voices.handle_control(ControlMessage::SustainPedal { pressed: false });
        assert!(voices.active_notes().is_empty());
    }

    #[test]
    fn disabling_unison_revoices_all_physically_held_keys_polyphonically() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(
            ParamId::KeyMode,
            KeyMode::Last.index() as f32,
        ));
        enable_unison(&mut voices, UnisonMode::V4);
        for note in [60, 64] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        assert!(voices.active_notes().iter().all(|note| note == 64));
        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonEnabled, 0.0));
        let notes = voices.active_notes();
        assert_eq!(notes.len(), 2);
        assert!(notes.contains(&60));
        assert!(notes.contains(&64));
    }

    #[test]
    fn repeated_note_on_retriggers_existing_voice() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.7,
        });

        let active_notes = voices.active_notes();
        let mut notes = active_notes.iter();
        assert_eq!(notes.next(), Some(60));
        assert_eq!(
            notes.next(),
            None,
            "repeated note-on should not allocate duplicate voices for the same key"
        );
    }

    #[test]
    fn note_on_reuses_silent_voice_before_stealing_held_voice() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 62,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 62 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        let held_notes = voices.active_notes();

        assert!(held_notes.contains(&60), "held note 60 was stolen");
        assert!(held_notes.contains(&64), "new note 64 was not allocated");
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn four_notes_are_rendered_as_distinct_simd_lanes() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        assert_eq!(voices.iter().len(), VOICE_PACKS);
        let block = &voices[0];
        assert_eq!(block.lanes().gates_array(), [true, true, true, true]);
        assert_eq!(block.lanes().notes_array(), [60, 64, 67, 72]);

        for lane in 0..WideF32::LANES {
            let expected = crate::tuning::midi_to_hz(block.lanes().notes_array()[lane]);
            let osc1_freq = block.oscillators().osc1_frequency_hz().to_array()[lane];
            assert!(
                (osc1_freq - expected).abs() < 0.1,
                "lane {lane} should keep its own pitch, got {} expected {expected}",
                osc1_freq
            );
        }
    }

    #[test]
    fn pitch_bend_transposes_both_oscillators_by_two_semitones() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::PitchBend { value: 1.0 });

        render_once(&mut voices);

        let expected = crate::tuning::midi_to_hz(62);
        let block = &voices[0];
        let osc1 = block.oscillators().osc1_frequency_hz().to_array()[0];
        assert!(
            (osc1 - expected).abs() < 0.1,
            "osc 1 was {osc1}, expected {expected}"
        );
    }

    #[test]
    fn pitch_bend_remains_available_as_a_mod_matrix_source() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetModulation {
            target: LayerTarget::Edit,
            route: ModRoute::Free(0),
            enabled: true,
            source: ModSource::PitchBend,
            destination: ModDestination::Osc1Frequency,
            amount: 0.5,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::PitchBend { value: 1.0 });

        render_once(&mut voices);

        let block = &voices[0];
        let osc1 = block.oscillators().osc1_frequency_hz().to_array()[0];
        let expected_osc1 = crate::tuning::midi_to_hz(68);
        assert!(
            (osc1 - expected_osc1).abs() < 0.1,
            "matrix-routed osc 1 was {osc1}, expected {expected_osc1}"
        );
    }

    #[test]
    fn physical_voices_use_the_rev2_pan_pattern() {
        let voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for voice_index in 0..REV2_VOICE_PAN_POSITIONS.len() {
            let VoiceLocation {
                block_index: block,
                lane,
            } = AllocatedVoices::<{ crate::VOICE_PACKS }>::voice_location(voice_index);
            assert_eq!(
                voices[block].lanes().pan_positions_array()[lane],
                REV2_VOICE_PAN_POSITIONS[voice_index]
            );
        }
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn pan_pattern_scales_with_configured_voice_count() {
        let four_voices = TestLayerEngine::<1>::new(44_100.0);
        assert_eq!(
            four_voices[0].lanes().pan_positions_array(),
            [1.0, -1.0, 0.5, -0.5]
        );

        let eight_voices = TestLayerEngine::<2>::new(44_100.0);
        assert_eq!(
            eight_voices[0].lanes().pan_positions_array(),
            [1.0, -1.0, 0.75, -0.75]
        );
        assert_eq!(
            eight_voices[1].lanes().pan_positions_array(),
            [0.5, -0.5, 0.25, -0.25]
        );
    }

    #[test]
    fn lfo_key_sync_resets_only_on_first_held_note() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Rate, 25.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1KeySync, 1.0));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 64);
        let before_second_note = voices[0].lfos()[0].output().to_array()[0];
        assert!(
            before_second_note.abs() > 0.01,
            "LFO should have advanced before the second note"
        );

        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);
        let after_second_note = voices[0].lfos()[0].output().to_array()[0];
        assert!(
            after_second_note.abs() > 0.01,
            "key sync should not reset when another note is already held"
        );

        voices.handle_control(ControlMessage::AllNotesOff);
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);
        let after_new_first_note = voices[0].lfos()[0].output().to_array()[0];
        assert!(
            after_new_first_note.abs() < 1.0e-6,
            "key sync should reset when a new first held note starts, got {after_new_first_note}"
        );
    }

    #[test]
    fn lfo_key_sync_phase_continues_when_a_later_block_becomes_active() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Rate, 25.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1KeySync, 1.0));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 64);
        for note in 61..=64 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        process_frames(&mut voices, 1);

        assert_eq!(
            voices[1].lfos()[0].output().to_array()[0].to_bits(),
            voices[0].lfos()[0].output().to_array()[0].to_bits(),
            "the fifth note should join the LFO cycle started by the first key"
        );
    }

    #[test]
    fn rebuilding_held_notes_does_not_reset_key_synced_lfos() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1Rate, 25.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Lfo1KeySync, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 64);

        voices.handle_control(ControlMessage::edit_param(ParamId::UnisonEnabled, 1.0));
        process_frames(&mut voices, 1);

        assert!(
            voices[0].lfos()[0].output().to_array()[0].abs() > 0.01,
            "rebuilding voices without a key press must preserve LFO phase"
        );
    }

    #[test]
    fn steals_oldest_voice_when_polyphony_exhausted() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 200);
        for note in 61..=75u8 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        assert_eq!(voices.active_voice_count(), VOICE_COUNT);

        voices.handle_control(ControlMessage::NoteOn {
            note: 76,
            velocity: 1.0,
        });

        let held = voices.active_notes();
        assert!(
            !held.contains(&60),
            "oldest note 60 should be stolen when polyphony is full"
        );
        assert!(held.contains(&76), "new note 76 should be allocated");
        assert_eq!(
            voices[0].active_note(0),
            Some(76),
            "stolen voice should reserve its lane for note 76"
        );
        assert_eq!(
            voices[0].lanes().note(0),
            60,
            "old DSP state should fade first"
        );
        assert!(voices[0].has_pending_note(0));
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn stolen_voice_starts_after_five_millisecond_shutdown() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 128);
        for note in 61..=63 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        assert!(voices[0].has_pending_note(0));
        assert_eq!(voices[0].lanes().note(0), 60);
        assert!(!voices[0].lanes().gate(0));

        process_frames(&mut voices, 221);
        assert!(voices[0].has_pending_note(0));
        assert_eq!(voices[0].lanes().note(0), 60);

        process_frames(&mut voices, 1);
        assert!(!voices[0].has_pending_note(0));
        assert_eq!(voices[0].lanes().note(0), 64);
        assert!(voices[0].lanes().gate(0));
    }

    #[test]
    fn note_off_cancels_a_pending_stolen_note() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
        for note in 60..=63 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        process_frames(&mut voices, 128);

        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 64 });
        process_frames(&mut voices, 128);

        assert!(!voices.active_notes().contains(&64));
        assert!(!voices[0].has_pending_note(0));
        assert_ne!(voices[0].lanes().note(0), 64);
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn sustain_holds_and_then_cancels_a_pending_stolen_note() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::AmpEgAttack, 0.0005));
        for note in 60..=63 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        process_frames(&mut voices, 128);

        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 64 });
        assert!(voices.active_notes().contains(&64));
        assert!(voices[0].has_pending_note(0));

        voices.handle_control(ControlMessage::SustainPedal { pressed: false });
        assert!(!voices.active_notes().contains(&64));
        assert!(!voices[0].has_pending_note(0));
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn one_voice_pack_limits_polyphony_to_four_voices() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        for note in [60, 61, 62, 63, 64] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        let held = voices.active_notes();
        assert_eq!(voices.iter().len(), 1);
        assert_eq!(voices.active_voice_count(), WideF32::LANES);
        assert_eq!(held.len(), WideF32::LANES);
        assert!(!held.contains(&60), "oldest note should be stolen");
        assert!(held.contains(&64), "new note should be allocated");
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn allocates_across_voice_blocks() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72, 76] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        assert_eq!(voices[0].lanes().gates_array(), [true, true, true, true]);
        assert!(voices[1].lanes().gates_array().iter().any(|gate| *gate));
        assert_eq!(find_gated_note(&voices, 76), Some((1, 0)));
    }

    #[test]
    fn zero_velocity_note_on_is_note_off() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.0,
        });

        assert!(voices.active_notes().is_empty());
        assert_eq!(voices.active_voice_count(), 0);
    }

    #[test]
    fn all_notes_off_clears_active_voices() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        voices.handle_control(ControlMessage::AllNotesOff);

        assert!(voices.active_notes().is_empty());
        assert_eq!(voices.active_voice_count(), 0);
    }

    #[test]
    fn standard_filter_control_changes_update_every_voice_block() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);

        voices.handle_control(ControlMessage::ControlChange {
            controller: MIDI_CC_FILTER_CUTOFF,
            value: 0.5,
        });
        voices.handle_control(ControlMessage::ControlChange {
            controller: MIDI_CC_FILTER_RESONANCE,
            value: 0.75,
        });

        let expected_cutoff = cutoff_raw_to_hz(64);
        for block in voices.iter() {
            assert!((block.filter().cutoff() - expected_cutoff).abs() < 0.001);
            assert_eq!(block.filter().resonance(), 0.75);
        }
    }

    #[test]
    fn partial_modulation_updates_activate_complete_routes() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetModulationParam {
            target: LayerTarget::Edit,
            route: ModRoute::Free(0),
            parameter: ModulationParam::Source(ModSource::Lfo1),
        });
        voices.handle_control(ControlMessage::SetModulationParam {
            target: LayerTarget::Edit,
            route: ModRoute::Free(0),
            parameter: ModulationParam::Destination(ModDestination::FilterCutoff),
        });
        voices.handle_control(ControlMessage::SetModulationParam {
            target: LayerTarget::Edit,
            route: ModRoute::Free(0),
            parameter: ModulationParam::Amount(0.75),
        });

        let slot = voices.modulation.test_matrix_slot(0);
        assert!(slot.enabled);
        assert_eq!(slot.source, ModSource::Lfo1);
        assert_eq!(slot.destination, ModDestination::FilterCutoff);
        assert_eq!(slot.amount, 0.75);
    }

    #[test]
    fn sustain_defers_note_off_until_pedal_release() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });

        assert!(voices.active_notes().contains(&60));
        assert!(voices.allocated.held.is_empty());
        assert!(voices.allocated.sustained.contains(0));

        voices.handle_control(ControlMessage::SustainPedal { pressed: false });

        assert!(!voices[0].lanes().gate(0));
        assert!(!voices.allocated.sustained.contains(0));
    }

    #[test]
    fn pedal_release_keeps_physically_held_notes_gated() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        voices.handle_control(ControlMessage::SustainPedal { pressed: false });

        assert!(!voices.active_notes().contains(&60));
        assert!(voices.active_notes().contains(&64));
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn sustained_voice_is_stolen_before_held_voice() {
        let mut voices = TestLayerEngine::<1>::new(44_100.0);
        for note in 60..=63 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        let active = voices.active_notes();
        assert_eq!(active.len(), WideF32::LANES);
        assert!(!active.contains(&60));
        for note in 61..=64 {
            assert!(active.contains(&note));
        }
    }

    #[test]
    fn retrigger_preserves_physical_voice_pan_position() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::PanSpread, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        let pan_before = voices[0].lanes().pan_positions_array()[0];
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.8,
        });

        assert_eq!(voices[0].lanes().pan_positions_array()[0], pan_before);
    }

    #[test]
    fn reuses_fully_silent_lane_after_release() {
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::AmpEgRelease, 0.0005));
        for note in 60..=75u8 {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }
        assert_eq!(voices.active_voice_count(), VOICE_COUNT);

        let released_location = find_gated_note(&voices, 61).unwrap();
        voices.handle_control(ControlMessage::NoteOff { note: 61 });
        process_frames(&mut voices, 512);
        assert!(voices[released_location.0].is_lane_silent(released_location.1));

        voices.handle_control(ControlMessage::NoteOn {
            note: 76,
            velocity: 1.0,
        });

        assert_eq!(
            find_gated_note(&voices, 76),
            Some(released_location),
            "fully silent lane should be reused"
        );
        assert!(!voices.active_notes().contains(&61));
        assert!(voices.active_notes().contains(&76));
    }

    #[test]
    fn staccato_c3_to_c5_triggers_glide_in_fixed_rate_mode() {
        let mut voices = TestLayerEngine::<4>::new(44_100.0);
        voices.handle_control(ControlMessage::edit_param(ParamId::Osc1Glide, 1.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::Osc2Glide, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::GlideMode,
            GlideMode::FixedRate.index() as f32,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::GlideEnabled, 1.0));

        // Press C3 (MIDI 48), then release it.
        voices.handle_control(ControlMessage::NoteOn {
            note: 48,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOff { note: 48 });
        process_frames(&mut voices, 512);

        // Press C5 (MIDI 72) — should glide from 48→72.
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);

        let (_block_idx, _lane) =
            find_gated_note(&voices, 72).expect("note 72 should be gated after C5 press");

        let freq_after_trigger = gated_note_frequency(&voices, 72);
        let freq_c3 = crate::tuning::midi_to_hz(48u8);
        let freq_c5 = crate::tuning::midi_to_hz(72u8);

        assert!(
            (freq_after_trigger - freq_c3).abs() / freq_c3 < 0.01,
            "immediately after C5 note-on, osc frequency {freq_after_trigger} should be near C3 ({freq_c3}), not C5 ({freq_c5})"
        );

        // Advance ~57 % through the 4.0 s, 24-semitone FixedRate glide.
        process_frames(&mut voices, 100_000);

        let freq_after_many = gated_note_frequency(&voices, 72);

        assert!(
            freq_after_many > freq_after_trigger,
            "after many samples, freq {freq_after_many} should be above initial {freq_after_trigger}"
        );
        assert!(
            freq_after_many < freq_c5,
            "freq {freq_after_many} should still be below C5 ({freq_c5})"
        );
    }

    #[test]
    fn arp_assign_plays_notes_in_press_order() {
        let sample_rate = 44_100.0f32;
        let bps = 120.0 / 60.0;
        let step = (sample_rate / (bps * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Assign.index() as f32,
        ));

        // Simulate pressing C, then E, then G quickly
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        // First step fires immediately
        process_frames(&mut voices, 1);
        let mut seen: heapless::Vec<u8, 6> = heapless::Vec::new();
        if let Some(n) = voices.active_notes().iter().next() {
            let _ = seen.push(n);
        }
        assert_eq!(seen.last().copied(), Some(60), "first immediate step");

        // Advance enough for step 2
        process_frames(&mut voices, step + 10);
        if let Some(n) = voices.active_notes().iter().next() {
            let _ = seen.push(n);
        }
        assert_eq!(seen.last().copied(), Some(64), "second step");

        // Advance for step 3
        process_frames(&mut voices, step + 10);
        if let Some(n) = voices.active_notes().iter().next() {
            let _ = seen.push(n);
        }
        assert_eq!(seen.last().copied(), Some(67), "third step");

        // Wrap back
        process_frames(&mut voices, step + 10);
        if let Some(n) = voices.active_notes().iter().next() {
            let _ = seen.push(n);
        }
        assert_eq!(seen.last().copied(), Some(60), "wrap back to C");
    }

    #[test]
    fn arp_cycles_through_notes_and_each_is_audible() {
        let sample_rate = 44_100.0f32;
        let bps = 120.0 / 60.0;
        let step = (sample_rate / (bps * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        // Keep filter wide open for audibility
        let mut ctx = crate::create_render_context!();
        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));

        // Press three notes — first note triggers immediate step
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Up.index() as f32,
        ));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        assert_eq!(voices.active_notes().len(), 0);

        // First step fires immediately
        process_frames(&mut voices, 1);
        assert_eq!(voices.active_notes().len(), 1);
        assert!(
            voices.active_notes().contains(&60),
            "first arp step should be 60"
        );

        // Render through the step and verify audio is non-silent
        let mut total_a = 0.0f32;
        for _ in 0..(step / 2) {
            let (l, r) = voices.next(&mut ctx);
            total_a += l.abs() + r.abs();
        }
        assert!(
            total_a > 0.01,
            "first arp note must produce audible output, got {total_a}"
        );

        // Advancing to step 2
        process_frames(&mut voices, step - step / 2);
        assert_eq!(voices.active_notes().len(), 1);
        assert!(
            voices.active_notes().contains(&64),
            "second arp step should be 64"
        );

        // Step 2 audio check
        let mut total_b = 0.0f32;
        for _ in 0..(step / 2) {
            let (l, r) = voices.next(&mut ctx);
            total_b += l.abs() + r.abs();
        }
        assert!(
            total_b > 0.01,
            "second arp note must produce audible output, got {total_b}"
        );

        // Wrap around after step 3
        process_frames(&mut voices, step + step - step / 2);
        assert!(
            voices.active_notes().contains(&60),
            "should have wrapped back to 60"
        );
    }

    #[test]
    fn arp_adding_notes_mid_cycle_eventually_plays_them() {
        let sample_rate = 44_100.0f32;
        let step = (sample_rate / (120.0 / 60.0 * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Assign.index() as f32,
        ));

        // Press C
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);
        assert!(voices.active_notes().contains(&60));

        // Run for half a step, then add E mid-cycle
        process_frames(&mut voices, step / 2);
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });

        // Advance past the first step trigger (C again, since step was at 0)
        process_frames(&mut voices, step / 2 + 10);
        // Now we should have stepped either to C or E depending on the rebuild timing
        let note = voices.active_notes().iter().next();
        assert!(note.is_some(), "arp should still have an active note");

        // Advance through several more steps — E must appear eventually
        let mut saw_e = false;
        for _ in 0..4 {
            process_frames(&mut voices, step + 10);
            if let Some(n) = voices.active_notes().iter().next() {
                if n == 64 {
                    saw_e = true;
                    break;
                }
            }
        }
        assert!(
            saw_e,
            "E must appear within a few cycles after being added mid-cycle"
        );
    }

    #[test]
    fn arp_with_fast_clock_division_steps_quickly() {
        let sample_rate = 44_100.0f32;
        let bps = 120.0 / 60.0;
        // Sixteenth = 4 steps per quarter
        let step = (sample_rate / (bps * 4.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Up.index() as f32,
        ));
        // ClockDivide is routed through SynthEngine, not set_param; call directly
        voices.set_clock_division(crate::patch::ClockDivision::Sixteenth);

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        assert!(
            step < (sample_rate / (bps * 1.0)) as usize / 2,
            "sixteenth step {step} should be < half of quarter step {}",
            (sample_rate / (bps * 1.0)) as usize
        );

        process_frames(&mut voices, 1);
        assert!(voices.active_notes().contains(&60), "first step");
        process_frames(&mut voices, step + 10);
        assert!(
            voices.active_notes().contains(&64),
            "second step at sixteenth"
        );
        process_frames(&mut voices, step + 10);
        assert!(
            voices.active_notes().contains(&67),
            "third step at sixteenth"
        );
    }

    #[test]
    fn arp_note_off_removes_note_from_cycle() {
        let sample_rate = 44_100.0f32;
        let step = (sample_rate / (120.0 / 60.0 * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Up.index() as f32,
        ));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        // First step
        process_frames(&mut voices, 1);
        assert!(voices.active_notes().contains(&60));

        // Release middle note
        voices.handle_control(ControlMessage::NoteOff { note: 64 });

        // Advance through several cycles — 64 should never appear
        let mut saw_64 = false;
        for _ in 0..8 {
            process_frames(&mut voices, step + 10);
            if let Some(n) = voices.active_notes().iter().next() {
                saw_64 = saw_64 || n == 64;
            }
        }
        assert!(
            !saw_64,
            "released note 64 must not appear in any subsequent arp step"
        );
    }

    #[test]
    fn arp_with_hold_persists_after_key_release() {
        let sample_rate = 44_100.0f32;
        let step = (sample_rate / (120.0 / 60.0 * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpHold, 1.0));
        voices.handle_control(ControlMessage::edit_param(
            ParamId::ArpMode,
            crate::patch::ArpMode::Up.index() as f32,
        ));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);

        // Release all keys — hold should keep them
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        voices.handle_control(ControlMessage::NoteOff { note: 67 });

        // Advance through a cycle — should still have notes
        process_frames(&mut voices, step + 10);
        assert!(
            voices.active_notes().len() > 0,
            "hold should keep arp running after release"
        );
    }

    #[test]
    fn arp_enable_disable_preserves_held_notes_on_reenable() {
        let sample_rate = 44_100.0f32;
        let _step = (sample_rate / (120.0 / 60.0 * 1.0)) as usize;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(sample_rate);

        voices.handle_control(ControlMessage::edit_param(
            ParamId::FilterCutoff,
            MAX_CUTOFF_HZ,
        ));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterResonance, 0.0));
        voices.handle_control(ControlMessage::edit_param(ParamId::FilterEnvAmount, 0.0));

        // Press notes while arp is ON — they go to arp held_notes, not direct allocation
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::NoteOn {
            note: 67,
            velocity: 1.0,
        });

        // Disable arp — should clear internal state but pressed_keys remain
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 0.0));
        // Notes are still physically held and revoice polyphonically
        assert!(
            voices.active_notes().len() > 0,
            "poly notes should play after arp disabled"
        );

        // Re-enable arp — pressed_keys are still held, should rebuild from them
        voices.handle_control(ControlMessage::edit_param(ParamId::ArpEnabled, 1.0));
        process_frames(&mut voices, 1);
        assert!(
            voices.active_notes().len() > 0,
            "arp should restart with currently held keys"
        );
    }

    #[test]
    fn unison_glide_releases_every_ordered_note_sequence() {
        let patch = unison_glide_retrigger_patch();
        assert!(patch.unison_enabled && patch.glide_enabled);
        assert_eq!(patch.unison_mode, UnisonMode::V8);
        assert_eq!(patch.key_mode, KeyMode::HighRetrigger);

        let release_orders = [
            [48, 59, 72],
            [48, 72, 59],
            [59, 48, 72],
            [59, 72, 48],
            [72, 48, 59],
            [72, 59, 48],
        ];
        for frames_between_events in [0, 1, 32, 256] {
            for release_order in release_orders {
                let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
                voices.apply_patch(&patch);
                for note in [59, 48, 72] {
                    voices.handle_control(ControlMessage::NoteOn {
                        note,
                        velocity: 1.0,
                    });
                    process_frames(&mut voices, frames_between_events);
                }
                for note in release_order {
                    voices.handle_control(ControlMessage::NoteOff { note });
                    process_frames(&mut voices, frames_between_events);
                }

                assert!(
                    voices.active_notes().is_empty(),
                    "gate or pending note survived release order {release_order:?} with delay {frames_between_events}"
                );
            }
        }
    }

    #[test]
    fn patch_rebuild_cannot_resurrect_released_pending_notes() {
        let patch = unison_glide_retrigger_patch();
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);

        voices.handle_control(ControlMessage::NoteOn {
            note: 59,
            velocity: 1.0,
        });
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOff { note: 72 });
        voices.handle_control(ControlMessage::NoteOff { note: 59 });
        for _ in 0..512 {
            process_frames(&mut voices, 1);
        }

        assert!(voices.active_notes().is_empty());
    }

    #[test]
    #[cfg(not(feature = "fast-math"))]
    fn last_retrigger_glides_in_place_without_pending_voices() {
        let mut patch = unison_glide_retrigger_patch();
        patch.key_mode = KeyMode::LastRetrigger;
        let mut voices = TestLayerEngine::<{ crate::VOICE_PACKS }>::new(48_000.0);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        let before = voices[0].oscillators().osc1_frequency_hz().to_array()[0];

        voices.handle_control(ControlMessage::NoteOn {
            note: 72,
            velocity: 1.0,
        });
        let start = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(
            (start - before).abs() < 0.1,
            "glide should start from current pitch; before={before}, start={start}"
        );
        for voice in 0..8 {
            assert!(!voices[voice / WideF32::LANES].has_pending_note(voice % WideF32::LANES));
        }

        for _ in 0..32 {
            process_frames(&mut voices, 1);
        }
        let progressing = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(progressing > start);
        assert!(progressing < before * 2.0);

        for _ in 0..48_000 {
            process_frames(&mut voices, 1);
        }
        let pre = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        for _ in 0..1_000 {
            process_frames(&mut voices, 1);
        }
        let post = voices[0].oscillators().osc1_frequency_hz().to_array()[0];
        assert!(
            (post - pre).abs() / pre < 1.0e-6,
            "frequency should be stable after glide completes; pre {pre}, post {post}"
        );
    }

    #[test]
    fn unison_glide_final_release_reaches_idle() {
        let patch = unison_glide_retrigger_patch();
        const SAMPLE_RATE: f32 = 1_000.0;
        let mut voices = TestLayerEngine::<2>::new(SAMPLE_RATE);
        voices.apply_patch(&patch);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        for _ in 0..SAMPLE_RATE as usize {
            process_frames(&mut voices, 1);
        }
        voices.handle_control(ControlMessage::NoteOff { note: 60 });
        assert!(voices.active_notes().is_empty());
        assert!(
            voices.active_voice_count() > 0,
            "release tail should still render"
        );

        let release_frames = ((patch.amplifier.eg_release + 1.0) * SAMPLE_RATE) as usize;
        for _ in 0..release_frames {
            process_frames(&mut voices, 1);
        }
        assert_eq!(
            voices.active_voice_count(),
            0,
            "release envelope never reached idle"
        );
    }

    #[test]
    #[cfg(not(feature = "wide-1"))]
    fn unison_glide_matches_pressed_key_model_under_adversarial_ordering() {
        let base_patch = unison_glide_retrigger_patch();
        for key_mode in KeyMode::ALL {
            for glide_mode in GlideMode::ALL {
                let mut patch = base_patch.clone();
                patch.key_mode = key_mode;
                patch.glide_mode = glide_mode;
                let mut voices = TestLayerEngine::<2>::new(48_000.0);
                voices.apply_patch(&patch);
                let mut pressed = heapless::Vec::<u8, 128>::new();
                let mut random = 0x6d2b_79f5_u32;

                for event in 0..4_096 {
                    random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let note = 48 + ((random >> 16) % 25) as u8;
                    if random & 1 == 0 {
                        if let Some(index) = pressed.iter().position(|held| *held == note) {
                            pressed.remove(index);
                        }
                        pressed.push(note).unwrap();
                        voices.handle_control(ControlMessage::NoteOn {
                            note,
                            velocity: 1.0,
                        });
                    } else {
                        if let Some(index) = pressed.iter().position(|held| *held == note) {
                            pressed.remove(index);
                        }
                        voices.handle_control(ControlMessage::NoteOff { note });
                    }
                    for _ in 0..((random >> 8) & 3) {
                        process_frames(&mut voices, 1);
                    }

                    let active = voices.active_notes();
                    if pressed.is_empty() {
                        assert!(
                            active.is_empty(),
                            "{key_mode:?}/{glide_mode:?} event {event}: unpressed note remained gated or pending"
                        );
                        continue;
                    }
                    let selected = match key_mode {
                        KeyMode::Low | KeyMode::LowRetrigger => *pressed.iter().min().unwrap(),
                        KeyMode::High | KeyMode::HighRetrigger => *pressed.iter().max().unwrap(),
                        KeyMode::Last | KeyMode::LastRetrigger => *pressed.last().unwrap(),
                    };
                    assert_eq!(
                        active.len(),
                        8,
                        "{key_mode:?}/{glide_mode:?} event {event}: incomplete unison group"
                    );
                    assert!(
                        active.iter().all(|note| note == selected),
                        "{key_mode:?}/{glide_mode:?} event {event}: selected {selected}, active {active:?}, pressed={pressed:?}"
                    );
                }

                voices.handle_control(ControlMessage::AllNotesOff);
                assert!(voices.active_notes().is_empty());
            }
        }
    }

    fn unison_glide_retrigger_patch() -> LayerPatch {
        let mut patch = LayerPatch::default();
        let _ = patch.name.push_str("UnisonGlideHR");
        patch.unison_enabled = true;
        patch.unison_mode = UnisonMode::V8;
        patch.key_mode = KeyMode::HighRetrigger;
        patch.glide_enabled = true;
        patch.glide_mode = GlideMode::FixedRate;
        patch.osc1.glide = 64.0 / 127.0;
        patch.osc2.glide = 64.0 / 127.0;
        patch.amplifier.eg_release = 21.415_247;
        patch
    }
}
