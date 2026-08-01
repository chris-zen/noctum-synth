//! Shared sample/MIDI step clock for the arpeggiator and sequencers.

use crate::{DEFAULT_TEMPO_BPM, patch::ClockDivision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepClockEvent {
    Boundary,
    Half,
}

pub(crate) struct StepClock {
    sample_rate: f32,
    tempo_bpm: f32,
    division: ClockDivision,
    external: bool,
    phase: f32,
    duration: f32,
    half_sent: bool,
    long_step: bool,
    midi_pulses: f32,
    pending_boundaries: u8,
}

impl StepClock {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let division = ClockDivision::default();
        let mut clock = Self {
            sample_rate: sample_rate.max(1.0),
            tempo_bpm: DEFAULT_TEMPO_BPM,
            division,
            external: false,
            phase: 0.0,
            duration: 1.0,
            half_sent: false,
            long_step: true,
            midi_pulses: 0.0,
            pending_boundaries: 0,
        };
        clock.duration = clock.step_duration();
        clock
    }

    pub(crate) fn set_tempo_bpm(&mut self, bpm: f32) {
        let progress = (self.phase / self.duration.max(1.0)).clamp(0.0, 1.0);
        self.tempo_bpm = bpm.clamp(30.0, 250.0);
        self.duration = self.step_duration();
        self.phase = progress * self.duration;
    }

    pub(crate) fn set_division(&mut self, division: ClockDivision) {
        let progress = (self.phase / self.duration.max(1.0)).clamp(0.0, 1.0);
        self.division = division;
        self.duration = self.step_duration();
        self.phase = progress * self.duration;
        self.midi_pulses = 0.0;
    }

    pub(crate) fn set_external(&mut self, external: bool) {
        if self.external != external {
            self.external = external;
            self.phase = 0.0;
            self.half_sent = false;
            self.midi_pulses = 0.0;
            self.pending_boundaries = 0;
        }
    }

    pub(crate) fn reset(&mut self) {
        self.phase = 0.0;
        self.half_sent = false;
        self.long_step = true;
        self.midi_pulses = 0.0;
        self.pending_boundaries = 0;
        self.duration = self.step_duration();
    }

    pub(crate) fn trigger_immediate(&mut self) {
        self.pending_boundaries = self.pending_boundaries.saturating_add(1);
    }

    /// Add one MIDI timing-clock pulse (24 pulses per quarter note).
    pub(crate) fn midi_tick(&mut self) {
        if !self.external {
            return;
        }
        self.midi_pulses += 1.0;
        let threshold = self.pulses_per_step();
        if self.midi_pulses + f32::EPSILON >= threshold {
            self.midi_pulses -= threshold;
            self.pending_boundaries = self.pending_boundaries.saturating_add(1);
        }
    }

    pub(crate) fn advance(&mut self, samples: usize) -> Option<StepClockEvent> {
        if self.pending_boundaries > 0 {
            self.pending_boundaries -= 1;
            self.begin_next_step();
            return Some(StepClockEvent::Boundary);
        }

        self.phase += samples as f32;
        if !self.half_sent && self.phase >= self.duration * 0.5 {
            self.half_sent = true;
            return Some(StepClockEvent::Half);
        }
        if !self.external && self.phase >= self.duration {
            self.phase -= self.duration;
            self.begin_next_step();
            return Some(StepClockEvent::Boundary);
        }
        None
    }

    fn begin_next_step(&mut self) {
        self.phase = 0.0;
        self.half_sent = false;
        self.long_step = !self.long_step;
        self.duration = self.step_duration();
    }

    fn step_duration(&self) -> f32 {
        self.base_samples_per_step() * self.swing_factor()
    }

    #[cfg(test)]
    pub(crate) fn nominal_samples_per_step(&self) -> f32 {
        self.base_samples_per_step()
    }

    fn pulses_per_step(&self) -> f32 {
        24.0 / self.division.steps_per_quarter().max(0.001) * self.swing_factor()
    }

    fn base_samples_per_step(&self) -> f32 {
        self.sample_rate * 60.0
            / (self.tempo_bpm.max(1.0) * self.division.steps_per_quarter().max(0.001))
    }

    fn swing_factor(&self) -> f32 {
        let (long, short) = match self.division {
            ClockDivision::EighthHalfSwing | ClockDivision::SixteenthHalfSwing => {
                (7.0 / 6.0, 5.0 / 6.0)
            }
            ClockDivision::EighthSwing | ClockDivision::SixteenthSwing => (4.0 / 3.0, 2.0 / 3.0),
            _ => (1.0, 1.0),
        };
        if self.long_step { long } else { short }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_clock_has_no_long_run_drift() {
        let mut clock = StepClock::new(48_000.0);
        clock.set_tempo_bpm(120.0);
        clock.set_division(ClockDivision::Sixteenth);
        assert_eq!(boundaries(&mut clock, 48_000 * 60), 480);
    }

    #[test]
    fn swing_pairs_keep_the_same_average_period() {
        for division in [ClockDivision::EighthHalfSwing, ClockDivision::EighthSwing] {
            let mut clock = StepClock::new(48_000.0);
            clock.set_tempo_bpm(120.0);
            clock.set_division(division);
            let mut positions = [0usize; 3];
            let mut found = 0;
            for sample in 0..100_000 {
                if clock.advance(1) == Some(StepClockEvent::Boundary) {
                    positions[found] = sample;
                    found += 1;
                    if found == positions.len() {
                        break;
                    }
                }
            }
            assert_eq!(positions[2] - positions[0], 24_000);
        }
    }

    #[test]
    fn external_clock_steps_without_transport_messages() {
        let mut clock = StepClock::new(48_000.0);
        clock.set_division(ClockDivision::Sixteenth);
        clock.set_external(true);
        for _ in 0..5 {
            clock.midi_tick();
            assert_ne!(clock.advance(1), Some(StepClockEvent::Boundary));
        }
        clock.midi_tick();
        assert_eq!(clock.advance(1), Some(StepClockEvent::Boundary));
    }

    #[test]
    fn tempo_change_preserves_fractional_phase() {
        let mut clock = StepClock::new(1_000.0);
        clock.set_tempo_bpm(60.0);
        clock.set_division(ClockDivision::Quarter);
        for _ in 0..250 {
            let _ = clock.advance(1);
        }
        clock.set_tempo_bpm(120.0);
        assert_eq!(boundaries(&mut clock, 374), 0);
        assert_eq!(boundaries(&mut clock, 1), 1);
    }

    #[test]
    fn every_division_has_finite_pair_timing_at_tempo_limits() {
        for tempo in [0.0, 250.0, f32::MAX] {
            for division in ClockDivision::ALL {
                let mut clock = StepClock::new(100.0);
                clock.set_tempo_bpm(tempo);
                clock.set_division(division);
                let base = clock.nominal_samples_per_step();
                assert!(base.is_finite() && base >= 1.0);
                let mut positions = [0usize; 3];
                let mut found = 0;
                for sample in 0..10_000 {
                    if clock.advance(1) == Some(StepClockEvent::Boundary) {
                        positions[found] = sample;
                        found += 1;
                        if found == 3 {
                            break;
                        }
                    }
                }
                assert_eq!(found, 3, "{tempo}/{division:?}");
                let pair = (positions[2] - positions[0]) as f32;
                assert!(
                    (pair - base * 2.0).abs() <= 2.0,
                    "{tempo}/{division:?}: {pair} vs {base}"
                );
            }
        }
    }

    fn boundaries(clock: &mut StepClock, samples: usize) -> usize {
        (0..samples)
            .filter(|_| clock.advance(1) == Some(StepClockEvent::Boundary))
            .count()
    }
}
