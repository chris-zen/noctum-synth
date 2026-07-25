# Noctum

A hobby project to build a virtual-analog synthesizer inspired by the
**Sequential Prophet Rev2** — the 16-voice, modulation-rich polysynth that
defined a generation of pad, lead, and bass sounds.

The goal is to capture the character of a large subtractive synth: warm
oscillators, a resonant low-pass filter, deep envelopes, and enough routing to
build evolving pads, aggressive leads, and animated bass lines. It is not a
clone, but the voice architecture and modulation philosophy follow the Rev2
closely enough that familiar patches should translate in spirit.

The DSP algorithms draw on *Designing Software Synthesizer Plugins in C++* by
Will C. Pirkle — band-limited oscillators, ladder filters, envelopes, and
related building blocks — restructured to follow the Rev2 voice layout and
rewritten in Rust with SIMD so multiple voices are rendered in parallel.

## The instrument

**16 voices of polyphony** — full keyboard coverage for chords and layered
textures, with voice stealing when you exceed the limit.

**Dual analog-style oscillators** per voice, each with saw, saw/triangle morph,
triangle, and pulse waveforms. Per-oscillator fine tune, shape, level, glide,
and optional hard sync. A global pitch-slop control adds the subtle instability
that keeps static patches from sounding sterile.

**Sub oscillator and white noise** for weight and air, mixed alongside the main
oscillators.

**Low-pass filter** switchable between 2-pole and 4-pole slope, with cutoff,
resonance, keyboard tracking, envelope amount, velocity sensitivity, and
audio-rate modulation. Push the resonance and the filter sings on its own.

**Three envelopes** — filter, amplifier, and auxiliary — each a full DADSR with
delay. The aux envelope can loop its attack and decay for rhythmic modulation.

**Four LFOs** with multiple waveforms, each routable to pitch, timbre, filter,
levels, pan, or the other LFOs. Clock sync and key sync keep modulation locked to
your playing.

**Stereo output** with programmable pan spread across voices.

## Project structure

The sound engine lives in `synth-core`, a portable library intended to
eventually power real products — a **VST instrument plugin** and/or **hardware
synth firmware**.

`synth-app` is a desktop application used only during development: a way to
audition patches, tweak parameters, and analyse the output while the engine is
still taking shape. It provides a parameter editor, MIDI input, and a
detachable analysis viewport for watching the spectrum and filter response in
real time. It is not intended for distribution.

See [synth-core/README.md](synth-core/README.md) for library documentation.

## Documentation

Project documentation lives in [`docs`](docs) and is built with
[mdBook](https://rust-lang.github.io/mdBook/) plus Mermaid diagrams. It is
organized into an overview, a player-facing synthesizer guide, the development
application harness, the `synth-core` SDK, and a future hardware section.

Install the documentation tools:

```bash
cargo install mdbook mdbook-mermaid
```

Serve the docs locally:

```bash
mdbook serve docs
```

Build the static documentation site:

```bash
mdbook build docs
```

## Requirements

- [Rust](https://rustup.rs/) 1.93 or newer
- An audio output device (for running the development harness)
- Optional: a MIDI keyboard or controller

## Running the development harness

```bash
cargo build --release
cargo run --release -p synth-app
```

To pick a specific MIDI port, audio output device, and audio input device by name:

```bash
cargo run --release -p synth-app -- "MIDI Port Name" "Output Device Name" "Input Device Name"
```

Available audio devices are listed on startup. MIDI port selection is also
available in the Settings tab and saved between sessions. The Settings tab also
lets you pick an **audio input device** (its signal is summed into the synth
output) and a **sample rate**; the input must match the output sample rate, and
both settings take effect on the next launch.

## Development

```bash
RUST_MIN_STACK=16777216 cargo test --workspace
cargo doc --no-deps --open -p synth-core
cargo run --release -p synth-core --example voice_block_perf
cargo run --release -p synth-core --example filter_perf
```

Some `synth-core` integration tests (notably `engine_tests`) build large
`SynthEngine` values on the stack. The default Rust thread stack is too small
and fails with a stack overflow / “memory allocation of … bytes failed”
style abort. Set `RUST_MIN_STACK` to at least **16 MiB** (`16777216`) when
running those tests:

```bash
RUST_MIN_STACK=16777216 cargo test -p synth-core --tests
```

## License

MIT — see the `license` field in each crate's `Cargo.toml`.
