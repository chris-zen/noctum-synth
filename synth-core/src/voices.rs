//! Polyphonic voice allocation and mixing.

use crate::effects::EffectModulation;
use crate::filter::{MAX_CUTOFF_HZ, MIN_CUTOFF_HZ};
use crate::fixed_index_list::FixedIndexList;
#[cfg(all(feature = "profiling", test))]
use crate::profiling::NoopProfiler;
#[cfg(feature = "profiling")]
use crate::profiling::RenderProfiler;
use crate::voice::PerformanceModulation;
use crate::{
    ControlMessage, FilterOversampling, FilterType, LANES, LfoWaveform, ModDestination, ParamId,
    VOICE_PACKS, VoiceBlock, Waveform,
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
    next_pan_side: f32,
    performance: PerformanceModulation,
    last_effect_modulation: EffectModulation,
}

impl<const PACKS: usize> Voices<PACKS> {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            blocks: core::array::from_fn(|_| VoiceBlock::new(sample_rate)),
            held_voices: FixedIndexList::new(),
            sustained_voices: [[false; LANES]; PACKS],
            sustain_pressed: false,
            next_voice: 0,
            next_pan_side: -1.0,
            performance: PerformanceModulation::default(),
            last_effect_modulation: EffectModulation::default(),
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
                let pan_side = if active_voice_idx.is_some() {
                    self.blocks[block_idx].pan_sides[lane]
                } else {
                    let pan_side = self.next_pan_side;
                    self.next_pan_side = -self.next_pan_side;
                    pan_side
                };
                self.blocks[block_idx].note_on(
                    lane,
                    note,
                    velocity.clamp(0.0, 1.0),
                    pan_side,
                    reset_key_synced_lfos,
                );
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
                            block.filter.set_resonance(value);
                        }
                    }
                    MIDI_CC_FILTER_CUTOFF => {
                        let cutoff = midi_filter_cutoff_hz(value);
                        for block in &mut self.blocks {
                            block.filter.set_cutoff(cutoff);
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
                if block.notes[lane] == note && block.gates[lane] {
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
            if block.gates[lane] && block.notes[lane] == note {
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
            match id {
                ParamId::Osc1Waveform => {
                    block
                        .oscillators
                        .set_osc1_waveform(int_to_waveform(value as i32));
                }
                ParamId::Osc1Enabled => block.oscillators.set_osc1_enabled(value >= 0.5),
                ParamId::Osc2Waveform => {
                    block
                        .oscillators
                        .set_osc2_waveform(int_to_waveform(value as i32));
                }
                ParamId::Osc2Enabled => block.oscillators.set_osc2_enabled(value >= 0.5),
                ParamId::Osc1Frequency => {
                    block.set_osc1_note_param(value);
                }
                ParamId::Osc2Frequency => {
                    block.set_osc2_note_param(value);
                }
                ParamId::Osc1FineTune => {
                    block.set_osc1_fine(value);
                }
                ParamId::Osc2FineTune => {
                    block.set_osc2_fine(value);
                }
                ParamId::Osc1Shape => {
                    block.oscillators.set_osc1_shape_mod(value);
                }
                ParamId::Osc2Shape => {
                    block.oscillators.set_osc2_shape_mod(value);
                }
                ParamId::Osc1Level => {
                    if value <= 0.0 {
                        block.oscillators.set_mix(1.0);
                    }
                }
                ParamId::Osc2Level => {
                    if value <= 0.0 {
                        block.oscillators.set_mix(0.0);
                    }
                }
                ParamId::OscMix => block.oscillators.set_mix(value),
                ParamId::SubOscLevel => block.oscillators.set_sub_octave(value),
                ParamId::NoiseLevel => block.oscillators.set_noise(value),
                ParamId::HardSync => block.oscillators.set_sync(value >= 0.5),
                ParamId::OscSlop | ParamId::AnalogDrift => {
                    block.oscillators.set_slop(value);
                }
                ParamId::Osc1NoteReset => block.oscillators.set_osc1_note_reset(value >= 0.5),
                ParamId::Osc2NoteReset => block.oscillators.set_osc2_note_reset(value >= 0.5),
                ParamId::Osc1KeyboardOn => {
                    block.oscillators.set_osc1_keyboard_on(value >= 0.5);
                }
                ParamId::Osc2KeyboardOn => {
                    block.oscillators.set_osc2_keyboard_on(value >= 0.5);
                }
                ParamId::FilterCutoff => {
                    block.filter.set_cutoff(value);
                }
                ParamId::FilterResonance => block.filter.set_resonance(value),
                ParamId::FilterPoles => block.filter.set_poles(if value < 0.5 { 2 } else { 4 }),
                ParamId::FilterKeyTrack => block.filter.set_key_track(value),
                ParamId::FilterEnvAmount => block.filter.set_env_amount(value),
                ParamId::FilterVelocity => block.filter.set_env_velocity_amount(value),
                ParamId::FilterAudioMod => block.filter.set_audio_mod(value),
                ParamId::FilterEgDelay => block.set_filter_delay(value),
                ParamId::FilterEgAttack => block.set_filter_attack(value),
                ParamId::FilterEgDecay => block.set_filter_decay(value),
                ParamId::FilterEgSustain => block.set_filter_sustain(value),
                ParamId::FilterEgRelease => block.set_filter_release(value),
                ParamId::AmpEnvAmount => block.set_amp_env_amount(value),
                ParamId::AmpVelocity => block.set_amp_velocity_amount(value),
                ParamId::AmpEgDelay => block.set_amp_delay(value),
                ParamId::AmpEgAttack => block.set_amp_attack(value),
                ParamId::AmpEgDecay => block.set_amp_decay(value),
                ParamId::AmpEgSustain => block.set_amp_sustain(value),
                ParamId::AmpEgRelease => block.set_amp_release(value),
                ParamId::AuxEgDestination => {
                    block.set_aux_destination(ModDestination::from_index(value as usize));
                }
                ParamId::AuxEgAmount => block.set_aux_amount(value),
                ParamId::AuxEgVelocity => block.set_aux_velocity_amount(value),
                ParamId::AuxEgDelay => block.set_aux_delay(value),
                ParamId::AuxEgAttack => block.set_aux_attack(value),
                ParamId::AuxEgDecay => block.set_aux_decay(value),
                ParamId::AuxEgSustain => block.set_aux_sustain(value),
                ParamId::AuxEgRelease => block.set_aux_release(value),
                ParamId::AuxEgLoop => block.set_aux_repeat(value >= 0.5),
                ParamId::PanSpread => block.set_pan_spread(value),
                ParamId::Lfo1Rate => block.set_lfo_rate_hz(0, value),
                ParamId::Lfo2Rate => block.set_lfo_rate_hz(1, value),
                ParamId::Lfo3Rate => block.set_lfo_rate_hz(2, value),
                ParamId::Lfo4Rate => block.set_lfo_rate_hz(3, value),
                ParamId::Lfo1Depth => block.set_lfo_depth(0, value),
                ParamId::Lfo2Depth => block.set_lfo_depth(1, value),
                ParamId::Lfo3Depth => block.set_lfo_depth(2, value),
                ParamId::Lfo4Depth => block.set_lfo_depth(3, value),
                ParamId::Lfo1Waveform => {
                    block.set_lfo_waveform(0, int_to_lfo_waveform(value as i32))
                }
                ParamId::Lfo2Waveform => {
                    block.set_lfo_waveform(1, int_to_lfo_waveform(value as i32))
                }
                ParamId::Lfo3Waveform => {
                    block.set_lfo_waveform(2, int_to_lfo_waveform(value as i32))
                }
                ParamId::Lfo4Waveform => {
                    block.set_lfo_waveform(3, int_to_lfo_waveform(value as i32))
                }
                ParamId::Lfo1Destination => {
                    block.set_lfo_destination(0, ModDestination::from_index(value as usize));
                }
                ParamId::Lfo2Destination => {
                    block.set_lfo_destination(1, ModDestination::from_index(value as usize));
                }
                ParamId::Lfo3Destination => {
                    block.set_lfo_destination(2, ModDestination::from_index(value as usize));
                }
                ParamId::Lfo4Destination => {
                    block.set_lfo_destination(3, ModDestination::from_index(value as usize));
                }
                ParamId::Lfo1ClockSync => block.set_lfo_clock_sync(0, value >= 0.5),
                ParamId::Lfo2ClockSync => block.set_lfo_clock_sync(1, value >= 0.5),
                ParamId::Lfo3ClockSync => block.set_lfo_clock_sync(2, value >= 0.5),
                ParamId::Lfo4ClockSync => block.set_lfo_clock_sync(3, value >= 0.5),
                ParamId::Lfo1KeySync => block.set_lfo_key_sync(0, value >= 0.5),
                ParamId::Lfo2KeySync => block.set_lfo_key_sync(1, value >= 0.5),
                ParamId::Lfo3KeySync => block.set_lfo_key_sync(2, value >= 0.5),
                ParamId::Lfo4KeySync => block.set_lfo_key_sync(3, value >= 0.5),
                ParamId::MasterVolume => {}
                ParamId::Osc1Glide | ParamId::Osc2Glide | ParamId::GlideTime => {}
                _ => {}
            }
        }
    }
}

fn midi_filter_cutoff_hz(value: f32) -> f32 {
    MIN_CUTOFF_HZ * crate::math::powf(MAX_CUTOFF_HZ / MIN_CUTOFF_HZ, value)
}

fn int_to_waveform(value: i32) -> Waveform {
    match value {
        0 => Waveform::Saw,
        1 => Waveform::SawTri,
        2 => Waveform::Triangle,
        3 => Waveform::Pulse,
        _ => Waveform::Saw,
    }
}

fn int_to_lfo_waveform(value: i32) -> LfoWaveform {
    match value {
        0 => LfoWaveform::Triangle,
        1 => LfoWaveform::Saw,
        2 => LfoWaveform::ReverseSaw,
        3 => LfoWaveform::Square,
        4 => LfoWaveform::SampleAndHold,
        _ => LfoWaveform::Triangle,
    }
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
                block.last_effect_modulation = EffectModulation::default();
                continue;
            }
            block.age_active_lanes();
            #[cfg(feature = "profiling")]
            let (block_left, block_right) = block.next_profiled(self.performance, profiler);
            #[cfg(not(feature = "profiling"))]
            let (block_left, block_right) = block.next(self.performance);
            left += block_left;
            right += block_right;
            effects.add(block.last_effect_modulation);
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
    use crate::{ModRoute, ModSource, ModulationParam, ParamId, VOICE_COUNT};

    fn process_frames(voices: &mut Voices, frames: usize) {
        for _ in 0..frames {
            voices.next();
        }
    }

    fn find_gated_note(voices: &Voices, note: u8) -> Option<(usize, usize)> {
        for (block_idx, block) in voices.iter().enumerate() {
            for lane in 0..LANES {
                if block.gates[lane] && block.notes[lane] == note {
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
        assert_eq!(block.gates, [true, true, true, true]);
        assert_eq!(block.notes, [60, 64, 67, 72]);

        for lane in 0..LANES {
            let expected = crate::midi_to_hz(block.notes[lane]);
            let osc1_freq = block.oscillators.osc1_frequency_hz().to_array()[lane];
            assert!(
                (osc1_freq - expected).abs() < 0.1,
                "lane {lane} should keep its own pitch, got {} expected {expected}",
                osc1_freq
            );
        }
    }

    #[test]
    fn pan_spread_assigns_new_voices_to_alternating_sides() {
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

        assert_eq!(voices[0].pan_sides[0], -1.0);
        assert_eq!(voices[0].pan_sides[1], 1.0);
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
        let before_second_note = voices[0].last_lfo_outputs[0].to_array()[0];
        assert!(
            before_second_note.abs() > 0.01,
            "LFO should have advanced before the second note"
        );

        voices.handle_control(ControlMessage::NoteOn {
            note: 64,
            velocity: 1.0,
        });
        process_frames(&mut voices, 1);
        let after_second_note = voices[0].last_lfo_outputs[0].to_array()[0];
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
        let after_new_first_note = voices[0].last_lfo_outputs[0].to_array()[0];
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
            voices[0].notes[0], 76,
            "stolen voice should be reused for note 76"
        );
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

        assert_eq!(voices[0].gates, [true, true, true, true]);
        assert!(voices[1].gates.iter().any(|gate| *gate));
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
            assert!((block.filter.cutoff() - expected_cutoff).abs() < 0.001);
            assert_eq!(block.filter.resonance(), 0.75);
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
            let slot = block.mod_matrix_slots[0];
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

        assert!(!voices[0].gates[0]);
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
    fn retrigger_preserves_pan_side() {
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

        let pan_before = voices[0].pan_sides[0];
        voices.handle_control(ControlMessage::NoteOn {
            note: 60,
            velocity: 0.8,
        });

        assert_eq!(voices[0].pan_sides[0], pan_before);
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
