# synth-core

Portable, `no_std` virtual-analog synthesis library for Rust.

## Overview

`synth-core` contains the full sound engine: band-limited oscillators, ladder
filter, envelopes, LFOs, and 16-voice voice management.

The underlying algorithms are based on *Designing Software Synthesizer Plugins
in C++* by Will C. Pirkle (BLEP/polyBLEP oscillators, ladder filter, envelope
curves, noise generation, and related techniques). They have been adapted to
match the Prophet Rev2 voice architecture and reimplemented in Rust with
four-wide SIMD (`wide::f32x4`) so each [`VoiceBlock`](src/voice/) renders
four notes per sample step.

The public entry point for hosts is [`SynthEngine`](src/engine.rs). Parameter
and performance changes arrive as [`ControlMessage`](src/lib.rs) values; patch
layout and defaults live under [`patch`](src/patch.rs).

## Modules

| Module | Purpose |
|--------|---------|
| [`engine`](src/engine.rs) | Top-level render loop and master volume |
| [`voice`](src/voice/) | Voice manager (polyphony, stealing) and per-block signal chain |
| [`dsp`](src/dsp/) | Generic DSP (oscillators, filter, envelopes, LFOs) |
| [`patch`](src/patch.rs) | Parameter structs and LFO destinations |
| [`noise`](src/dsp/noise.rs) | White (and internal pink) noise |
| [`tuning`](src/tuning.rs) | MIDI note → Hz conversion |

## Example

```rust
use synth_core::{ParamId, SynthEngine};

let sample_rate = 44_100.0;
let mut engine = SynthEngine::new(sample_rate);

engine.set_param(ParamId::FilterCutoff, 0.4);
engine.note_on(57, 0.9);

let mut interleaved = [0.0f32; 1024];
engine.process_interleaved(&mut interleaved, 2);
```

## Tests and benchmarks

Some integration tests (notably `engine_tests`) construct a large
`SynthEngine` on the stack. Raise the Rust thread stack with
`RUST_MIN_STACK` (at least 16 MiB) or those tests abort with a stack overflow:

```bash
RUST_MIN_STACK=16777216 cargo test -p synth-core
RUST_MIN_STACK=16777216 cargo test -p synth-core --tests
make -C synth-core test-matrix
cargo run --release -p synth-tools --bin voice_block_perf
cargo run --release -p synth-tools --bin filter_perf
```

Official Sequential factory-bank regressions are opt-in integration tests.
They need the gitignored factory `.syx` archives on disk and:

```bash
RUST_MIN_STACK=16777216 cargo test -p synth-core --features official-sysex-fixtures --test official_sysex_fixtures
```

The `synth-core` Makefile sets `RUST_MIN_STACK` and runs a curated feature
matrix (widths × math backends, plus Daisy-like smoke configs) via
`make -C synth-core test-matrix`.

## Documentation

```bash
cargo doc --no-deps --open -p synth-core
```
