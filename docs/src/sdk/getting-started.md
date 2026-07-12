# SDK: Getting Started

`synth-core` is the portable Rust sound-engine library. It is `#![no_std]` and
keeps DSP, patch state, voice allocation, modulation, and rendering independent
of desktop UI, device enumeration, file storage, or MIDI backends. A host can
therefore use it in a desktop app, plugin wrapper, embedded runtime, or other
audio environment.

The host-facing entry point is `SynthEngine`. Construct it at the audio sample
rate, submit control messages, and render into a buffer from the audio thread.

## Minimal host

```rust,ignore
use synth_core::{ParamId, SynthEngine};

let mut engine = SynthEngine::new(48_000.0);
engine.set_param(ParamId::FilterCutoff, 4_000.0);
engine.note_on(60, 0.9);

let mut stereo = [0.0_f32; 256 * 2];
engine.process_interleaved(&mut stereo, 2);
```

`process_interleaved` accepts an interleaved buffer and a channel count. A
single channel receives a mono sum; for two or more channels, left is written
to even channels and right to odd channels. The engine expects an audio buffer
whose length is a whole number of frames; any incomplete tail is ignored.

## Main SDK types

| Type | Role |
| --- | --- |
| `SynthEngine` | Owns voices, effects, master volume, and rendering. |
| `ControlMessage` | Host-to-engine protocol for notes, parameters, modulation, performance data, and filter quality. |
| `ParamId` | Address of one continuous or indexed synth setting. |
| `Patch` | Complete serializable patch snapshot, including modulation and effects. |
| `ModRoute`, `ModSource`, `ModDestination` | Modulation routing model. |
| `FilterOversampling` | Nonlinear-filter quality policy. |

## Applying a patch

`Patch` represents the full stored patch surface. To load it into a running
engine, forward every parameter and modulation entry to the same control path
used by the host UI:

```rust,ignore
use synth_core::{ControlMessage, Patch, SynthEngine};

fn apply_patch(engine: &mut SynthEngine, patch: &Patch) {
    patch.for_each_param(|id, value| engine.handle_control(ControlMessage::SetParam(id, value)));
    patch.for_each_modulation(|route, enabled, source, destination, amount| {
        engine.handle_control(ControlMessage::SetModulation {
            route,
            enabled,
            source,
            destination,
            amount,
        });
    });
}
```

When the optional `serde` feature is enabled, `Patch` can participate in a
host's chosen serialization workflow. The SDK does not prescribe a filesystem,
configuration format, UI toolkit, audio library, or MIDI library.
