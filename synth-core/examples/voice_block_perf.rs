use std::hint::black_box;
use std::time::{Duration, Instant};

use synth_core::{ModDestination, PerformanceModulation, VoiceBlock};

const SAMPLE_RATE: f32 = 44_100.0;
const ITERATIONS: usize = 200_000;
const BUFFER_FRAMES: usize = 512;
const VOICE_BLOCKS_FOR_16_VOICES: usize = 4;

fn main() {
    eprintln!(
        "VoiceBlock perf: {ITERATIONS} iterations, {SAMPLE_RATE:.0}Hz, {BUFFER_FRAMES}-frame estimate"
    );

    print_case("neutral_filter", time_case(configured_block));
    print_case("active_filter", time_case(active_filter_block));
    print_case("modulation_heavy", time_case(modulation_heavy_block));
    print_case("self_oscillation", time_case(self_oscillation_block));
}

fn print_case(label: &str, elapsed: Duration) {
    let ns = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    eprintln!(
        "{label:<24} {:>9.2} ns/sample-block  {:>8.3} ms/512f/1block  {:>8.3} ms/512f/4blocks ({elapsed:?})",
        ns,
        ms_per_buffer(ns, 1),
        ms_per_buffer(ns, VOICE_BLOCKS_FOR_16_VOICES),
    );
}

fn ms_per_buffer(ns_per_sample_block: f64, voice_blocks: usize) -> f64 {
    ns_per_sample_block * BUFFER_FRAMES as f64 * voice_blocks as f64 / 1_000_000.0
}

fn time_case(make_block: fn() -> VoiceBlock) -> Duration {
    let mut block = make_block();

    for _ in 0..4096 {
        black_box(block.next(PerformanceModulation::default()));
    }

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(block.next(PerformanceModulation::default()));
    }
    start.elapsed()
}

fn configured_block() -> VoiceBlock {
    let mut block = VoiceBlock::new(SAMPLE_RATE);
    for (lane, note) in [48, 55, 60, 67].into_iter().enumerate() {
        let pan_side = if lane % 2 == 0 { -1.0 } else { 1.0 };
        block.note_on(lane, note, 1.0, pan_side, false);
    }
    block
}

fn active_filter_block() -> VoiceBlock {
    let mut block = configured_block();
    block.filter.set_cutoff(1_200.0);
    block.filter.set_resonance(0.65);
    block
}

fn modulation_heavy_block() -> VoiceBlock {
    let mut block = active_filter_block();
    block.filter.set_key_track(0.5);
    block.filter.set_env_amount(0.5);
    block.filter.set_env_velocity_amount(0.5);
    block.filter.set_audio_mod(0.35);
    block.oscillators.set_osc2_enabled(true);
    block.oscillators.set_noise(0.15);
    block.oscillators.set_sub_octave(0.2);
    block.set_pan_spread(1.0);
    block.set_lfo_rate_hz(0, 0.9);
    block.set_lfo_depth(0, 0.7);
    block.set_lfo_destination(0, ModDestination::FilterCutoff);
    block.set_lfo_rate_hz(1, 1.3);
    block.set_lfo_depth(1, 0.4);
    block.set_lfo_destination(1, ModDestination::Pan);
    block.set_lfo_rate_hz(2, 2.1);
    block.set_lfo_depth(2, 0.3);
    block.set_lfo_destination(2, ModDestination::Vca);
    block.set_lfo_rate_hz(3, 0.5);
    block.set_lfo_depth(3, 0.25);
    block.set_lfo_destination(3, ModDestination::OscAllFrequency);
    block
}

fn self_oscillation_block() -> VoiceBlock {
    let mut block = active_filter_block();
    block.filter.set_cutoff(440.0);
    block.filter.set_resonance(1.0);
    block
}
