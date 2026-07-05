use std::hint::black_box;
use std::time::{Duration, Instant};

use synth_core::{LANES, LadderFilter, VOICE_PACKS};
use wide::f32x4;

const SAMPLE_RATE: f32 = 44_100.0;
const ITERATIONS: usize = 1_000_000;
const BUFFER_FRAMES: usize = 512;

fn main() {
    let linear_static = time_static_filter(0.65);
    let linear_modulated = time_modulated_filter(0.65);
    let self_osc_1_voice_block = time_static_filter(1.0);
    let self_osc_4_voice_blocks = time_static_voice_blocks(1.0, VOICE_PACKS);

    eprintln!(
        "Filter perf: {ITERATIONS} iterations, {SAMPLE_RATE:.0}Hz, {BUFFER_FRAMES}-frame estimate"
    );
    print_one_block("linear_static", linear_static);
    print_one_block("linear_modulated", linear_modulated);
    print_one_block("self_osc_1_voice_block", self_osc_1_voice_block);
    print_voice_blocks(
        "self_osc_4_voice_blocks_estimate",
        self_osc_4_voice_blocks,
        VOICE_PACKS,
    );

    eprintln!();
    eprintln!("Resonance sweep:");
    for resonance in [0.89, 0.90, 0.95, 0.98, 1.0] {
        let elapsed = time_static_filter(resonance);
        print_sweep(resonance, elapsed);
    }
}

fn print_one_block(label: &str, elapsed: Duration) {
    let ns = ns_per_sample_block(elapsed, ITERATIONS);
    eprintln!(
        "{label:<32} {:>9.2} ns/sample-block  {:>8.3} ms/512f/1block  ({elapsed:?})",
        ns,
        ms_per_buffer(ns, 1),
    );
}

fn print_voice_blocks(label: &str, elapsed: Duration, voice_blocks: usize) {
    let sample_blocks = ITERATIONS * voice_blocks;
    let ns = ns_per_sample_block(elapsed, sample_blocks);
    eprintln!(
        "{label:<32} {:>9.2} ns/sample-block  {:>8.3} ms/512f/{voice_blocks}blocks ({elapsed:?})",
        ns,
        ms_per_buffer(ns, voice_blocks),
    );
}

fn print_sweep(resonance: f32, elapsed: Duration) {
    let ns = ns_per_sample_block(elapsed, ITERATIONS);
    eprintln!(
        "  resonance={resonance:>4.2}: {:>9.2} ns/sample-block  {:>8.3} ms/512f/1block",
        ns,
        ms_per_buffer(ns, 1),
    );
}

fn ns_per_sample_block(elapsed: Duration, sample_blocks: usize) -> f64 {
    elapsed.as_nanos() as f64 / sample_blocks as f64
}

fn ms_per_buffer(ns_per_sample_block: f64, voice_blocks: usize) -> f64 {
    ns_per_sample_block * BUFFER_FRAMES as f64 * voice_blocks as f64 / 1_000_000.0
}

fn time_static_filter(resonance: f32) -> Duration {
    let mut filter = configured_filter(resonance);
    let mut phase = [0.0, 0.17, 0.33, 0.71];
    let phase_inc = [0.013, 0.019, 0.023, 0.031];
    let notes = f32x4::new([48.0, 55.0, 60.0, 67.0]);
    let start = Instant::now();

    for _ in 0..ITERATIONS {
        advance_phase(&mut phase, phase_inc);
        let input = phase_to_signal(phase);
        let out = process(&mut filter, input, notes, SAMPLE_RATE);
        black_box(out);
    }

    start.elapsed()
}

fn time_modulated_filter(resonance: f32) -> Duration {
    let mut filter = configured_filter(resonance);
    filter.set_key_track(0.5);
    filter.set_env_amount(0.45);
    filter.set_audio_mod(0.35);

    let mut phase = [0.0, 0.17, 0.33, 0.71];
    let phase_inc = [0.013, 0.019, 0.023, 0.031];
    let notes = f32x4::new([48.0, 55.0, 60.0, 67.0]);
    let filter_env = f32x4::new([0.2, 0.4, 0.6, 0.8]);
    let start = Instant::now();

    for _ in 0..ITERATIONS {
        advance_phase(&mut phase, phase_inc);
        let input = phase_to_signal(phase);
        let osc1 = input;
        let out = process_modulated(
            &mut filter,
            input,
            notes,
            filter_env,
            f32x4::splat(1.0),
            osc1,
            SAMPLE_RATE,
        );
        black_box(out);
    }

    start.elapsed()
}

fn time_static_voice_blocks(resonance: f32, voice_blocks: usize) -> Duration {
    let mut filters: Vec<LadderFilter> = (0..voice_blocks)
        .map(|_| configured_filter(resonance))
        .collect();
    let mut phase = [0.0, 0.17, 0.33, 0.71];
    let phase_inc = [0.013, 0.019, 0.023, 0.031];
    let notes = f32x4::new([48.0, 55.0, 60.0, 67.0]);
    let start = Instant::now();

    for _ in 0..ITERATIONS {
        advance_phase(&mut phase, phase_inc);
        let input = phase_to_signal(phase);
        for filter in &mut filters {
            let out = process(filter, input, notes, SAMPLE_RATE);
            black_box(out);
        }
    }

    start.elapsed()
}

fn process(filter: &mut LadderFilter, input: f32x4, note: f32x4, sample_rate: f32) -> f32x4 {
    process_modulated(
        filter,
        input,
        note,
        f32x4::splat(0.0),
        f32x4::splat(1.0),
        f32x4::splat(0.0),
        sample_rate,
    )
}

fn process_modulated(
    filter: &mut LadderFilter,
    input: f32x4,
    note: f32x4,
    filter_env: f32x4,
    velocity: f32x4,
    osc1_audio: f32x4,
    sample_rate: f32,
) -> f32x4 {
    filter.process(
        input,
        note,
        filter_env,
        velocity,
        osc1_audio,
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        f32x4::splat(0.0),
        sample_rate,
    )
}

fn configured_filter(resonance: f32) -> LadderFilter {
    let mut filter = LadderFilter::default();
    filter.set_cutoff(1_200.0);
    filter.set_resonance(resonance);
    filter.set_poles(4);
    filter
}

fn advance_phase(phase: &mut [f32; LANES], phase_inc: [f32; LANES]) {
    for lane in 0..LANES {
        phase[lane] += phase_inc[lane];
        if phase[lane] >= 1.0 {
            phase[lane] -= 1.0;
        }
    }
}

fn phase_to_signal(phase: [f32; LANES]) -> f32x4 {
    f32x4::new(phase) * f32x4::splat(2.0) - f32x4::splat(1.0)
}
