//! Isolated release benchmark for the private sequencer playback runtimes.

use std::hint::black_box;
use std::time::Instant;

use synth_core::sequencer::benchmark::{
    run_clock_samples, run_gated_samples, run_poly_samples, sequencer_runtime_sizes,
};

const SAMPLES: usize = 50_000_000;

fn main() {
    let sizes = sequencer_runtime_sizes();
    println!(
        "runtime sizes: clock={} gated={} poly={} bytes",
        sizes[0], sizes[1], sizes[2]
    );
    measure("clock", || run_clock_samples(SAMPLES));
    measure("gated_no_slew", || run_gated_samples(SAMPLES, false));
    measure("gated_slew", || run_gated_samples(SAMPLES, true));
    measure("poly_six_lane", || run_poly_samples(SAMPLES));
}

fn measure(name: &str, run: impl FnOnce() -> u64) {
    let started = Instant::now();
    black_box(run());
    let elapsed = started.elapsed();
    println!(
        "{name:<18} total={:.3}ms ns/sample={:.3}",
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_nanos() as f64 / SAMPLES as f64,
    );
}
