# synth-core

Portable, `no_std` virtual-analog synthesis library for Rust.

## Overview

`synth-core` contains the full sound engine: band-limited oscillators, ladder
filter, envelopes, LFOs, and 16-voice voice management.

The underlying algorithms are based on *Designing Software Synthesizer Plugins
in C++* by Will C. Pirkle (BLEP/polyBLEP oscillators, ladder filter, envelope
curves, noise generation, and related techniques). They have been adapted to
match the Prophet Rev2 voice architecture and reimplemented in Rust with
four-wide SIMD (`wide::f32x4`) so each [`VoiceBlock`](src/voice.rs) renders
four notes per sample step.

The public entry point for hosts is [`SynthEngine`](src/engine.rs). Parameter
and performance changes arrive as [`ControlMessage`](src/lib.rs) values; patch
layout and defaults live under [`patch`](src/patch.rs).

## Modules

| Module | Purpose |
|--------|---------|
| [`engine`](src/engine.rs) | Top-level render loop and master volume |
| [`voices`](src/voices.rs) | Polyphony, voice stealing, note routing |
| [`voice`](src/voice.rs) | Per-block signal chain (osc → filter → amp) |
| [`analog_oscillator`](src/analog_oscillator.rs) | Single SIMD oscillator (BLEP / polyBLEP) |
| [`analog_oscillators`](src/analog_oscillators.rs) | Dual-osc mixer, sub, noise, sync |
| [`filter`](src/filter.rs) | Nonlinear ladder low-pass |
| [`envelope`](src/envelope.rs) | Delayed ADSR with linear or analog curves |
| [`lfo`](src/lfo.rs) | Four-lane LFO |
| [`patch`](src/patch.rs) | Parameter structs and LFO destinations |
| [`noise`](src/noise.rs) | White (and internal pink) noise |
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

```bash
cargo test -p synth-core
cargo run --release -p synth-core --example voice_block_perf
cargo run --release -p synth-core --example filter_perf
```

## Documentation

```bash
cargo doc --no-deps --open -p synth-core
```
