//! Six-lane Prophet-style polyphonic sequencer playback runtime.

use crate::{
    patch::ClockDivision,
    sequencer::{
        clock::{StepClock, StepClockEvent},
        model::{
            LayerSequence, POLY_LANE_COUNT, POLY_STEP_COUNT, PolyLaneStep, PolySequence, PolyStep,
            SequenceUpdate, SequencerType,
        },
    },
};

const PACKED_NOTE_MASK: u16 = 0x00ff;
const PACKED_VELOCITY_SHIFT: u32 = 8;

/// Audio-thread representation of the lossless patch data. Each lane is the
/// Rev2 note byte and velocity byte packed into one word; the masks compile
/// the two playback-wide queries that would otherwise scan typed storage.
struct PolyPlaybackData {
    lanes: [[u16; POLY_LANE_COUNT]; POLY_STEP_COUNT],
    reset_mask: u64,
    tie_masks: [u8; POLY_STEP_COUNT],
}

impl PolyPlaybackData {
    fn compile(poly: &PolySequence) -> Self {
        let mut data = Self {
            lanes: [[0; POLY_LANE_COUNT]; POLY_STEP_COUNT],
            reset_mask: 0,
            tie_masks: [0; POLY_STEP_COUNT],
        };
        for (step, value) in poly.steps.iter().enumerate() {
            for (lane, value) in value.lanes.iter().copied().enumerate() {
                data.lanes[step][lane] = pack_lane(value);
            }
            data.refresh_masks(step);
        }
        data
    }

    fn apply(&mut self, update: SequenceUpdate) -> Option<usize> {
        let step = match update {
            SequenceUpdate::PolyNote { step, lane, value } => {
                let (step, lane) = (usize::from(step), usize::from(lane));
                let packed = self.lanes.get_mut(step)?.get_mut(lane)?;
                *packed = (*packed & !PACKED_NOTE_MASK) | value.rev2_raw();
                step
            }
            SequenceUpdate::PolyVelocity { step, lane, value } => {
                let (step, lane) = (usize::from(step), usize::from(lane));
                let packed = self.lanes.get_mut(step)?.get_mut(lane)?;
                *packed =
                    (*packed & PACKED_NOTE_MASK) | (value.rev2_raw() << PACKED_VELOCITY_SHIFT);
                step
            }
            SequenceUpdate::PolyLaneStep { step, lane, value } => {
                let (step, lane) = (usize::from(step), usize::from(lane));
                *self.lanes.get_mut(step)?.get_mut(lane)? = pack_lane(value);
                step
            }
            SequenceUpdate::Type(_)
            | SequenceUpdate::GatedMode(_)
            | SequenceUpdate::GatedDestination { .. }
            | SequenceUpdate::GatedStep { .. } => return None,
        };
        self.refresh_masks(step);
        Some(step)
    }

    fn replace_step(&mut self, step: usize, value: PolyStep) -> bool {
        let Some(target) = self.lanes.get_mut(step) else {
            return false;
        };
        for (target, value) in target.iter_mut().zip(value.lanes) {
            *target = pack_lane(value);
        }
        self.refresh_masks(step);
        true
    }

    fn refresh_masks(&mut self, step: usize) {
        let mut reset = true;
        let mut ties = 0_u8;
        for (lane, packed) in self.lanes[step].iter().copied().enumerate() {
            let velocity = packed_velocity(packed);
            reset &= velocity == 0;
            if velocity >= 129 && packed_note(packed) == 128 {
                ties |= 1 << lane;
            }
        }
        if reset {
            self.reset_mask |= 1 << step;
        } else {
            self.reset_mask &= !(1 << step);
        }
        self.tie_masks[step] = ties;
    }
}

fn pack_lane(value: PolyLaneStep) -> u16 {
    value.note.rev2_raw() | (value.velocity.rev2_raw() << PACKED_VELOCITY_SHIFT)
}

const fn packed_note(value: u16) -> u8 {
    (value & PACKED_NOTE_MASK) as u8
}

const fn packed_velocity(value: u16) -> u8 {
    (value >> PACKED_VELOCITY_SHIFT) as u8
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PolyNoteOn {
    pub lane: u8,
    pub note: u8,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PolyStepEvent {
    pub note_off: [bool; 6],
    pub note_on: [Option<PolyNoteOn>; 6],
}

impl PolyStepEvent {
    const EMPTY: Self = Self {
        note_off: [false; 6],
        note_on: [None; 6],
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PolyEvent {
    Step(PolyStepEvent),
    GateOff([bool; 6]),
}

pub(crate) struct PolySequencer {
    playback: PolyPlaybackData,
    selected: bool,
    clock: StepClock,
    transport_running: bool,
    step: u8,
    transpose_note: u8,
    active: [bool; 6],
    release_at_half: [bool; 6],
    last_step: Option<u8>,
}

impl PolySequencer {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let poly = PolySequence::default();
        Self {
            playback: PolyPlaybackData::compile(&poly),
            selected: false,
            clock: StepClock::new(sample_rate),
            transport_running: false,
            step: 0,
            transpose_note: 60,
            active: [false; 6],
            release_at_half: [false; 6],
            last_step: None,
        }
    }

    /// Replaces sequence contents without changing transport, cursor, or phase.
    /// Returns lanes whose old sequence-owned notes must be released.
    pub(crate) fn apply_sequence(&mut self, sequence: &LayerSequence) -> [bool; 6] {
        let release = self.active;
        self.active = [false; 6];
        self.release_at_half = [false; 6];
        self.playback = PolyPlaybackData::compile(&sequence.poly);
        self.selected = sequence.sequencer_type == SequencerType::Polyphonic;
        if !self.selected {
            self.transport_running = false;
            self.last_step = None;
        }
        release
    }

    pub(crate) fn apply_update(&mut self, update: SequenceUpdate) -> [bool; 6] {
        let was_poly = self.is_active();
        match update {
            SequenceUpdate::Type(value) => {
                self.selected = value == SequencerType::Polyphonic;
            }
            SequenceUpdate::PolyNote { .. }
            | SequenceUpdate::PolyVelocity { .. }
            | SequenceUpdate::PolyLaneStep { .. } => {
                self.playback.apply(update);
            }
            SequenceUpdate::GatedMode(_)
            | SequenceUpdate::GatedDestination { .. }
            | SequenceUpdate::GatedStep { .. } => {}
        }
        if !self.selected {
            self.transport_running = false;
            self.last_step = None;
        }
        if was_poly && self.is_active() {
            self.refresh_active_release();
        }
        if was_poly && !self.is_active() {
            let release = self.active;
            self.active = [false; 6];
            self.release_at_half = [false; 6];
            release
        } else {
            [false; 6]
        }
    }

    pub(crate) fn replace_step(&mut self, step: u8, value: PolyStep) {
        if self.playback.replace_step(usize::from(step), value) {
            if self.is_active() {
                self.refresh_active_release();
            }
        }
    }

    pub(crate) const fn is_active(&self) -> bool {
        self.transport_running && self.selected
    }

    pub(crate) const fn is_selected(&self) -> bool {
        self.selected
    }

    pub(crate) fn start(&mut self) -> bool {
        if !self.selected {
            return false;
        }
        self.transport_running = true;
        self.step = 0;
        self.transpose_note = 60;
        self.active = [false; 6];
        self.release_at_half = [false; 6];
        self.clock.reset();
        self.last_step = None;
        self.clock.trigger_immediate();
        true
    }

    pub(crate) fn continue_playback(&mut self) -> bool {
        if !self.selected {
            return false;
        }
        self.transport_running = true;
        true
    }

    pub(crate) fn stop(&mut self) -> [bool; 6] {
        let release = self.active;
        self.transport_running = false;
        self.active = [false; 6];
        self.release_at_half = [false; 6];
        release
    }

    pub(crate) fn set_transpose_note(&mut self, note: u8) {
        self.transpose_note = note.min(127);
    }

    pub(crate) fn set_tempo_bpm(&mut self, bpm: f32) {
        self.clock.set_tempo_bpm(bpm);
    }

    pub(crate) fn set_clock_division(&mut self, division: ClockDivision) {
        self.clock.set_division(division);
    }

    pub(crate) fn set_external_clock(&mut self, external: bool) {
        self.clock.set_external(external);
    }

    pub(crate) fn midi_clock_tick(&mut self) {
        if self.transport_running {
            self.clock.midi_tick();
        }
    }

    pub(crate) const fn last_step(&self) -> Option<u8> {
        self.last_step
    }

    pub(crate) fn advance(&mut self) -> Option<PolyEvent> {
        if !self.is_active() {
            return None;
        }
        match self.clock.advance(1) {
            Some(StepClockEvent::Boundary) => Some(PolyEvent::Step(self.advance_step())),
            Some(StepClockEvent::Half) => {
                let release = self.release_at_half;
                for (lane, should_release) in release.iter().copied().enumerate() {
                    if should_release {
                        self.active[lane] = false;
                    }
                }
                self.release_at_half = [false; 6];
                if release.iter().any(|value| *value) {
                    Some(PolyEvent::GateOff(release))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    fn advance_step(&mut self) -> PolyStepEvent {
        let mut event = PolyStepEvent::EMPTY;
        let mut step_index = usize::from(self.step).min(63);
        if self.is_step_reset(step_index) {
            for lane in 0..6 {
                event.note_off[lane] = self.active[lane];
                self.active[lane] = false;
            }
            step_index = 0;
            if self.is_step_reset(0) {
                self.step = 0;
                self.release_at_half = [false; 6];
                return event;
            }
        }

        let step = self.playback.lanes[step_index];
        self.last_step = Some(step_index as u8);
        let next_index = self.next_effective_step(step_index);
        let next_ties = self.playback.tie_masks[next_index];
        for lane in 0..6 {
            let packed = step[lane];
            let velocity = packed_velocity(packed);
            if velocity <= 128 {
                event.note_off[lane] = self.active[lane];
                self.active[lane] = false;
                self.release_at_half[lane] = false;
                continue;
            }
            let note = packed_note(packed);
            if note == 128 {
                // A tie occupies the entire step: the following step decides
                // whether the held note is released, replaced, or tied again.
                self.release_at_half[lane] = false;
                continue;
            }
            event.note_off[lane] = self.active[lane];
            let transposed =
                (i16::from(note) + i16::from(self.transpose_note) - 60).clamp(0, 127) as u8;
            event.note_on[lane] = Some(PolyNoteOn {
                lane: lane as u8,
                note: transposed,
                velocity: f32::from(velocity - 128) / 127.0,
            });
            self.active[lane] = true;
            self.release_at_half[lane] = next_ties & (1 << lane) == 0;
        }
        self.step = ((step_index + 1) % 64) as u8;
        event
    }

    fn next_effective_step(&self, current: usize) -> usize {
        let next = (current + 1) % 64;
        if self.is_step_reset(next) { 0 } else { next }
    }

    fn refresh_active_release(&mut self) {
        let candidate = usize::from(self.step).min(63);
        let next = if self.is_step_reset(candidate) {
            0
        } else {
            candidate
        };
        let next_ties = self.playback.tie_masks[next];
        for lane in 0..POLY_LANE_COUNT {
            if !self.active[lane] {
                continue;
            }
            let current_is_tie = self
                .last_step
                .is_some_and(|step| self.playback.tie_masks[usize::from(step)] & (1 << lane) != 0);
            self.release_at_half[lane] = !current_is_tie && next_ties & (1 << lane) == 0;
        }
    }

    const fn is_step_reset(&self, step: usize) -> bool {
        self.playback.reset_mask & (1 << step) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        patch::ClockDivision,
        sequencer::model::{PolyLaneStep, PolyNote, PolyStep, PolyVelocity},
    };

    #[test]
    fn compiled_runtime_has_bounded_footprint() {
        assert!(core::mem::size_of::<PolySequencer>() <= 900);
        assert!(core::mem::size_of::<PolySequencer>() < core::mem::size_of::<PolySequence>());
    }

    #[test]
    fn split_edits_preserve_inactive_lane_fields_in_packed_playback() {
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Polyphonic;
        sequence.poly.steps[0].lanes[0] = PolyLaneStep {
            note: PolyNote::Note(37),
            velocity: PolyVelocity::Rest,
        };
        let mut engine = PolySequencer::new(100.0);
        engine.apply_sequence(&sequence);

        engine.apply_update(SequenceUpdate::PolyNote {
            step: 0,
            lane: 0,
            value: PolyNote::Note(42),
        });
        let packed = engine.playback.lanes[0][0];
        assert_eq!(packed_note(packed), 42);
        assert_eq!(packed_velocity(packed), 128);

        engine.apply_update(SequenceUpdate::PolyVelocity {
            step: 0,
            lane: 0,
            value: PolyVelocity::Velocity(100),
        });
        let packed = engine.playback.lanes[0][0];
        assert_eq!(packed_note(packed), 42);
        assert_eq!(packed_velocity(packed), 228);
        assert!(!engine.is_step_reset(0));

        assert!(engine.start());
        let PolyEvent::Step(event) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(event.note_on[0].unwrap().note, 42);
    }

    #[test]
    fn split_velocity_edit_restores_a_stored_tie() {
        let steps = [
            lane_step(note(60, 100)),
            lane_step(PolyLaneStep {
                note: PolyNote::Tie,
                velocity: PolyVelocity::Rest,
            }),
        ];
        let mut engine = started(&patch(&steps));
        let PolyEvent::Step(first) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(first.note_on[0].is_some());
        engine.apply_update(SequenceUpdate::PolyVelocity {
            step: 1,
            lane: 0,
            value: PolyVelocity::Velocity(127),
        });

        for _ in 1..50 {
            assert!(!matches!(
                engine.advance(),
                Some(PolyEvent::GateOff(release)) if release[0]
            ));
        }
        let PolyEvent::Step(tie) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(!tie.note_off[0]);
        assert!(tie.note_on[0].is_none());
        assert!(engine.active[0]);
    }

    #[test]
    fn note_tie_tie_rest_has_sample_exact_event_trace() {
        let steps = [
            lane_step(note(60, 100)),
            lane_step(tie()),
            lane_step(tie()),
            lane_step(PolyLaneStep {
                note: PolyNote::Note(60),
                velocity: PolyVelocity::Rest,
            }),
        ];
        let sequence = patch(&steps);
        let mut engine = PolySequencer::new(8.0);
        engine.apply_sequence(&sequence);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        engine.start();
        let mut trace = heapless::Vec::<(usize, u8), 8>::new();
        for sample in 0..=16 {
            match engine.advance() {
                Some(PolyEvent::Step(event)) if event.note_on[0].is_some() => {
                    trace.push((sample, 1)).unwrap();
                }
                Some(PolyEvent::Step(event)) if event.note_off[0] => {
                    trace.push((sample, 4)).unwrap();
                }
                Some(PolyEvent::Step(_)) => trace.push((sample, 2)).unwrap(),
                Some(PolyEvent::GateOff(release)) if release[0] => {
                    trace.push((sample, 3)).unwrap();
                }
                _ => {}
            }
        }
        assert_eq!(
            trace.as_slice(),
            &[(0, 1), (4, 2), (8, 2), (12, 4), (16, 1)]
        );
    }

    #[test]
    fn editing_the_next_step_to_tie_refreshes_the_active_note_gate() {
        let steps = [
            lane_step(note(60, 100)),
            lane_step(note(62, 100)),
            lane_step(PolyLaneStep {
                note: PolyNote::Note(60),
                velocity: PolyVelocity::Rest,
            }),
        ];
        let sequence = patch(&steps);
        let mut engine = PolySequencer::new(8.0);
        engine.apply_sequence(&sequence);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        engine.start();

        let PolyEvent::Step(first) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(first.note_on[0].is_some());
        engine.apply_update(SequenceUpdate::PolyLaneStep {
            step: 1,
            lane: 0,
            value: tie(),
        });

        for _ in 1..4 {
            assert!(!matches!(
                engine.advance(),
                Some(PolyEvent::GateOff(release)) if release[0]
            ));
        }
        let PolyEvent::Step(tie) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(!tie.note_off[0]);
        assert!(tie.note_on[0].is_none());
        assert!(engine.active[0]);
    }

    #[test]
    fn recording_a_tie_into_the_next_step_refreshes_the_active_note_gate() {
        let steps = [
            lane_step(note(60, 100)),
            lane_step(note(62, 100)),
            lane_step(PolyLaneStep {
                note: PolyNote::Note(60),
                velocity: PolyVelocity::Rest,
            }),
        ];
        let sequence = patch(&steps);
        let mut engine = PolySequencer::new(8.0);
        engine.apply_sequence(&sequence);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        engine.start();

        let PolyEvent::Step(first) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(first.note_on[0].is_some());
        engine.replace_step(1, lane_step(tie()));

        for _ in 1..4 {
            assert!(!matches!(
                engine.advance(),
                Some(PolyEvent::GateOff(release)) if release[0]
            ));
        }
        let PolyEvent::Step(tie) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(!tie.note_off[0]);
        assert!(tie.note_on[0].is_none());
        assert!(engine.active[0]);
    }

    #[test]
    fn an_update_during_a_tie_does_not_restore_the_half_step_release() {
        let steps = [
            lane_step(note(60, 100)),
            lane_step(tie()),
            lane_step(PolyLaneStep {
                note: PolyNote::Note(60),
                velocity: PolyVelocity::Rest,
            }),
        ];
        let sequence = patch(&steps);
        let mut engine = PolySequencer::new(8.0);
        engine.apply_sequence(&sequence);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        engine.start();

        assert!(matches!(engine.advance(), Some(PolyEvent::Step(_))));
        for _ in 1..4 {
            assert_eq!(engine.advance(), None);
        }
        assert!(matches!(engine.advance(), Some(PolyEvent::Step(_))));
        engine.apply_update(SequenceUpdate::PolyNote {
            step: 2,
            lane: 1,
            value: PolyNote::Note(72),
        });

        for _ in 5..8 {
            assert!(!matches!(
                engine.advance(),
                Some(PolyEvent::GateOff(release)) if release[0]
            ));
        }
        let Some(PolyEvent::Step(rest)) = engine.advance() else {
            panic!()
        };
        assert!(rest.note_off[0]);
    }

    #[test]
    fn transpose_clamps_at_both_midi_boundaries() {
        let sequence = patch(&[lane_step(note(0, 127))]);
        let mut engine = started(&sequence);
        engine.set_transpose_note(0);
        let PolyEvent::Step(low) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(low.note_on[0].unwrap().note, 0);
        engine.stop();

        let sequence = patch(&[lane_step(note(127, 127))]);
        engine.apply_sequence(&sequence);
        engine.start();
        engine.set_transpose_note(127);
        let PolyEvent::Step(high) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(high.note_on[0].unwrap().note, 127);
    }

    #[test]
    fn play_resets_transposition_to_middle_c() {
        let sequence = patch(&[lane_step(note(60, 127))]);
        let mut engine = started(&sequence);
        engine.set_transpose_note(72);
        engine.stop();
        engine.start();

        let PolyEvent::Step(event) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(event.note_on[0].unwrap().note, 60);
    }

    #[test]
    fn stop_retains_position_and_continue_does_not_restart() {
        let sequence = patch(&[lane_step(note(60, 127)), lane_step(note(62, 127))]);
        let mut engine = started(&sequence);
        let PolyEvent::Step(first) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(first.note_on[0].unwrap().note, 60);
        assert_eq!(engine.step, 1);

        let released = engine.stop();
        assert!(released[0]);
        assert_eq!(engine.step, 1);
        assert!(engine.continue_playback());
        assert_eq!(engine.step, 1);

        for _ in 0..49 {
            let _ = engine.advance();
        }
        let PolyEvent::Step(second) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(second.note_on[0].unwrap().note, 62);
    }

    #[test]
    fn stopped_external_transport_does_not_buffer_clock_steps() {
        let sequence = patch(&[lane_step(note(60, 127)), lane_step(note(62, 127))]);
        let mut engine = PolySequencer::new(48_000.0);
        engine.apply_sequence(&sequence);
        engine.set_clock_division(ClockDivision::Sixteenth);
        engine.set_external_clock(true);
        assert!(engine.start());
        assert!(matches!(engine.advance(), Some(PolyEvent::Step(_))));
        engine.stop();
        for _ in 0..6 {
            engine.midi_clock_tick();
        }
        assert!(engine.continue_playback());
        assert_eq!(engine.advance(), None);
        for _ in 0..6 {
            engine.midi_clock_tick();
        }
        assert!(matches!(engine.advance(), Some(PolyEvent::Step(_))));
    }

    #[test]
    fn leading_tie_and_empty_sequence_never_create_notes() {
        for sequence in [patch(&[lane_step(tie())]), {
            let mut empty = LayerSequence::default();
            empty.sequencer_type = SequencerType::Polyphonic;
            empty
        }] {
            let mut engine = started(&sequence);
            let PolyEvent::Step(event) = engine.advance().unwrap() else {
                panic!()
            };
            assert!(event.note_on.iter().all(Option::is_none));
        }
    }

    #[test]
    fn patch_swap_preserves_running_clock_phase_but_releases_ownership() {
        let first = patch(&[lane_step(note(60, 127))]);
        let second = patch(&[lane_step(note(72, 127))]);
        let mut engine = started(&first);
        let _ = engine.advance();
        for _ in 0..10 {
            let _ = engine.advance();
        }
        assert_eq!(engine.apply_sequence(&second)[0], true);
        assert!(engine.is_active());
        for _ in 0..39 {
            assert_eq!(engine.advance(), None);
        }
        assert!(matches!(engine.advance(), Some(PolyEvent::Step(_))));
    }

    #[test]
    fn reset_loops_correctly_from_every_nonzero_position() {
        for reset_at in 1..64 {
            let mut sequence = patch(&[lane_step(note(60, 127))]);
            for step in 1..reset_at {
                sequence.poly.steps[step] = lane_step(PolyLaneStep {
                    note: PolyNote::Note(60),
                    velocity: PolyVelocity::Rest,
                });
            }
            let mut engine = started(&sequence);
            let _ = engine.advance();
            for _ in 1..reset_at {
                let _ = engine.advance_step();
            }
            let looped = engine.advance_step();
            assert!(looped.note_on[0].is_some(), "reset at {reset_at}");
        }
    }

    #[test]
    fn one_step_can_emit_all_six_lanes() {
        let mut step = lane_step(note(60, 1));
        for lane in 0..6 {
            step.lanes[lane] = note(60 + lane as u8, 127 - lane as u8);
        }
        let mut engine = started(&patch(&[step]));
        let PolyEvent::Step(event) = engine.advance().unwrap() else {
            panic!()
        };
        assert_eq!(event.note_on.iter().flatten().count(), 6);
        assert_eq!(event.note_on[5].unwrap().note, 65);
    }

    #[test]
    fn reset_in_an_unused_lane_does_not_hide_notes_in_other_lanes() {
        let mut step = PolyStep::default();
        step.lanes[1] = note(75, 91);
        let mut engine = started(&patch(&[step]));

        let PolyEvent::Step(event) = engine.advance().unwrap() else {
            panic!()
        };
        assert!(event.note_on[0].is_none());
        assert_eq!(event.note_on[1].unwrap().note, 75);
    }

    fn note(note: u8, velocity: u8) -> PolyLaneStep {
        PolyLaneStep {
            note: PolyNote::Note(note),
            velocity: PolyVelocity::Velocity(velocity),
        }
    }

    fn tie() -> PolyLaneStep {
        PolyLaneStep {
            note: PolyNote::Tie,
            velocity: PolyVelocity::Velocity(127),
        }
    }

    fn patch(steps: &[PolyStep]) -> LayerSequence {
        let mut patch = LayerSequence::default();
        patch.sequencer_type = SequencerType::Polyphonic;
        for (index, step) in steps.iter().copied().enumerate() {
            patch.poly.steps[index] = step;
        }
        patch
    }

    fn lane_step(value: PolyLaneStep) -> PolyStep {
        let mut step = PolyStep::default();
        step.lanes.fill(PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Rest,
        });
        step.lanes[0] = value;
        step
    }

    fn started(sequence: &LayerSequence) -> PolySequencer {
        let mut engine = PolySequencer::new(100.0);
        engine.apply_sequence(sequence);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        assert!(engine.start());
        engine
    }
}
