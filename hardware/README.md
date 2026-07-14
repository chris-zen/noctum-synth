# Hardware

This directory contains the hardware implementation of Analog Synth. It is
intentionally outside the root Rust workspace: each hardware platform owns its
own firmware toolchain, build configuration, and release process.

## Layout

- `daisy/` contains the first supported hardware platform.

Future platforms should be added as siblings of `daisy/`.

## Status

The Daisy platform contains an Embassy-based board support package and a
hardware-proof firmware application. Other hardware platforms have not been
added yet.
