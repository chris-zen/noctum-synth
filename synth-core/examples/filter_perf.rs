use std::hint::black_box;
use std::time::{Duration, Instant};

use synth_core::dsp::{Filter, FilterOversampling, FilterType};
use synth_core::{LANES, VOICE_PACKS, f32x4};

const SAMPLE_RATE: f32 = 44_100.0;
const ITERATIONS: usize = 1_000_000;
const BUFFER_FRAMES: usize = 512;

fn main() {
    let selected_model = std::env::args().nth(1);
    eprintln!(
        "Filter perf: {ITERATIONS} iterations, {SAMPLE_RATE:.0}Hz, {BUFFER_FRAMES}-frame estimate"
    );
    for filter_type in FilterType::ALL
        .into_iter()
        .filter(|filter_type| filter_type.is_implemented())
        .filter(|filter_type| {
            selected_model
                .as_deref()
                .is_none_or(|selected| filter_type.name() == selected)
        })
    {
        eprintln!();
        eprintln!("Model: {}", filter_type.name());
        print_one_block("linear_static", time_static_filter(filter_type, 0.65));
        print_one_block("linear_modulated", time_modulated_filter(filter_type, 0.65));
        print_one_block(
            "self_osc_1_voice_block",
            time_static_filter(filter_type, 1.0),
        );
        print_voice_blocks(
            "self_osc_4_voice_blocks_estimate",
            time_static_voice_blocks(filter_type, 1.0, VOICE_PACKS),
            VOICE_PACKS,
        );
        print_one_block(
            "self_osc_off_1_voice_block",
            time_static_filter_mode(filter_type, 1.0, FilterOversampling::Off),
        );
        print_voice_blocks(
            "self_osc_off_4_voice_blocks",
            time_static_voice_blocks_mode(filter_type, 1.0, VOICE_PACKS, FilterOversampling::Off),
            VOICE_PACKS,
        );

        eprintln!("Resonance sweep:");
        for resonance in [0.89, 0.90, 0.95, 0.98, 1.0] {
            let elapsed = time_static_filter(filter_type, resonance);
            print_sweep(resonance, elapsed);
        }
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

fn time_static_filter(filter_type: FilterType, resonance: f32) -> Duration {
    time_static_filter_mode(filter_type, resonance, FilterOversampling::Auto)
}

fn time_static_filter_mode(
    filter_type: FilterType,
    resonance: f32,
    oversampling: FilterOversampling,
) -> Duration {
    let mut filter = configured_filter(filter_type, resonance, oversampling);
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

fn time_modulated_filter(filter_type: FilterType, resonance: f32) -> Duration {
    let mut filter = configured_filter(filter_type, resonance, FilterOversampling::Auto);
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

fn time_static_voice_blocks(
    filter_type: FilterType,
    resonance: f32,
    voice_blocks: usize,
) -> Duration {
    time_static_voice_blocks_mode(
        filter_type,
        resonance,
        voice_blocks,
        FilterOversampling::Auto,
    )
}

fn time_static_voice_blocks_mode(
    filter_type: FilterType,
    resonance: f32,
    voice_blocks: usize,
    oversampling: FilterOversampling,
) -> Duration {
    let mut filters: Vec<Filter> = (0..voice_blocks)
        .map(|_| configured_filter(filter_type, resonance, oversampling))
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

fn process(filter: &mut Filter, input: f32x4, note: f32x4, sample_rate: f32) -> f32x4 {
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
    filter: &mut Filter,
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

fn configured_filter(
    filter_type: FilterType,
    resonance: f32,
    oversampling: FilterOversampling,
) -> Filter {
    let mut filter = Filter::new(filter_type);
    filter.set_cutoff(1_200.0);
    filter.set_resonance(resonance);
    filter.set_poles(4);
    filter.set_oversampling(oversampling);
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
