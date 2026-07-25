# Hardware Platforms

Noctum runs on different hardware platforms, each with its own build
environment, toolchain, and flashing procedure. Instrument characteristics for
each model — voice count, sample rate, physical I/O — are documented in the
[Models](../models/overview.md) section.

## Platforms

| Platform | Model | Build system | Toolchain |
| --- | --- | --- | --- |
| Daisy Seed 1.1 | [Micro 4](../models/micro-4.md), [Micro 1](../models/micro-1.md) | Cargo (Rust) | `thumbv7em-none-eabihf` |

## Shared architecture

Every platform runs `synth-core` through the same control protocol — `ControlMessage`
events fed to `SynthEngine` from an audio interrupt or callback. The library
is `#![no_std]` and does not depend on platform timers, USB stacks, or audio
drivers. See the [SDK](../sdk/getting-started.md) for the host contract.

Platform-specific build and flashing guides:

- [Daisy Seed](daisy.md) — firmware build, feature flags, flashing, diagnostics, profiling
