//! Four-track Prophet-style gated modulation sequencer runtime.

use crate::{
    midi::prophet::attack_decay_seconds,
    patch::ClockDivision,
    sequencer::{
        clock::{StepClock, StepClockEvent},
        model::{
            GATED_STEP_COUNT, GATED_TRACK_COUNT, GatedDestination, GatedSequence,
            GatedSequencerMode, LayerSequence, SequenceUpdate, SequencerType,
        },
    },
};

const GATED_RESET: u8 = 126;
const GATED_REST: u8 = 127;

/// Compact audio-thread form of the gated patch. Modulation destinations are
/// compiled by `PatchModulation`; playback only needs step bytes and whether
/// tracks 2/4 slew the preceding value track.
struct GatedPlaybackData {
    steps: [[u8; GATED_STEP_COUNT]; GATED_TRACK_COUNT],
    slew_mask: u8,
}

impl GatedPlaybackData {
    fn compile(sequence: &GatedSequence) -> Self {
        let mut data = Self {
            steps: [[GATED_RESET; GATED_STEP_COUNT]; GATED_TRACK_COUNT],
            slew_mask: 0,
        };
        for (track, source) in sequence.tracks.iter().enumerate() {
            for (step, value) in source.steps.iter().copied().enumerate() {
                data.steps[track][step] = value.rev2_raw() as u8;
            }
        }
        data.set_destination(1, sequence.tracks[1].destination);
        data.set_destination(3, sequence.tracks[3].destination);
        data
    }

    fn apply(&mut self, update: SequenceUpdate) -> bool {
        match update {
            SequenceUpdate::GatedDestination { track, destination } => {
                self.set_destination(usize::from(track), destination)
            }
            SequenceUpdate::GatedStep { track, step, value } => {
                let Some(target) = self
                    .steps
                    .get_mut(usize::from(track))
                    .and_then(|track| track.get_mut(usize::from(step)))
                else {
                    return false;
                };
                *target = value.rev2_raw() as u8;
                false
            }
            SequenceUpdate::Type(_)
            | SequenceUpdate::GatedMode(_)
            | SequenceUpdate::PolyNote { .. }
            | SequenceUpdate::PolyVelocity { .. }
            | SequenceUpdate::PolyLaneStep { .. } => false,
        }
    }

    fn set_destination(&mut self, track: usize, destination: GatedDestination) -> bool {
        let Some(pair) = (match track {
            1 => Some(0),
            3 => Some(1),
            _ => None,
        }) else {
            return false;
        };
        let old = self.slew_mask;
        if destination == GatedDestination::Slew {
            self.slew_mask |= 1 << pair;
        } else {
            self.slew_mask &= !(1 << pair);
        }
        old != self.slew_mask
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatedEvent {
    Boundary { gate: bool },
    GateOff,
}

pub(crate) struct GatedSequencer {
    playback: GatedPlaybackData,
    gated_mode: GatedSequencerMode,
    selected: bool,
    clock: StepClock,
    positions: [u8; 4],
    raw_values: [u8; 4],
    targets: [f32; 4],
    outputs: [f32; 4],
    slew_alpha: [f32; 2],
    gate_high: bool,
    triggered: bool,
    sample_rate: f32,
    last_step: Option<u8>,
}

impl GatedSequencer {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            playback: GatedPlaybackData::compile(&GatedSequence::default()),
            gated_mode: GatedSequencerMode::default(),
            selected: false,
            clock: StepClock::new(sample_rate),
            positions: [0; 4],
            raw_values: [0; 4],
            targets: [0.0; 4],
            outputs: [0.0; 4],
            slew_alpha: [1.0; 2],
            gate_high: false,
            triggered: false,
            sample_rate: sample_rate.max(1.0),
            last_step: None,
        }
    }

    pub(crate) fn apply_sequence(&mut self, sequence: &LayerSequence) {
        self.playback = GatedPlaybackData::compile(&sequence.gated);
        self.gated_mode = sequence.gated_mode;
        self.selected = sequence.sequencer_type == SequencerType::Gated;
        self.last_step = None;
        self.refresh_slew_coefficients();
        self.snap_unslewed_outputs();
        if !self.selected {
            self.deactivate();
        }
    }

    pub(crate) fn apply_update(&mut self, update: SequenceUpdate) {
        match update {
            SequenceUpdate::Type(value) => {
                self.selected = value == SequencerType::Gated;
            }
            SequenceUpdate::GatedMode(value) => {
                self.gated_mode = value;
            }
            SequenceUpdate::GatedDestination { .. } | SequenceUpdate::GatedStep { .. } => {
                let destination_changed = self.playback.apply(update);
                self.refresh_slew_coefficients();
                if destination_changed {
                    self.snap_unslewed_outputs();
                }
            }
            SequenceUpdate::PolyNote { .. }
            | SequenceUpdate::PolyVelocity { .. }
            | SequenceUpdate::PolyLaneStep { .. } => {}
        }
        if !self.selected {
            self.deactivate();
        }
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
        self.clock.midi_tick();
    }

    pub(crate) fn note_on(&mut self, first_held_note: bool) -> Option<GatedEvent> {
        if !self.selected {
            return None;
        }
        self.triggered = true;
        let mode = self.gated_mode;
        if first_held_note && mode.resets_on_key() {
            self.positions = [0; 4];
            self.clock.reset();
        }
        if mode == GatedSequencerMode::KeyStep {
            let gate = self.advance_tracks() && mode.gates_envelopes();
            self.gate_high = gate;
            Some(GatedEvent::Boundary { gate })
        } else {
            if first_held_note {
                self.clock.trigger_immediate();
            }
            None
        }
    }

    pub(crate) fn note_off_all(&mut self) {
        self.gate_high = false;
        self.triggered = false;
        self.last_step = None;
    }

    pub(crate) fn advance(&mut self, held_note: bool) -> Option<GatedEvent> {
        if !self.is_active(held_note) {
            return None;
        }
        self.advance_slew();
        if self.gated_mode == GatedSequencerMode::KeyStep {
            return None;
        }
        match self.clock.advance(1) {
            Some(StepClockEvent::Boundary) => {
                let gate = self.advance_tracks() && self.gated_mode.gates_envelopes();
                self.gate_high = gate;
                Some(GatedEvent::Boundary { gate })
            }
            Some(StepClockEvent::Half) if self.gate_high => {
                self.gate_high = false;
                Some(GatedEvent::GateOff)
            }
            _ => None,
        }
    }

    pub(crate) const fn outputs(&self) -> [f32; 4] {
        self.outputs
    }

    pub(crate) const fn last_step(&self) -> Option<u8> {
        self.last_step
    }

    pub(crate) const fn envelope_gating(&self) -> bool {
        self.gated_mode.gates_envelopes()
            && self.triggered
            && self.selected
            && self.playback.steps[0][0] != GATED_RESET
    }

    pub(crate) const fn is_active(&self, held_note: bool) -> bool {
        self.selected && self.triggered && held_note
    }

    fn advance_tracks(&mut self) -> bool {
        let mut track_one_gate = true;
        for track_index in 0..4 {
            let mut position = usize::from(self.positions[track_index]).min(15);
            let mut step = self.playback.steps[track_index][position];
            if step == GATED_RESET {
                position = 0;
                step = self.playback.steps[track_index][0];
                if step == GATED_RESET {
                    self.positions[track_index] = 0;
                    if track_index == 0 {
                        track_one_gate = false;
                    }
                    continue;
                }
            }
            match step {
                0..=125 => {
                    self.raw_values[track_index] = step;
                    self.targets[track_index] = f32::from(step) / 125.0;
                    if !self.is_slewed_track(track_index) {
                        self.outputs[track_index] = self.targets[track_index];
                    }
                }
                GATED_REST => {
                    if track_index == 0 {
                        track_one_gate = false;
                    }
                }
                GATED_RESET => unreachable!(),
                _ => unreachable!(),
            }
            if track_index == 0 {
                self.last_step = Some(position as u8);
            }
            self.positions[track_index] = ((position + 1) % 16) as u8;
        }
        self.refresh_slew_coefficients();
        track_one_gate
    }

    fn deactivate(&mut self) {
        self.positions = [0; 4];
        self.raw_values = [0; 4];
        self.targets = [0.0; 4];
        self.outputs = [0.0; 4];
        self.gate_high = false;
        self.triggered = false;
        self.last_step = None;
        self.clock.reset();
    }

    fn is_slewed_track(&self, track: usize) -> bool {
        match track {
            0 => self.playback.slew_mask & 1 != 0,
            2 => self.playback.slew_mask & 2 != 0,
            _ => false,
        }
    }

    fn advance_slew(&mut self) {
        if self.playback.slew_mask & 1 != 0 {
            self.advance_slew_pair(0, 0, 1);
        }
        if self.playback.slew_mask & 2 != 0 {
            self.advance_slew_pair(1, 2, 3);
        }
    }

    fn advance_slew_pair(&mut self, pair: usize, target_track: usize, slew_track: usize) {
        let raw = self.raw_values[slew_track];
        if raw == 0 {
            self.outputs[target_track] = self.targets[target_track];
            return;
        }
        let alpha = self.slew_alpha[pair];
        self.outputs[target_track] +=
            (self.targets[target_track] - self.outputs[target_track]) * alpha;
    }

    fn refresh_slew_coefficients(&mut self) {
        for (pair, slew_track) in [1, 3].into_iter().enumerate() {
            let raw = self.raw_values[slew_track].min(125);
            self.slew_alpha[pair] = if self.playback.slew_mask & (1 << pair) != 0 && raw > 0 {
                let seconds = attack_decay_seconds(u16::from(raw));
                (1.0 / (seconds * self.sample_rate)).clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
    }

    fn snap_unslewed_outputs(&mut self) {
        if self.playback.slew_mask & 1 == 0 {
            self.outputs[0] = self.targets[0];
        }
        if self.playback.slew_mask & 2 == 0 {
            self.outputs[2] = self.targets[2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{patch::ModDestination, sequencer::model::GatedStep};

    #[test]
    fn compiled_runtime_has_bounded_footprint() {
        assert!(core::mem::size_of::<GatedSequencer>() <= 160);
    }

    #[test]
    fn compiled_gated_data_keeps_wire_values_and_only_playback_slew_flags() {
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated.tracks[0].destination =
            GatedDestination::Modulation(ModDestination::FilterCutoff);
        sequence.gated.tracks[0].steps[0] = GatedStep::Value(125);
        sequence.gated.tracks[0].steps[1] = GatedStep::Reset;
        sequence.gated.tracks[0].steps[2] = GatedStep::Rest;
        sequence.gated.tracks[1].destination = GatedDestination::Slew;
        let runtime = GatedPlaybackData::compile(&sequence.gated);

        assert_eq!(&runtime.steps[0][..3], &[125, GATED_RESET, GATED_REST]);
        assert_eq!(runtime.slew_mask, 1);
    }

    #[test]
    fn independent_reset_repeats_track_prefix() {
        let mut engine = GatedSequencer::new(1_000.0);
        let mut sequence = sequence();
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        engine.apply_sequence(&sequence);
        for expected in [0.2, 0.4, 0.2, 0.4] {
            assert_eq!(
                engine.note_on(false),
                Some(GatedEvent::Boundary { gate: true })
            );
            assert!((engine.outputs()[0] - expected).abs() < 1.0e-6);
        }
    }

    #[test]
    fn track_one_rest_suppresses_shared_gate() {
        let mut engine = GatedSequencer::new(1_000.0);
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        sequence.gated.tracks[0].steps[0] = GatedStep::Rest;
        engine.apply_sequence(&sequence);
        assert_eq!(
            engine.note_on(true),
            Some(GatedEvent::Boundary { gate: false })
        );
    }

    #[test]
    fn no_gate_modes_still_advance_modulation() {
        let mut engine = GatedSequencer::new(1_000.0);
        let mut sequence = sequence();
        sequence.gated_mode = GatedSequencerMode::NoGate;
        engine.apply_sequence(&sequence);
        engine.note_on(true);
        assert_eq!(
            engine.advance(true),
            Some(GatedEvent::Boundary { gate: false })
        );
        assert!((engine.outputs()[0] - 0.2).abs() < 1.0e-6);
    }

    #[test]
    fn slew_moves_monotonically_and_stays_finite() {
        let mut engine = GatedSequencer::new(48_000.0);
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        sequence.gated.tracks[0].steps[0] = GatedStep::Value(125);
        sequence.gated.tracks[1].destination = GatedDestination::Slew;
        sequence.gated.tracks[1].steps[0] = GatedStep::Value(64);
        engine.apply_sequence(&sequence);
        engine.note_on(true);
        let mut previous = 0.0;
        for _ in 0..10_000 {
            let _ = engine.advance(true);
            let value = engine.outputs()[0];
            assert!(value.is_finite() && value >= previous && value <= 1.0);
            previous = value;
        }
        assert!(previous > 0.0 && previous < 1.0);
    }

    #[test]
    fn cached_slew_matches_the_reference_recurrence() {
        let sample_rate = 48_000.0;
        let slew_raw = 64;
        let mut engine = GatedSequencer::new(sample_rate);
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        sequence.gated.tracks[0].steps[0] = GatedStep::Value(125);
        sequence.gated.tracks[1].destination = GatedDestination::Slew;
        sequence.gated.tracks[1].steps[0] = GatedStep::Value(slew_raw);
        engine.apply_sequence(&sequence);
        engine.note_on(true);

        let alpha =
            (1.0 / (attack_decay_seconds(u16::from(slew_raw)) * sample_rate)).clamp(0.0, 1.0);
        let mut reference = 0.0;
        for _ in 0..256 {
            reference += (1.0 - reference) * alpha;
            assert_eq!(engine.advance(true), None);
            assert_eq!(engine.outputs()[0].to_bits(), reference.to_bits());
        }
    }

    #[test]
    fn inactive_sequencer_does_not_advance_cached_slew() {
        let mut engine = GatedSequencer::new(48_000.0);
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        sequence.gated.tracks[0].steps[0] = GatedStep::Value(125);
        sequence.gated.tracks[1].destination = GatedDestination::Slew;
        sequence.gated.tracks[1].steps[0] = GatedStep::Value(64);
        engine.apply_sequence(&sequence);

        for _ in 0..256 {
            assert_eq!(engine.advance(false), None);
        }
        assert_eq!(engine.outputs(), [0.0; 4]);
        assert_eq!(engine.last_step(), None);
    }

    #[test]
    fn pitch_values_use_half_semitones() {
        assert_eq!(gated_pitch_semitones(0), 0.0);
        assert_eq!(gated_pitch_semitones(14), 7.0);
        assert_eq!(gated_pitch_semitones(48), 24.0);
    }

    #[test]
    fn reset_and_no_reset_modes_diverge_after_retrigger() {
        let mut normal = GatedSequencer::new(1_000.0);
        let mut no_reset = GatedSequencer::new(1_000.0);
        let mut patch = sequence();
        patch.gated_mode = GatedSequencerMode::Normal;
        normal.apply_sequence(&patch);
        patch.gated_mode = GatedSequencerMode::NoReset;
        no_reset.apply_sequence(&patch);

        for engine in [&mut normal, &mut no_reset] {
            engine.note_on(true);
            assert_eq!(
                engine.advance(true),
                Some(GatedEvent::Boundary { gate: true })
            );
            engine.note_off_all();
            engine.note_on(true);
            assert_eq!(
                engine.advance(true),
                Some(GatedEvent::Boundary { gate: true })
            );
        }
        assert!((normal.outputs()[0] - 0.2).abs() < 1.0e-6);
        assert!((no_reset.outputs()[0] - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn gate_goes_low_at_exactly_half_a_step() {
        let mut engine = GatedSequencer::new(100.0);
        let mut patch = sequence();
        patch.gated_mode = GatedSequencerMode::Normal;
        engine.apply_sequence(&patch);
        engine.set_tempo_bpm(120.0);
        engine.set_clock_division(ClockDivision::Quarter);
        engine.note_on(true);
        assert_eq!(
            engine.advance(true),
            Some(GatedEvent::Boundary { gate: true })
        );
        for _ in 0..24 {
            assert_eq!(engine.advance(true), None);
        }
        assert_eq!(engine.advance(true), Some(GatedEvent::GateOff));
    }

    #[test]
    fn external_clock_steps_without_start_and_switch_has_no_phantom_step() {
        let mut engine = GatedSequencer::new(48_000.0);
        let mut patch = sequence();
        patch.gated_mode = GatedSequencerMode::NoGateNoReset;
        engine.apply_sequence(&patch);
        engine.set_clock_division(ClockDivision::Sixteenth);
        engine.set_external_clock(true);
        engine.note_on(true);
        assert_eq!(
            engine.advance(true),
            Some(GatedEvent::Boundary { gate: false })
        );
        for _ in 0..5 {
            engine.midi_clock_tick();
            assert_eq!(engine.advance(true), None);
        }
        engine.midi_clock_tick();
        assert_eq!(
            engine.advance(true),
            Some(GatedEvent::Boundary { gate: false })
        );
        engine.set_external_clock(false);
        assert_eq!(engine.advance(true), None);
    }

    #[test]
    fn empty_track_one_never_enables_envelope_gating() {
        let mut engine = GatedSequencer::new(48_000.0);
        engine.apply_sequence(&LayerSequence::default());
        assert!(!engine.envelope_gating());
    }

    #[test]
    fn gated_selection_waits_for_a_note_trigger() {
        let mut engine = GatedSequencer::new(100.0);
        let sequence = sequence();
        engine.apply_sequence(&sequence);
        for _ in 0..100 {
            assert_eq!(engine.advance(true), None);
        }
        assert_eq!(engine.last_step(), None);
    }

    #[test]
    fn switching_to_polyphonic_clears_gated_runtime_state() {
        let mut engine = GatedSequencer::new(100.0);
        let mut sequence = sequence();
        sequence.gated_mode = GatedSequencerMode::KeyStep;
        engine.apply_sequence(&sequence);
        assert!(matches!(
            engine.note_on(true),
            Some(GatedEvent::Boundary { .. })
        ));
        assert!(engine.outputs()[0] > 0.0);

        engine.apply_update(SequenceUpdate::Type(SequencerType::Polyphonic));
        assert_eq!(engine.outputs(), [0.0; 4]);
        assert_eq!(engine.last_step(), None);
        assert!(!engine.envelope_gating());
    }

    fn sequence() -> LayerSequence {
        let mut sequence = LayerSequence::default();
        sequence.sequencer_type = SequencerType::Gated;
        sequence.gated.tracks[0].steps[0] = GatedStep::Value(25);
        sequence.gated.tracks[0].steps[1] = GatedStep::Value(50);
        sequence.gated.tracks[0].steps[2] = GatedStep::Reset;
        sequence
    }

    const fn gated_pitch_semitones(raw: u8) -> f32 {
        let value = if raw > 125 { 125 } else { raw };
        value as f32 * 0.5
    }
}
