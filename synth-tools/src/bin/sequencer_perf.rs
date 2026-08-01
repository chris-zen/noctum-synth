//! Host-side callback benchmark for sequencer runtime overhead.

use std::hint::black_box;
use std::time::Instant;

use synth_core::dsp::FilterOversampling;
use synth_core::{
    ClockDivision, ControlMessage, GatedDestination, GatedSequencerMode, GatedStep, LayerId,
    LayerMode, LayerTarget, ModDestination, Patch, PolyLaneStep, PolyNote, PolyVelocity,
    SequencerTransportCommand, SequencerType, SynthEngineWithMemory, VOICE_PACKS,
};

const SAMPLE_RATE: f32 = 48_000.0;
const CALLBACK_FRAMES: usize = 48;
const WARMUP_BLOCKS: usize = 2_000;
const MEASURE_BLOCKS: usize = 20_000;
const EFFECT_SAMPLES_PER_LAYER: usize = 48_000 * 2;

type Engine = SynthEngineWithMemory<Vec<f32>, { VOICE_PACKS }, 2>;

#[derive(Clone, Copy)]
enum Case {
    Baseline,
    PolyWithStoredGatedSlew,
    GatedNoSlew,
    GatedSlew,
    PolyOneLane,
    PolyOneLaneShortRelease,
    PolySixLanes,
    StackInactivePayload,
}

fn main() {
    println!(
        "sequencer perf: sample_rate={} callback_frames={} warmup={} measured={}",
        SAMPLE_RATE, CALLBACK_FRAMES, WARMUP_BLOCKS, MEASURE_BLOCKS
    );
    for (name, case) in [
        ("baseline_poly_stopped", Case::Baseline),
        ("poly_stored_gated_slew", Case::PolyWithStoredGatedSlew),
        ("gated_active_no_slew", Case::GatedNoSlew),
        ("gated_active_slew", Case::GatedSlew),
        ("poly_active_1_lane", Case::PolyOneLane),
        ("poly_1_lane_short_release", Case::PolyOneLaneShortRelease),
        ("poly_active_6_lanes", Case::PolySixLanes),
        ("stack_inactive_payload", Case::StackInactivePayload),
    ] {
        run_case(name, case);
    }
}

fn run_case(name: &str, case: Case) {
    let (mut engine, start_poly) = configured_engine(case);
    if start_poly {
        for layer in active_layers(case).iter().copied() {
            engine.handle_control(ControlMessage::SetSequencerTransport {
                target: LayerTarget::Explicit(layer),
                command: SequencerTransportCommand::Start,
            });
        }
    } else {
        engine.note_on(60, 1.0);
    }

    let mut output = [0.0_f32; CALLBACK_FRAMES * 2];
    for _ in 0..WARMUP_BLOCKS {
        engine.process_interleaved(&mut output, 2);
    }
    let active_voices = engine.active_voice_count();

    let mut timings = Vec::with_capacity(MEASURE_BLOCKS);
    let mut checksum = 0.0_f32;
    for _ in 0..MEASURE_BLOCKS {
        let started = Instant::now();
        engine.process_interleaved(&mut output, 2);
        timings.push(started.elapsed().as_nanos() as u64);
        checksum += output[0];
    }
    black_box(checksum);
    timings.sort_unstable();
    let mean = timings.iter().map(|value| *value as u128).sum::<u128>() / timings.len() as u128;
    println!(
        "{name:<28} voices={active_voices:>2} mean={mean:>8}ns p50={:>8}ns p95={:>8}ns p99={:>8}ns max={:>8}ns",
        percentile(&timings, 50),
        percentile(&timings, 95),
        percentile(&timings, 99),
        timings[timings.len() - 1],
    );
}

fn configured_engine(case: Case) -> (Engine, bool) {
    let mut patch = Patch::default();
    patch.layer_a.bpm = 120.0;
    patch.layer_a.clock_divide = ClockDivision::Sixteenth;
    match case {
        Case::Baseline => {}
        Case::PolyWithStoredGatedSlew => configure_stored_gated_slew(&mut patch.layer_a),
        Case::GatedNoSlew => configure_gated(&mut patch.layer_a, false),
        Case::GatedSlew => configure_gated(&mut patch.layer_a, true),
        Case::PolyOneLane => configure_poly(&mut patch.layer_a, 1),
        Case::PolyOneLaneShortRelease => {
            configure_poly(&mut patch.layer_a, 1);
            patch.layer_a.amplifier.eg_release = 0.01;
        }
        Case::PolySixLanes => configure_poly(&mut patch.layer_a, 6),
        Case::StackInactivePayload => {
            patch.mode = LayerMode::Stack;
            configure_stored_gated_slew(&mut patch.layer_a);
            configure_stored_gated_slew(&mut patch.layer_b);
            patch.layer_b.bpm = 120.0;
            patch.layer_b.clock_divide = ClockDivision::Sixteenth;
        }
    }
    let start_poly = matches!(
        case,
        Case::PolyOneLane | Case::PolyOneLaneShortRelease | Case::PolySixLanes
    );
    let mut engine =
        Engine::new_with_effects_memory(SAMPLE_RATE, vec![0.0; EFFECT_SAMPLES_PER_LAYER * 2])
            .expect("two-layer effects layout is valid");
    engine.set_filter_oversampling(FilterOversampling::Off);
    engine.apply_patch(&patch);
    (engine, start_poly)
}

fn active_layers(case: Case) -> &'static [LayerId] {
    if matches!(case, Case::StackInactivePayload) {
        &[LayerId::A, LayerId::B]
    } else {
        &[LayerId::A]
    }
}

fn configure_stored_gated_slew(layer: &mut synth_core::LayerPatch) {
    layer.sequence.sequencer_type = SequencerType::Polyphonic;
    configure_gated_payload(layer, true);
}

fn configure_gated(layer: &mut synth_core::LayerPatch, slew: bool) {
    layer.sequence.sequencer_type = SequencerType::Gated;
    layer.sequence.gated_mode = GatedSequencerMode::NoGate;
    configure_gated_payload(layer, slew);
}

fn configure_gated_payload(layer: &mut synth_core::LayerPatch, slew: bool) {
    layer.sequence.gated.tracks[0].destination =
        GatedDestination::Modulation(ModDestination::Osc1Frequency);
    layer.sequence.gated.tracks[1].destination = if slew {
        GatedDestination::Slew
    } else {
        GatedDestination::Off
    };
    for step in 0..16 {
        layer.sequence.gated.tracks[0].steps[step] =
            GatedStep::Value(if step % 2 == 0 { 0 } else { 24 });
        layer.sequence.gated.tracks[1].steps[step] = GatedStep::Value(96);
    }
}

fn configure_poly(layer: &mut synth_core::LayerPatch, lanes: usize) {
    layer.sequence.sequencer_type = SequencerType::Polyphonic;
    for step in 0..64 {
        for lane in 0..lanes {
            layer.sequence.poly.steps[step].lanes[lane] = PolyLaneStep {
                note: PolyNote::Note(60 + lane as u8),
                velocity: PolyVelocity::Velocity(100),
            };
        }
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted[index]
}
