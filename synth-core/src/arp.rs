use rand_core::Rng;
use rand_pcg::Pcg32;
use rand_core::SeedableRng;

use crate::pressed_keys::PressedKeys;
use crate::patch::{ArpMode, ArpParams, ArpSustainMode, ClockDivision};

pub const MAX_ARP_NOTES: usize = 16;
pub const MAX_ARP_STEPS: usize = MAX_ARP_NOTES * 3 * 3;

#[derive(Debug, Clone, Copy)]
struct ArpStep {
    note: u8,
    velocity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArpEvent {
    Step(u8),
    Release(u8),
}

pub(crate) struct ArpEngine {
    params: ArpParams,
    held_notes: PressedKeys,
    pending_notes: PressedKeys,
    sequence: heapless::Vec<ArpStep, MAX_ARP_STEPS>,
    step: usize,
    current_note: Option<u8>,
    current_velocity: f32,
    phase: f32,
    samples_per_step: f32,
    tempo_bpm: f32,
    clock_division: ClockDivision,
    sample_rate: f32,
    sustain_pedal: bool,
    cycle_count: u32,
    needs_rebuild: bool,
}

impl ArpEngine {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            params: ArpParams::default(),
            held_notes: PressedKeys::default(),
            pending_notes: PressedKeys::default(),
            sequence: heapless::Vec::new(),
            step: 0,
            current_note: None,
            current_velocity: 0.0,
            phase: 0.0,
            samples_per_step: Self::calc_samples_per_step(sample_rate, 120.0, ClockDivision::default()),
            tempo_bpm: 120.0,
            clock_division: ClockDivision::default(),
            sample_rate,
            sustain_pedal: false,
            cycle_count: 0,
            needs_rebuild: false,
        }
    }

    pub(crate) fn params(&self) -> &ArpParams {
        &self.params
    }

    pub(crate) fn set_params(&mut self, params: &ArpParams) {
        self.params = params.clone();
        self.rebuild_sequence();
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if self.params.enabled == enabled {
            return;
        }
        self.params.enabled = enabled;
        if !enabled {
            self.clear();
        } else if !self.held_notes.is_empty() {
            self.rebuild_sequence();
        }
    }

    pub(crate) fn set_tempo_bpm(&mut self, tempo_bpm: f32) {
        self.tempo_bpm = tempo_bpm;
        self.samples_per_step = Self::calc_samples_per_step(
            self.sample_rate,
            tempo_bpm,
            self.clock_division,
        );
    }

    pub(crate) fn set_clock_division(&mut self, division: ClockDivision) {
        self.clock_division = division;
        self.samples_per_step = Self::calc_samples_per_step(
            self.sample_rate,
            self.tempo_bpm,
            division,
        );
    }

    pub(crate) fn note_on(&mut self, note: u8, velocity: f32) {
        if self.params.relatch && !self.params.hold {
            self.held_notes.clear();
        }
        if self.params.beat_sync && self.current_note.is_some() {
            self.pending_notes.press(note, velocity);
            return;
        }
        self.held_notes.press(note, velocity);
        self.rebuild_sequence();
    }

    pub(crate) fn note_off(&mut self, note: u8) {
        self.pending_notes.release(note);
        if !self.params.hold {
            let arp_hold_mom_engaged = self.sustain_pedal
                && matches!(self.params.sustain_mode, ArpSustainMode::ArpHoldMom);
            if !arp_hold_mom_engaged {
                self.held_notes.release(note);
                self.rebuild_sequence();
            }
        }
    }

    pub(crate) fn all_notes_off(&mut self) {
        self.held_notes.clear();
        self.pending_notes.clear();
        self.sequence.clear();
        self.step = 0;
        self.current_note = None;
        self.phase = 0.0;
        self.cycle_count = 0;
        self.needs_rebuild = false;
    }

    pub(crate) fn set_sustain_pedal(&mut self, pressed: bool) {
        let was_pressed = self.sustain_pedal;
        self.sustain_pedal = pressed;

        match self.params.sustain_mode {
            ArpSustainMode::ArpHold => {
                if !was_pressed && pressed {
                    self.params.hold = !self.params.hold;
                    if !self.params.hold {
                        self.rebuild_sequence();
                    }
                }
            }
            ArpSustainMode::ArpHoldMom => {
                if !pressed {
                    if !self.params.hold {
                        self.held_notes.clear();
                        self.rebuild_sequence();
                    }
                }
            }
            ArpSustainMode::Sustain => {
                if !was_pressed && pressed {
                    self.params.hold = true;
                } else if was_pressed && !pressed {
                    self.params.hold = false;
                    self.rebuild_sequence();
                }
            }
        }
    }

    pub(crate) fn sustain_forward(&self) -> bool {
        match self.params.sustain_mode {
            ArpSustainMode::ArpHold | ArpSustainMode::ArpHoldMom => false,
            ArpSustainMode::Sustain => true,
        }
    }

    pub(crate) fn advance(&mut self, samples: usize) -> Option<ArpEvent> {
        if self.needs_rebuild {
            self.needs_rebuild = false;
            self.rebuild_sequence();
        }

        if self.sequence.is_empty() {
            return self.current_note.take().map(ArpEvent::Release);
        }

        self.phase += samples as f32;
        if self.phase < self.samples_per_step {
            return None;
        }

        self.phase -= self.samples_per_step;

        if self.step >= self.sequence.len() {
            if self.params.mode == ArpMode::Random {
                self.cycle_count = self.cycle_count.wrapping_add(1);
                self.rebuild_sequence();
            }
            self.step = 0;

            // At a cycle boundary (step just reset to 0), transfer any
            // beat-sync queued notes into held_notes by merging them in
            // (not replacing — notes held throughout the cycle must be
            // preserved).
            if self.params.beat_sync && !self.pending_notes.is_empty() {
                let pending = core::mem::take(&mut self.pending_notes);
                for (note, velocity) in pending.iter() {
                    self.held_notes.press(note, velocity);
                }
                self.rebuild_sequence();
                if self.sequence.is_empty() {
                    return self.current_note.take().map(ArpEvent::Release);
                }
            }
        }

        let step_note = self.sequence[self.step];
        self.step += 1;
        self.current_note = Some(step_note.note);
        self.current_velocity = step_note.velocity;
        Some(ArpEvent::Step(step_note.note))
    }

    pub(crate) fn current_note(&self) -> Option<u8> {
        self.current_note
    }

    pub(crate) fn current_velocity(&self) -> f32 {
        self.current_velocity
    }

    fn rebuild_sequence(&mut self) {
        let was_empty = self.sequence.is_empty();
        self.sequence.clear();

        if self.held_notes.is_empty() {
            self.step = 0;
            return;
        }

        let held: heapless::Vec<(u8, f32), MAX_ARP_NOTES> =
            self.held_notes.iter().collect();

        let mut notes: heapless::Vec<(u8, f32), MAX_ARP_NOTES> = heapless::Vec::new();
        match self.params.mode {
            ArpMode::Up => {
                let mut sorted = held.clone();
                sorted.sort_unstable_by_key(|(n, _)| *n);
                notes = sorted;
            }
            ArpMode::Down => {
                let mut sorted = held.clone();
                sorted.sort_unstable_by_key(|(n, _)| core::cmp::Reverse(*n));
                notes = sorted;
            }
            ArpMode::UpDown => {
                let mut sorted = held.clone();
                sorted.sort_unstable_by_key(|(n, _)| *n);
                for item in &sorted {
                    let _ = notes.push(*item);
                }
                if sorted.len() > 2 {
                    let mut rev = sorted.clone();
                    rev.sort_unstable_by_key(|(n, _)| core::cmp::Reverse(*n));
                    for item in rev.iter().skip(1).take(rev.len() - 2) {
                        let _ = notes.push(*item);
                    }
                }
            }
            ArpMode::Random => {
                let mut randomized = held.clone();
                let seed = (self.cycle_count as u64)
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let mut rng = Pcg32::seed_from_u64(seed);
                for i in (1..randomized.len()).rev() {
                    let j = (rng.next_u32() as usize) % (i + 1);
                    randomized.swap(i, j);
                }
                notes = randomized;
            }
            ArpMode::Assign => {
                notes = held;
            }
        }

        let range = self.params.range.clamp(1, 3) as usize;
        let repeats = self.params.repeats.clamp(1, 3) as usize;

        for octave in 0..range {
            let offset = (octave * 12) as u8;
            for (note, velocity) in &notes {
                let transposed = note.saturating_add(offset);
                if transposed > 127 {
                    continue;
                }
                for _ in 0..repeats {
                    let _ = self.sequence.push(ArpStep {
                        note: transposed,
                        velocity: *velocity,
                    });
                }
            }
        }

        if self.step >= self.sequence.len() {
            self.step = 0;
        }
        if was_empty {
            self.phase = self.samples_per_step;
        }
    }

    fn clear(&mut self) {
        self.held_notes.clear();
        self.pending_notes.clear();
        self.sequence.clear();
        self.step = 0;
        self.current_note = None;
        self.current_velocity = 0.0;
        self.phase = 0.0;
        self.cycle_count = 0;
        self.needs_rebuild = false;
    }

    fn calc_samples_per_step(
        sample_rate: f32,
        tempo_bpm: f32,
        division: ClockDivision,
    ) -> f32 {
        let bps = tempo_bpm / 60.0;
        let steps_per_beat = division.steps_per_quarter();
        if bps <= 0.0 || steps_per_beat <= 0.0 {
            return sample_rate;
        }
        sample_rate / (bps * steps_per_beat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::{ArpMode, ArpSustainMode};
    extern crate std;
    use std::vec::Vec;

    fn next_note(arp: &mut ArpEngine) -> u8 {
        loop {
            match arp.advance(1) {
                Some(ArpEvent::Step(note)) => return note,
                Some(ArpEvent::Release(_)) => continue,
                None => {}
            }
        }
    }

    #[test]
    fn arp_disabled_passes_no_notes() {
        let mut arp = ArpEngine::new(44100.0);
        assert!(arp.advance(1).is_none());
    }

    #[test]
    fn arp_up_mode_produces_ascending_sequence() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.note_on(67, 1.0);
        arp.note_on(60, 1.0);
        arp.note_on(72, 1.0);

        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 67);
        assert_eq!(next_note(&mut arp), 72);
    }

    #[test]
    fn arp_down_mode_produces_descending_sequence() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Down;
        arp.note_on(60, 1.0);
        arp.note_on(72, 1.0);
        arp.note_on(67, 1.0);

        assert_eq!(next_note(&mut arp), 72);
        assert_eq!(next_note(&mut arp), 67);
        assert_eq!(next_note(&mut arp), 60);
    }

    #[test]
    fn arp_assign_mode_preserves_press_order() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Assign;
        arp.note_on(72, 1.0);
        arp.note_on(60, 1.0);
        arp.note_on(67, 1.0);

        assert_eq!(next_note(&mut arp), 72);
        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 67);
    }

    #[test]
    fn arp_range_2_octaves_duplicates_notes() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.range = 2;
        arp.note_on(60, 1.0);

        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 72);
    }

    #[test]
    fn arp_range_3_octaves() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.range = 3;
        arp.note_on(60, 1.0);

        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 72);
        assert_eq!(next_note(&mut arp), 84);
    }

    #[test]
    fn arp_repeats_2() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.repeats = 2;
        arp.note_on(60, 1.0);

        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 60);
    }

    #[test]
    fn arp_updown_mode() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::UpDown;
        arp.note_on(60, 1.0);
        arp.note_on(64, 1.0);
        arp.note_on(67, 1.0);

        let notes: Vec<u8> = (0..5).map(|_| next_note(&mut arp)).collect();
        assert_eq!(notes, &[60, 64, 67, 64, 60]);
    }

    #[test]
    fn arp_hold_latches_after_release() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.hold = true;
        arp.note_on(60, 1.0);
        arp.note_off(60);

        assert_eq!(next_note(&mut arp), 60);
    }

    #[test]
    fn arp_relatch_clears_on_new_press() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.relatch = true;
        arp.note_on(60, 1.0);
        arp.note_on(64, 1.0);

        assert_eq!(next_note(&mut arp), 64);
    }

    #[test]
    fn arp_beat_sync_queues_pending_keys() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.beat_sync = true;
        arp.note_on(72, 1.0);

        // First note plays immediately
        assert_eq!(next_note(&mut arp), 72);

        // Add new note while arp is running — queued in pending, NOT immediate
        arp.note_on(60, 1.0);

        // The single-note [72] cycle wraps immediately on the next advance,
        // triggering beat_sync merge: held_notes → [72, 60], rebuilds as
        // [60, 72] (ascending).  Next plays the first note of the new sequence.
        assert_eq!(next_note(&mut arp), 60);

        // Then the higher note
        assert_eq!(next_note(&mut arp), 72);

        // Wraps back to 60
        assert_eq!(next_note(&mut arp), 60);
    }

    #[test]
    fn arp_beat_sync_preserves_notes_held_throughout_cycle() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.beat_sync = true;

        // Press two notes before any step fires.  Both go to held_notes because
        // beat_sync only queues notes AFTER the arp has started playing.
        arp.note_on(60, 1.0);
        arp.note_on(72, 1.0);

        // Sequence: [60, 72]
        assert_eq!(next_note(&mut arp), 60);

        // Press a third note — now queued in pending because current_note is set.
        arp.note_on(67, 1.0);

        // Plays 72 from the current cycle.
        assert_eq!(next_note(&mut arp), 72);

        // Wraps: step 0 → beat_sync merges pending 67 → sequence [60, 67, 72].
        assert_eq!(next_note(&mut arp), 60);
        assert_eq!(next_note(&mut arp), 67);
        assert_eq!(next_note(&mut arp), 72);

        // Full cycle wraps cleanly.
        assert_eq!(next_note(&mut arp), 60);
    }

    #[test]
    fn arp_disable_clears_all() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.note_on(60, 1.0);
        arp.set_enabled(false);
        assert!(arp.advance(1).is_none());
    }

    #[test]
    fn arp_tempo_changes_step_rate() {
        let mut arp = ArpEngine::new(48000.0);
        arp.set_tempo_bpm(120.0);
        let s1 = arp.samples_per_step;
        arp.set_tempo_bpm(60.0);
        assert!((arp.samples_per_step - s1 * 2.0).abs() < 0.01);
    }

    #[test]
    fn arp_clock_division_changes_step_rate() {
        let mut arp = ArpEngine::new(48000.0);
        arp.set_tempo_bpm(120.0);
        arp.set_clock_division(ClockDivision::Quarter);
        let s1 = arp.samples_per_step;
        arp.set_clock_division(ClockDivision::Sixteenth);
        assert!((arp.samples_per_step - s1 / 4.0).abs() < 0.01);
    }

    #[test]
    fn arp_sustain_arp_hold_toggles_hold() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.sustain_mode = ArpSustainMode::ArpHold;
        arp.note_on(60, 1.0);
        arp.set_sustain_pedal(true);
        assert!(arp.params.hold);
        arp.set_sustain_pedal(false);
        arp.set_sustain_pedal(true);
        assert!(!arp.params.hold);
    }

    #[test]
    fn arp_sustain_mom_stops_on_release() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.params.sustain_mode = ArpSustainMode::ArpHoldMom;
        arp.note_on(60, 1.0);
        arp.set_sustain_pedal(true);
        assert!(!arp.sequence.is_empty());
        arp.set_sustain_pedal(false);
        assert!(arp.sequence.is_empty());
    }

    #[test]
    fn arp_sustain_forward_returns_true_for_sustain_mode() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.sustain_mode = ArpSustainMode::Sustain;
        assert!(arp.sustain_forward());
        arp.params.sustain_mode = ArpSustainMode::ArpHold;
        assert!(!arp.sustain_forward());
        arp.params.sustain_mode = ArpSustainMode::ArpHoldMom;
        assert!(!arp.sustain_forward());
    }

    #[test]
    fn arp_releasing_last_key_returns_current_note_as_stuck_release() {
        let mut arp = ArpEngine::new(44100.0);
        arp.params.enabled = true;
        arp.params.mode = ArpMode::Up;
        arp.note_on(60, 1.0);

        // Start playing
        let n = next_note(&mut arp);
        assert_eq!(n, 60);
        assert!(arp.current_note().is_some());

        // Release the key (no hold)
        arp.note_off(60);

        // The current note should be returned as a release event
        let released = arp.advance(1);
        assert_eq!(released, Some(ArpEvent::Release(60)));
        assert!(arp.current_note().is_none());

        // Subsequent calls return None
        assert!(arp.advance(1).is_none());
    }
}
