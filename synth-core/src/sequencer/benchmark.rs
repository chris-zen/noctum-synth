//! Host benchmark hooks for the private sequencer playback runtimes.

use crate::{
    patch::{ClockDivision, ModDestination},
    sequencer::{
        clock::StepClock,
        gated::{GatedEvent, GatedSequencer},
        model::{
            GatedDestination, GatedSequencerMode, GatedStep, LayerSequence, PolyLaneStep, PolyNote,
            PolyVelocity, SequencerType,
        },
        poly::{PolyEvent, PolySequencer},
    },
};

pub fn sequencer_runtime_sizes() -> [usize; 3] {
    [
        core::mem::size_of::<StepClock>(),
        core::mem::size_of::<GatedSequencer>(),
        core::mem::size_of::<PolySequencer>(),
    ]
}

pub fn run_clock_samples(samples: usize) -> u64 {
    let mut clock = StepClock::new(48_000.0);
    clock.set_tempo_bpm(120.0);
    clock.set_division(ClockDivision::Sixteenth);
    let mut checksum = 0_u64;
    for sample in 0..samples {
        if let Some(event) = clock.advance(1) {
            checksum = checksum.wrapping_add(sample as u64 + event as u64 + 1);
        }
    }
    checksum
}

pub fn run_gated_samples(samples: usize, slew: bool) -> u64 {
    let mut sequence = LayerSequence::default();
    sequence.sequencer_type = SequencerType::Gated;
    sequence.gated_mode = GatedSequencerMode::NoGate;
    sequence.gated.tracks[0].destination =
        GatedDestination::Modulation(ModDestination::Osc1Frequency);
    sequence.gated.tracks[1].destination = if slew {
        GatedDestination::Slew
    } else {
        GatedDestination::Off
    };
    for step in 0..16 {
        sequence.gated.tracks[0].steps[step] = GatedStep::Value(if step % 2 == 0 { 0 } else { 24 });
        sequence.gated.tracks[1].steps[step] = GatedStep::Value(96);
    }
    let mut runtime = GatedSequencer::new(48_000.0);
    runtime.apply_sequence(&sequence);
    runtime.set_tempo_bpm(120.0);
    runtime.set_clock_division(ClockDivision::Sixteenth);
    runtime.note_on(true);

    let mut checksum = 0_u64;
    for sample in 0..samples {
        match runtime.advance(true) {
            Some(GatedEvent::Boundary { gate }) => {
                checksum = checksum.wrapping_add(sample as u64 + u64::from(gate));
            }
            Some(GatedEvent::GateOff) => checksum = checksum.wrapping_add(sample as u64),
            None => {}
        }
        checksum = checksum.wrapping_add(u64::from(runtime.outputs()[0].to_bits()));
    }
    checksum
}

pub fn run_poly_samples(samples: usize) -> u64 {
    let mut sequence = LayerSequence::default();
    sequence.sequencer_type = SequencerType::Polyphonic;
    for step in 0..64 {
        for lane in 0..6 {
            sequence.poly.steps[step].lanes[lane] = PolyLaneStep {
                note: if step % 4 == 1 {
                    PolyNote::Tie
                } else {
                    PolyNote::Note(60 + lane as u8)
                },
                velocity: if step % 4 == 3 {
                    PolyVelocity::Rest
                } else {
                    PolyVelocity::Velocity(100)
                },
            };
        }
    }
    let mut runtime = PolySequencer::new(48_000.0);
    runtime.apply_sequence(&sequence);
    runtime.set_tempo_bpm(120.0);
    runtime.set_clock_division(ClockDivision::Sixteenth);
    runtime.start();

    let mut checksum = 0_u64;
    for sample in 0..samples {
        match runtime.advance() {
            Some(PolyEvent::Step(event)) => {
                for note in event.note_on.into_iter().flatten() {
                    checksum = checksum.wrapping_add(
                        sample as u64 + u64::from(note.note) + u64::from(note.velocity.to_bits()),
                    );
                }
            }
            Some(PolyEvent::GateOff(release)) => {
                checksum = checksum.wrapping_add(
                    sample as u64 + release.into_iter().filter(|value| *value).count() as u64,
                );
            }
            None => {}
        }
    }
    checksum
}
