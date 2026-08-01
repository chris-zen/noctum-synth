//! Fixed-capacity MIDI chord step recorder.

use heapless::{Deque, Vec};

use crate::{
    math::F32,
    sequencer::model::{PolyLaneStep, PolyNote, PolyStep, PolyVelocity, SequencerRecordCommand},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordedNote {
    note: u8,
    velocity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecorderEvent {
    Status { recording: bool, cursor: u8 },
    StepChanged { step: u8, value: PolyStep },
    Overflow { cursor: u8 },
}

pub(crate) struct StepRecorder {
    recording: bool,
    cursor: u8,
    physical_held: [u64; 2],
    chord: Vec<RecordedNote, 6>,
    overflow_latched: bool,
    events: Deque<RecorderEvent, 8>,
}

impl Default for StepRecorder {
    fn default() -> Self {
        Self {
            recording: false,
            cursor: 0,
            physical_held: [0; 2],
            chord: Vec::new(),
            overflow_latched: false,
            events: Deque::new(),
        }
    }
}

impl StepRecorder {
    pub(crate) const fn is_recording(&self) -> bool {
        self.recording
    }

    pub(crate) fn command(&mut self, command: SequencerRecordCommand) {
        match command {
            SequencerRecordCommand::Start => {
                self.recording = true;
                self.cancel_pending();
                self.push(RecorderEvent::Status {
                    recording: true,
                    cursor: self.cursor,
                });
            }
            SequencerRecordCommand::Stop => {
                self.recording = false;
                self.cancel_pending();
                self.push(RecorderEvent::Status {
                    recording: false,
                    cursor: self.cursor,
                });
            }
            SequencerRecordCommand::SetCursor(cursor) => {
                self.cursor = cursor.min(63);
                self.cancel_pending();
                self.status();
            }
            SequencerRecordCommand::MoveCursor(delta) => {
                self.cursor = (i16::from(self.cursor) + i16::from(delta)).rem_euclid(64) as u8;
                self.cancel_pending();
                self.status();
            }
            SequencerRecordCommand::InsertRest => {
                self.write_step(rest_step(), true);
            }
            SequencerRecordCommand::InsertTie => {
                self.write_step(tie_step(), true);
            }
            SequencerRecordCommand::InsertReset => {
                self.write_step(PolyStep::default(), true);
            }
            SequencerRecordCommand::ClearStep => {
                self.write_step(rest_step(), false);
            }
        }
    }

    pub(crate) fn note_on(&mut self, note: u8, velocity: f32) {
        if !self.recording || note >= 128 || self.is_held(note) {
            return;
        }
        self.set_held(note, true);
        let raw_velocity = F32(velocity.clamp(0.0, 1.0) * 127.0).round().as_f32() as u8;
        if self.chord.len() < 6 {
            self.chord
                .push(RecordedNote {
                    note,
                    velocity: raw_velocity.max(1),
                })
                .ok();
        } else if !self.overflow_latched {
            self.overflow_latched = true;
            self.push(RecorderEvent::Overflow {
                cursor: self.cursor,
            });
        }
    }

    pub(crate) fn note_off(&mut self, note: u8) {
        if !self.recording || note >= 128 || !self.is_held(note) {
            return;
        }
        self.set_held(note, false);
        if self.physical_held == [0; 2] && !self.chord.is_empty() {
            let mut step = rest_step();
            for (lane, recorded) in self.chord.iter().copied().enumerate() {
                step.lanes[lane] = PolyLaneStep {
                    note: PolyNote::Note(recorded.note),
                    velocity: PolyVelocity::Velocity(recorded.velocity),
                };
            }
            self.write_step(step, true);
            self.cancel_pending();
        }
    }

    pub(crate) fn cancel_pending(&mut self) {
        self.physical_held = [0; 2];
        self.chord.clear();
        self.overflow_latched = false;
    }

    pub(crate) fn pop_event(&mut self) -> Option<RecorderEvent> {
        self.events.pop_front()
    }

    fn write_step(&mut self, value: PolyStep, advance: bool) {
        let step = self.cursor;
        self.push(RecorderEvent::StepChanged { step, value });
        if advance {
            self.cursor = (self.cursor + 1) % 64;
        }
        self.status();
    }

    fn status(&mut self) {
        self.push(RecorderEvent::Status {
            recording: self.recording,
            cursor: self.cursor,
        });
    }

    fn push(&mut self, event: RecorderEvent) {
        if self.events.push_back(event).is_err() {
            let _ = self.events.pop_front();
            let _ = self.events.push_back(event);
        }
    }

    fn is_held(&self, note: u8) -> bool {
        let index = usize::from(note / 64);
        self.physical_held[index] & (1_u64 << (note % 64)) != 0
    }

    fn set_held(&mut self, note: u8, held: bool) {
        let index = usize::from(note / 64);
        let mask = 1_u64 << (note % 64);
        if held {
            self.physical_held[index] |= mask;
        } else {
            self.physical_held[index] &= !mask;
        }
    }
}

fn rest_step() -> PolyStep {
    PolyStep {
        lanes: [PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Rest,
        }; 6],
    }
}

fn tie_step() -> PolyStep {
    PolyStep {
        lanes: [PolyLaneStep {
            note: PolyNote::Tie,
            velocity: PolyVelocity::Velocity(127),
        }; 6],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_commits_only_after_last_physical_release() {
        let mut recorder = StepRecorder::default();
        recorder.command(SequencerRecordCommand::Start);
        drain(&mut recorder);
        recorder.note_on(60, 0.5);
        recorder.note_on(64, 1.0);
        recorder.note_off(60);
        assert!(drain(&mut recorder).is_empty());
        recorder.note_off(64);
        let events = drain(&mut recorder);
        let RecorderEvent::StepChanged { step, value } = events[0] else {
            panic!()
        };
        assert_eq!(step, 0);
        assert_eq!(value.lanes[0].note, PolyNote::Note(60));
        assert_eq!(value.lanes[0].velocity, PolyVelocity::Velocity(64));
        assert_eq!(value.lanes[1].note, PolyNote::Note(64));
        assert!(matches!(events[1], RecorderEvent::Status { cursor: 1, .. }));
    }

    #[test]
    fn repeated_notes_are_unique_and_seventh_note_reports_one_overflow() {
        let mut recorder = StepRecorder::default();
        recorder.command(SequencerRecordCommand::Start);
        drain(&mut recorder);
        recorder.note_on(60, 0.1);
        recorder.note_on(60, 1.0);
        for note in 61..=67 {
            recorder.note_on(note, 1.0);
        }
        assert_eq!(
            drain(&mut recorder).as_slice(),
            &[RecorderEvent::Overflow { cursor: 0 }]
        );
        for note in 60..=67 {
            recorder.note_off(note);
        }
        let events = drain(&mut recorder);
        let RecorderEvent::StepChanged { value, .. } = events[0] else {
            panic!()
        };
        assert_eq!(
            value
                .lanes
                .iter()
                .filter(|lane| matches!(lane.velocity, PolyVelocity::Velocity(_)))
                .count(),
            6
        );
    }

    #[test]
    fn stop_cancels_partial_chord_and_cursor_commands_wrap() {
        let mut recorder = StepRecorder::default();
        recorder.command(SequencerRecordCommand::Start);
        recorder.note_on(60, 1.0);
        recorder.command(SequencerRecordCommand::Stop);
        recorder.note_off(60);
        assert!(
            !drain(&mut recorder)
                .iter()
                .any(|event| matches!(event, RecorderEvent::StepChanged { .. }))
        );
        recorder.command(SequencerRecordCommand::SetCursor(63));
        recorder.command(SequencerRecordCommand::MoveCursor(1));
        assert!(
            drain(&mut recorder)
                .iter()
                .any(|event| matches!(event, RecorderEvent::Status { cursor: 0, .. }))
        );
    }

    #[test]
    fn deliberate_edits_have_distinct_reset_tie_and_clear_encodings() {
        let mut recorder = StepRecorder::default();
        recorder.command(SequencerRecordCommand::InsertTie);
        recorder.command(SequencerRecordCommand::InsertReset);
        recorder.command(SequencerRecordCommand::ClearStep);
        let events = drain(&mut recorder);
        let changed: heapless::Vec<_, 3> = events
            .iter()
            .filter_map(|event| match event {
                RecorderEvent::StepChanged { step, value } => Some((*step, *value)),
                _ => None,
            })
            .collect();
        assert_eq!(changed[0].1.lanes[0].note, PolyNote::Tie);
        assert_eq!(changed[1].1.lanes[0].velocity, PolyVelocity::Reset);
        assert_eq!(changed[2].0, 2);
        assert_eq!(changed[2].1.lanes[0].velocity, PolyVelocity::Rest);
    }

    #[test]
    fn velocity_extremes_are_clamped_to_recordable_values() {
        for (input, expected) in [(-1.0, 1), (0.0, 1), (1.0, 127), (2.0, 127)] {
            let mut recorder = StepRecorder::default();
            recorder.command(SequencerRecordCommand::Start);
            drain(&mut recorder);
            recorder.note_on(60, input);
            recorder.note_off(60);
            let events = drain(&mut recorder);
            let RecorderEvent::StepChanged { value, .. } = events[0] else {
                panic!()
            };
            assert_eq!(value.lanes[0].velocity, PolyVelocity::Velocity(expected));
        }
    }

    #[test]
    fn adversarial_event_ordering_stays_within_fixed_capacities() {
        let mut recorder = StepRecorder::default();
        recorder.command(SequencerRecordCommand::Start);
        let mut random = 0x91e1_0da5_u32;
        for _ in 0..16_384 {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let note = ((random >> 16) & 127) as u8;
            match random & 7 {
                0..=2 => recorder.note_on(note, (random & 255) as f32 / 255.0),
                3..=5 => recorder.note_off(note),
                6 => recorder.command(SequencerRecordCommand::MoveCursor((random as i8) | 1)),
                _ => recorder.command(if random & 0x100 != 0 {
                    SequencerRecordCommand::Stop
                } else {
                    SequencerRecordCommand::Start
                }),
            }
            assert!(recorder.chord.len() <= 6);
            assert!(recorder.cursor < 64);
            while recorder.pop_event().is_some() {}
        }
    }

    fn drain(recorder: &mut StepRecorder) -> heapless::Vec<RecorderEvent, 16> {
        let mut events = heapless::Vec::new();
        while let Some(event) = recorder.pop_event() {
            events.push(event).unwrap();
        }
        events
    }
}
