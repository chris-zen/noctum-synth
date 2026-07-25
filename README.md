# Noctum

A hobby project to build a virtual-analog synthesizer inspired by the
**Sequential Prophet Rev2** — the 16-voice, modulation-rich polysynth that
defined a generation of pad, lead, and bass sounds. It is not a clone, but the
voice architecture and modulation philosophy follow the Rev2 closely enough that
familiar patches should translate in spirit.

The DSP algorithms draw on *Designing Software Synthesizer Plugins in C++* by
Will C. Pirkle, rewritten in Rust with SIMD so multiple voices are rendered in
parallel.

## Models

| Model | Platform | Voices | Status |
| --- | --- | --- | --- |
| [Micro 4](docs/src/models/micro-4.md) | Daisy Seed 1.1 | 4 | Current |
| [Micro 1](docs/src/models/micro-1.md) | Daisy Seed 1.1 | 1 | Planned |
| [Mini](docs/src/models/mini.md) | Raspberry Pi Zero | TBD | Planned |

## Project structure

- `synth-core` — portable `#![no_std]` DSP library (voice engine, effects, MIDI codec)
- `synth-app` — desktop development harness (egui + CPAL + midir)
- `hardware/daisy/firmware` — Micro firmware (`noctum-micro`) for Daisy Seed 1.1
- `hardware/daisy/embassy-daisy` — Embassy-based BSP for Daisy Seed

## Running the development harness

```bash
cargo build --release
cargo run --release -p synth-app
```

```bash
cargo run --release -p synth-app -- "MIDI Port Name" "Output Device Name" "Input Device Name"
```

## Documentation

Project documentation is built with [mdBook](https://rust-lang.github.io/mdBook/)
and served at [`docs`](docs):

```bash
cargo install mdbook mdbook-mermaid
mdbook serve docs
```

## Requirements

- [Rust](https://rustup.rs/) 1.93 or newer
- An audio output device
- Optional: a MIDI keyboard or controller

## Development

```bash
RUST_MIN_STACK=16777216 cargo test --workspace
cargo doc --no-deps --open -p synth-core
cargo run --release -p synth-core --example voice_block_perf
cargo run --release -p synth-core --example filter_perf
```

Some `synth-core` integration tests build large `SynthEngine` values on the
stack. Set `RUST_MIN_STACK` to at least **16 MiB** (`16777216`):

```bash
RUST_MIN_STACK=16777216 cargo test -p synth-core --tests
```

## License

MIT — see the `license` field in each crate's `Cargo.toml`.
