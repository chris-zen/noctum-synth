# SDK: Development Workflow

The workspace contains `synth-core`, the portable DSP library, and `synth-app`,
the desktop harness. Rust API reference material is generated separately from
this narrative book.

## Requirements

- Rust 1.93 or newer.
- An audio output device to run `synth-app`.
- Optional MIDI keyboard or controller.
- `mdbook` and `mdbook-mermaid` to author this book.

Install the documentation tools:

```bash
cargo install mdbook mdbook-mermaid
```

## Common commands

Run all workspace tests:

```bash
cargo test --workspace
```

Generate Rust API documentation:

```bash
cargo doc --no-deps -p synth-core
```

Run the development harness:

```bash
cargo run --release -p synth-app
```

Run performance examples:

```bash
cargo run --release -p synth-core --example voice_block_perf
cargo run --release -p synth-core --example filter_perf
```

Serve or build the documentation site:

```bash
mdbook serve docs
mdbook build docs
```

## Writing this book

Source files live under `docs/src`; `docs/src/SUMMARY.md` controls navigation.
Keep Synthesizer pages in player and sound-designer language. Keep Rust types,
threading, data layout, and host code in SDK pages. Mermaid figures stay in
Markdown fenced blocks and are transformed by `mdbook-mermaid`.
