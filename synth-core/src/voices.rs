//! Polyphonic voice allocation and mixing.

use crate::effects::EffectModulation;
#[cfg(test)]
use crate::filter::MAX_CUTOFF_HZ;
use crate::filter::MIN_CUTOFF_HZ;
use crate::fixed_index_list::FixedIndexList;
#[cfg(all(feature = "profiling", test))]
use crate::profiling::NoopProfiler;
#[cfg(feature = "profiling")]
use crate::profiling::RenderProfiler;
use crate::voice::PerformanceModulation;
use crate::{
    ControlMessage, FilterOversampling, FilterType, LANES, ParamId, Patch, VOICE_PACKS, VoiceBlock,
    voice_pan_position,
};
use core::ops::{Deref, DerefMut, Index, IndexMut};

const MIDI_CC_FILTER_RESONANCE: u8 = 71;
const MIDI_CC_FILTER_CUTOFF: u8 = 74;

/// Snapshot of MIDI notes currently sounding across all voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveNotes<const PACKS: usize = VOICE_PACKS> {
    notes: [[u8; LANES]; PACKS],
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
            notes: [[0; LANES]; PACKS],
            len: 0,
        }
    }

    pub fn push(&mut self, note: u8) -> bool {
        if self.len >= self.capacity() {
            return false;
        }

        self.notes[self.len / LANES][self.len % LANES] = note;
        self.len += 1;
        true
    }

    pub const fn capacity(&self) -> usize {
        PACKS * LANES
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

        let note = self.notes.notes[self.index / LANES][self.index % LANES];
        self.index += 1;
        Some(note)
    }
}

/// Sixteen-voice manager built from `PACKS` SIMD [`VoiceBlock`]s.
///
/// Handles note on/off, sustain pedal, parameter updates, pitch bend, and
/// modulation sources, then sums active blocks into a stereo output each sample.
pub struct Voices<const PACKS: usize = VOICE_PACKS> {
    blocks: [VoiceBlock; PACKS],
    held_voices: FixedIndexList<PACKS, LANES>,
    sustained_voices: [[bool; LANES]; PACKS],
    sustain_pressed: bool,
    next_voice: usize,
    performance: PerformanceModulation,
    last_effect_modulation: EffectModulation,
}

impl<const PACKS: usize> Voices<PACKS> {
    pub fn new(sample_rate: f32) -> Self {
        let patch = Patch::default();
        Self {
            blocks: core::array::from_fn(|block_index| {
                let mut block = VoiceBlock::new(sample_rate, &patch);
                block.set_pan_positions(core::array::from_fn(|lane| {
                    voice_pan_position(block_index * LANES + lane, Self::VOICE_COUNT)
                }));
                block
            }),
            held_voices: FixedIndexList::new(),
            sustained_voices: [[false; LANES]; PACKS],
            sustain_pressed: false,
            next_voice: 0,
            performance: PerformanceModulation::default(),
            last_effect_modulation: EffectModulation::default(),
        }
    }

    pub(crate) fn apply_patch(&mut self, patch: &Patch) {
        for block in &mut self.blocks {
            block.apply_patch(patch);
        }
    }

    const VOICE_COUNT: usize = PACKS * LANES;

    pub fn handle_control(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::NoteOn { note, velocity } => {
                if velocity <= 0.0 {
                    self.note_off(note);
                    return;
                }

                let reset_key_synced_lfos = self.held_voices.is_empty();
                let active_voice_idx = self.find_active_voice(note);
                let voice_idx = active_voice_idx.unwrap_or_else(|| self.allocate_voice());
                let (block_idx, lane) = self.voice_location(voice_idx);
                self.sustained_voices[block_idx][lane] = false;
                let block = &mut self.blocks[block_idx];
                let velocity = velocity.clamp(0.0, 1.0);
                if block.is_lane_silent(lane)
                    || (active_voice_idx.is_some() && !block.has_pending_note(lane))
                {
                    block.note_on(lane, note, velocity, reset_key_synced_lfos);
                } else {
                    block.schedule_note_on(lane, note, velocity, reset_key_synced_lfos);
                }
                self.mark_held_voice(voice_idx);
                self.next_voice = (voice_idx + 1) % Self::VOICE_COUNT;
            }
            ControlMessage::NoteOff { note } => {
                self.note_off(note);
            }
            ControlMessage::AllNotesOff => {
                for block in &mut self.blocks {
                    block.all_notes_off();
                }
                self.held_voices.clear();
                self.sustained_voices = [[false; LANES]; PACKS];
            }
            ControlMessage::SetParam(id, value) => self.set_param(id, value),
            ControlMessage::SetFilterType(filter_type) => self.set_filter_type(filter_type),
            ControlMessage::SetModulation {
                route,
                enabled,
                source,
                destination,
                amount,
            } => {
                for block in &mut self.blocks {
                    block.set_mod_route(route, enabled, source, destination, amount);
                }
            }
            ControlMessage::SetModulationParam { route, parameter } => {
                for block in &mut self.blocks {
                    block.set_mod_route_param(route, parameter);
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
                self.set_sustain_pedal(pressed);
            }
            ControlMessage::ControlChange { controller, value } => {
                let value = value.clamp(0.0, 1.0);
                match controller {
                    2 => self.performance.breath = value,
                    4 => self.performance.foot = value,
                    11 => self.performance.expression = value,
                    MIDI_CC_FILTER_RESONANCE => {
                        for block in &mut self.blocks {
                            block.set_filter_resonance(value);
                        }
                    }
                    MIDI_CC_FILTER_CUTOFF => {
                        let cutoff = midi_filter_cutoff_hz(value);
                        for block in &mut self.blocks {
                            block.set_filter_cutoff(cutoff);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn note_off(&mut self, note: u8) {
        while let Some(voice_idx) = self.find_active_voice(note) {
            let (block_idx, lane) = self.voice_location(voice_idx);
            self.held_voices.remove(voice_idx);
            if self.sustain_pressed {
                self.sustained_voices[block_idx][lane] = true;
            } else {
                self.sustained_voices[block_idx][lane] = false;
                let block = &mut self.blocks[block_idx];
                if block.active_note(lane) == Some(note) {
                    block.note_off_lane(lane);
                }
            }
        }
    }

    fn set_sustain_pedal(&mut self, pressed: bool) {
        if self.sustain_pressed && !pressed {
            for (block_idx, sustained) in self.sustained_voices.iter_mut().enumerate() {
                for (lane, is_sustained) in sustained.iter_mut().enumerate() {
                    if *is_sustained {
                        self.blocks[block_idx].note_off_lane(lane);
                        *is_sustained = false;
                    }
                }
            }
        }
        self.sustain_pressed = pressed;
    }

    fn find_active_voice(&self, note: u8) -> Option<usize> {
        for voice_idx in self.held_voices.iter() {
            let (block_idx, lane) = self.voice_location(voice_idx);
            let block = &self.blocks[block_idx];
            if block.active_note(lane) == Some(note) {
                return Some(voice_idx);
            }
        }

        None
    }

    fn mark_held_voice(&mut self, voice_idx: usize) {
        if self.held_voices.contains(voice_idx) {
            self.held_voices.move_to_back(voice_idx);
        } else {
            self.held_voices.push_back(voice_idx);
        }
    }

    fn allocate_voice(&self) -> usize {
        // 1. Steal silent voices first (gate off, envelope done)
        for offset in 0..Self::VOICE_COUNT {
            let idx = (self.next_voice + offset) % Self::VOICE_COUNT;
            let (block_idx, lane) = self.voice_location(idx);
            if self.blocks[block_idx].is_lane_silent(lane) {
                return idx;
            }
        }

        // 2. Reuse a released voice before stealing any held key.
        for offset in 0..Self::VOICE_COUNT {
            let idx = (self.next_voice + offset) % Self::VOICE_COUNT;
            let (block_idx, lane) = self.voice_location(idx);
            let block = &self.blocks[block_idx];
            if block.is_lane_released(lane) {
                return idx;
            }
        }

        // 3. Reuse a sustained voice before stealing a physically held key.
        for offset in 0..Self::VOICE_COUNT {
            let idx = (self.next_voice + offset) % Self::VOICE_COUNT;
            let (block_idx, lane) = self.voice_location(idx);
            if self.sustained_voices[block_idx][lane] {
                return idx;
            }
        }

        // 4. Last resort: steal the oldest held voice.
        self.held_voices.front().unwrap_or(0)
    }

    fn voice_location(&self, voice_idx: usize) -> (usize, usize) {
        (voice_idx / LANES, voice_idx % LANES)
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        for block in &mut self.blocks {
            block.set_param(id, value);
        }
    }
}

fn midi_filter_cutoff_hz(value: f32) -> f32 {
    const CUTOFF_RANGE_OCTAVES: f32 = 9.813_781;
    MIN_CUTOFF_HZ * crate::math::exp2(CUTOFF_RANGE_OCTAVES * value)
}

impl<const PACKS: usize> Voices<PACKS> {
    #[cfg(not(feature = "profiling"))]
    pub(crate) fn next(&mut self) -> (f32, f32) {
        self.next_inner()
    }

    #[cfg(all(feature = "profiling", test))]
    pub(crate) fn next(&mut self) -> (f32, f32) {
        self.next_inner(&mut NoopProfiler)
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn next_profiled(&mut self, profiler: &mut impl RenderProfiler) -> (f32, f32) {
        self.next_inner(profiler)
    }

    fn next_inner(
        &mut self,
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let mut effects = EffectModulation::default();
        for block in &mut self.blocks {
            let block_voice_count = block.active_lane_count();
            if block_voice_count == 0 {
                continue;
            }
            block.age_active_lanes();
            #[cfg(feature = "profiling")]
            let (block_left, block_right) = block.next_profiled(self.performance, profiler);
            #[cfg(not(feature = "profiling"))]
            let (block_left, block_right) = block.next_block(self.performance);
            left += block_left;
            right += block_right;
            effects.add(block.take_effect_modulation());
        }

        self.last_effect_modulation = effects.scale(1.0 / PACKS as f32);
        (left, right)
    }

    pub fn effect_modulation(&self) -> EffectModulation {
        self.last_effect_modulation
    }

    pub fn active_notes(&self) -> ActiveNotes<PACKS> {
        let mut notes = ActiveNotes::<PACKS>::new();
        self.for_each_active_note(|note| {
            notes.push(note);
        });
        notes
    }

    pub fn active_notes_into(&self, out: &mut [u8]) -> usize {
        let mut len = 0;
        self.for_each_active_note(|note| {
            if len < out.len() {
                out[len] = note;
                len += 1;
            }
        });
        len
    }

    pub fn for_each_active_note(&self, mut f: impl FnMut(u8)) {
        for block in &self.blocks {
            block.for_each_active_note(&mut f);
        }
    }

    pub fn active_voice_count(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| block.active_lane_count())
            .sum()
    }

    pub fn lowest_active_note(&self) -> Option<u8> {
        let mut lowest = None;
        self.for_each_active_note(|note| {
            lowest = Some(lowest.map_or(note, |current: u8| current.min(note)));
        });
        lowest
    }

    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        for block in &mut self.blocks {
            block.set_filter_oversampling(oversampling);
        }
    }

    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        for block in &mut self.blocks {
            block.set_filter_type(filter_type);
        }
    }
}

impl<const PACKS: usize> Deref for Voices<PACKS> {
    type Target = [VoiceBlock; PACKS];

    fn deref(&self) -> &Self::Target {
        &self.blocks
    }
}

impl<const PACKS: usize> DerefMut for Voices<PACKS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.blocks
    }
}

impl<const PACKS: usize> Index<usize> for Voices<PACKS> {
    type Output = VoiceBlock;

    fn index(&self, index: usize) -> &Self::Output {
        &self.blocks[index]
    }
}

impl<const PACKS: usize> IndexMut<usize> for Voices<PACKS> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.blocks[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModDestination, ModRoute, ModSource, ModulationParam, ParamId, VOICE_COUNT};

    fn process_frames<const PACKS: usize>(voices: &mut Voices<PACKS>, frames: usize) {
        for _ in 0..frames {
            voices.next();
        }
    }

    fn find_gated_note(voices: &Voices, note: u8) -> Option<(usize, usize)> {
        for (block_idx, block) in voices.iter().enumerate() {
            for lane in 0..LANES {
                if block.test_gate(lane) && block.test_note(lane) == note {
                    return Some((block_idx, lane));
                }
            }
        }
        None
    }

    #[test]
    fn repeated_note_on_retriggers_existing_voice() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
    fn four_notes_are_rendered_as_distinct_simd_lanes() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        assert_eq!(voices.len(), VOICE_PACKS);
        let block = &voices[0];
        assert_eq!(block.test_gates(), [true, true, true, true]);
        assert_eq!(block.test_notes(), [60, 64, 67, 72]);

        for lane in 0..LANES {
            let expected = crate::midi_to_hz(block.test_notes()[lane]);
            let osc1_freq = block.test_osc1_frequency_hz().to_array()[lane];
            assert!(
                (osc1_freq - expected).abs() < 0.1,
                "lane {lane} should keep its own pitch, got {} expected {expected}",
                osc1_freq
            );
        }
    }

    #[test]
    fn pitch_bend_transposes_both_oscillators_by_two_semitones() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::PitchBend { value: 1.0 });

        voices.next();

        let expected = crate::midi_to_hz(62);
        let block = &voices[0];
        let osc1 = block.test_osc1_frequency_hz().to_array()[0];
        assert!(
            (osc1 - expected).abs() < 0.1,
            "osc 1 was {osc1}, expected {expected}"
        );
    }

    #[test]
    fn pitch_bend_remains_available_as_a_mod_matrix_source() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetModulation {
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

        voices.next();

        let block = &voices[0];
        let osc1 = block.test_osc1_frequency_hz().to_array()[0];
        let expected_osc1 = crate::midi_to_hz(68);
        assert!(
            (osc1 - expected_osc1).abs() < 0.1,
            "matrix-routed osc 1 was {osc1}, expected {expected_osc1}"
        );
    }

    #[test]
    fn physical_voices_use_the_rev2_pan_pattern() {
        let voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for voice_index in 0..crate::REV2_VOICE_PAN_POSITIONS.len() {
            let (block, lane) = voices.voice_location(voice_index);
            assert_eq!(
                voices[block].test_pan_positions()[lane],
                crate::REV2_VOICE_PAN_POSITIONS[voice_index]
            );
        }
    }

    #[test]
    fn pan_pattern_scales_with_configured_voice_count() {
        let four_voices = Voices::<1>::new(44_100.0);
        assert_eq!(four_voices[0].test_pan_positions(), [1.0, -1.0, 0.5, -0.5]);

        let eight_voices = Voices::<2>::new(44_100.0);
        assert_eq!(eight_voices[0].test_pan_positions(), [1.0, -1.0, 0.75, -0.75]);
        assert_eq!(eight_voices[1].test_pan_positions(), [0.5, -0.5, 0.25, -0.25]);
    }

    #[test]
    fn lfo_key_sync_resets_only_on_first_held_note() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Depth, 1.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1Rate, 25.0));
        voices.handle_control(ControlMessage::SetParam(ParamId::Lfo1KeySync, 1.0));

        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        process_frames(&mut voices, 64);
        let before_second_note = voices[0].test_lfo_output(0);
        assert!(
            before_second_note.abs() > 0.01,
            "LFO should have advanced before the second note"
        );

        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);
        let after_second_note = voices[0].test_lfo_output(0);
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
        let after_new_first_note = voices[0].test_lfo_output(0);
        assert!(
            after_new_first_note.abs() < 1.0e-6,
            "key sync should reset when a new first held note starts, got {after_new_first_note}"
        );
    }

    #[test]
    fn steals_oldest_voice_when_polyphony_exhausted() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        assert_eq!(voices[0].test_note(0), 60, "old DSP state should fade first");
        assert!(voices[0].has_pending_note(0));
    }

    #[test]
    fn stolen_voice_starts_after_five_millisecond_shutdown() {
        let mut voices = Voices::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
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
        assert_eq!(voices[0].test_note(0), 60);
        assert!(!voices[0].test_gate(0));

        process_frames(&mut voices, 221);
        assert!(voices[0].has_pending_note(0));
        assert_eq!(voices[0].test_note(0), 60);

        process_frames(&mut voices, 1);
        assert!(!voices[0].has_pending_note(0));
        assert_eq!(voices[0].test_note(0), 64);
        assert!(voices[0].test_gate(0));
    }

    #[test]
    fn note_off_cancels_a_pending_stolen_note() {
        let mut voices = Voices::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
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
        assert_ne!(voices[0].test_note(0), 64);
    }

    #[test]
    fn sustain_holds_and_then_cancels_a_pending_stolen_note() {
        let mut voices = Voices::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgAttack, 0.0005));
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
    fn one_voice_pack_limits_polyphony_to_four_voices() {
        let mut voices = Voices::<1>::new(44_100.0);
        for note in [60, 61, 62, 63, 64] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        let held = voices.active_notes();
        assert_eq!(voices.len(), 1);
        assert_eq!(voices.active_voice_count(), LANES);
        assert_eq!(held.len(), LANES);
        assert!(!held.contains(&60), "oldest note should be stolen");
        assert!(held.contains(&64), "new note should be allocated");
    }

    #[test]
    fn allocates_across_voice_blocks() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        for note in [60, 64, 67, 72, 76] {
            voices.handle_control(ControlMessage::NoteOn {
                note,
                velocity: 1.0,
            });
        }

        assert_eq!(voices[0].test_gates(), [true, true, true, true]);
        assert!(voices[1].test_gates().iter().any(|gate| *gate));
        assert_eq!(find_gated_note(&voices, 76), Some((1, 0)));
    }

    #[test]
    fn zero_velocity_note_on_is_note_off() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
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
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);

        voices.handle_control(ControlMessage::ControlChange {
            controller: MIDI_CC_FILTER_CUTOFF,
            value: 0.5,
        });
        voices.handle_control(ControlMessage::ControlChange {
            controller: MIDI_CC_FILTER_RESONANCE,
            value: 0.75,
        });

        let expected_cutoff = (MIN_CUTOFF_HZ * MAX_CUTOFF_HZ).sqrt();
        for block in &voices.blocks {
            assert!((block.test_filter_cutoff() - expected_cutoff).abs() < 0.001);
            assert_eq!(block.test_filter_resonance(), 0.75);
        }
    }

    #[test]
    fn partial_modulation_updates_activate_complete_routes() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetModulationParam {
            route: ModRoute::Free(0),
            parameter: ModulationParam::Source(ModSource::Lfo1),
        });
        voices.handle_control(ControlMessage::SetModulationParam {
            route: ModRoute::Free(0),
            parameter: ModulationParam::Destination(ModDestination::FilterCutoff),
        });
        voices.handle_control(ControlMessage::SetModulationParam {
            route: ModRoute::Free(0),
            parameter: ModulationParam::Amount(0.75),
        });

        for block in &voices.blocks {
            let slot = block.test_mod_matrix_slot(0);
            assert!(slot.enabled);
            assert_eq!(slot.source, ModSource::Lfo1);
            assert_eq!(slot.destination, ModDestination::FilterCutoff);
            assert_eq!(slot.amount, 0.75);
        }
    }

    #[test]
    fn sustain_defers_note_off_until_pedal_release() {
        let mut voices = Voices::<1>::new(44_100.0);
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 1.0,
        });
        voices.handle_control(ControlMessage::SustainPedal { pressed: true });
        voices.handle_control(ControlMessage::NoteOff { note: 60 });

        assert!(voices.active_notes().contains(&60));
        assert!(voices.held_voices.is_empty());
        assert!(voices.sustained_voices[0][0]);

        voices.handle_control(ControlMessage::SustainPedal { pressed: false });

        assert!(!voices[0].test_gate(0));
        assert!(!voices.sustained_voices[0][0]);
    }

    #[test]
    fn pedal_release_keeps_physically_held_notes_gated() {
        let mut voices = Voices::<1>::new(44_100.0);
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
    fn sustained_voice_is_stolen_before_held_voice() {
        let mut voices = Voices::<1>::new(44_100.0);
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
        assert_eq!(active.len(), LANES);
        assert!(!active.contains(&60));
        for note in 61..=64 {
            assert!(active.contains(&note));
        }
    }

    #[test]
    fn retrigger_preserves_physical_voice_pan_position() {
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

        let pan_before = voices[0].test_pan_positions()[0];
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.8,
        });

        assert_eq!(voices[0].test_pan_positions()[0], pan_before);
    }

    #[test]
    fn reuses_fully_silent_lane_after_release() {
        let mut voices = Voices::<{ crate::VOICE_PACKS }>::new(44_100.0);
        voices.handle_control(ControlMessage::SetParam(ParamId::AmpEgRelease, 0.0005));
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
}
