# Daisy platform

This directory contains the Daisy-based implementation of Noctum.

- `embassy-daisy/` is a reusable Embassy board support package.
- `firmware/` is the Daisy Seed application (`noctum-micro`), with
  `firmware/tools-micro/` (on-target factory-bank layout + flasher) and
  `firmware/tools-host/` (host compress).
- `kicad/` will hold Daisy-specific KiCad source files, such as the carrier or
  control board.

The local Cargo workspace currently supports Daisy Seed 1.1 audio at 48 kHz
with 32-frame blocks. Micro model feature sets (`FEATURES_micro-4`,
`FEATURES_micro-1`) live in the [Makefile](Makefile). See the package READMEs
and the [Daisy Seed](../../docs/src/hardware/daisy.md) guide for API and
flashing details.
