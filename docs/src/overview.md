# Analog Synth

Analog Synth is a Rust virtual-analog polysynth project. It takes inspiration
from the Sequential Prophet Rev2 approach to a broad, modulation-rich
subtractive instrument: two oscillators per voice, a resonant low-pass filter,
three envelopes, four LFOs, flexible routing, stereo spread, and global
effects. It is an original implementation, not a clone.

The project has three distinct pieces:

- The **Synthesizer** is the instrument itself: its sound engine, controls,
  voice behavior, and effects.
- The **Application** is `synth-app`, a desktop harness for playing, editing,
  and observing the synth while it is under development. It is not a product
  distribution target.
- The **SDK** is the `synth-core` library and its host integration contract.
  It is intended for software hosts today and leaves a portable boundary for a
  future hardware implementation.

## Reading this book

Start with **Synthesizer** when you want to understand what the instrument can
do or how a parameter changes a sound. It deliberately avoids Rust API detail.

Read **Application** to use the development harness for auditioning patches,
MIDI, audio devices, and live analysis.

Read **SDK** when you are embedding `SynthEngine` in a program, building a
wrapper, or working on the DSP implementation. That section contains the Rust
types, audio-thread contract, data flow, and development commands.

The **Hardware** section is a reserved home for the future physical
implementation.

## Quick start

Run the development harness:

```bash
cargo run --release -p synth-app
```

Serve this book locally:

```bash
mdbook serve docs
```
