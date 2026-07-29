# SDK: Engine Architecture

This page describes implementation ownership and data flow. It is intentionally
separate from the Synthesizer guide, where the same signal path is explained in
musical terms.

`SynthEngine` owns the physical `VoicePool`, effects memory, MIDI clock, rate
adapter, and output limiter. A `LayerEngine` owns logical allocation, held-note,
patch modulation, arpeggiator, effect state, tempo, division, and program-volume
state. The one-layer engine assigns the complete pool to that layer.

```mermaid
flowchart TD
    Host["Host control queue"]
    Control["ControlMessage"]
    Engine["SynthEngine"]
    Layer["LayerEngine"]
    Pool["VoicePool"]
    Blocks["4 VoiceBlocks x 4 SIMD lanes"]
    Sum["Stereo voice sum"]
    FX["Effects"]
    Output["Interleaved host buffer"]

    Host --> Control
    Control --> Engine
    Engine --> Layer
    Engine --> Pool
    Layer --> Pool
    Pool --> Blocks
    Blocks --> Sum
    Sum --> FX
    FX --> Output
```

## Voice layout

The engine has 16 voices. They are implemented as four `VoiceBlock` instances,
each operating on four `wide::f32x4` lanes. A lane carries its own note,
velocity, gate, envelopes, LFO outputs, and filter state. Patch parameters are
shared across the lanes in a block.

```mermaid
flowchart TD
    Layer["LayerEngine: allocation and patch state"]
    Pool["VoicePool: 16 physical voices"]
    B0["VoiceBlock 0: lanes 0-3"]
    B1["VoiceBlock 1: lanes 4-7"]
    B2["VoiceBlock 2: lanes 8-11"]
    B3["VoiceBlock 3: lanes 12-15"]
    Mix["Stereo mix"]

    Layer --> Pool
    Pool --> B0
    Pool --> B1
    Pool --> B2
    Pool --> B3
    B0 --> Mix
    B1 --> Mix
    B2 --> Mix
    B3 --> Mix
```

## Portability boundary

The core library does not depend on `std`, CPAL, egui, MIDI enumeration, or
desktop configuration. `synth-app` owns those concerns. This boundary is the
reason hosts can use the same control protocol and DSP engine across different
software runtimes, and it is the intended starting point for future hardware
work.
