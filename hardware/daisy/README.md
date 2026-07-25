# Daisy platform

This directory contains the Daisy-based implementation of Noctum.

- `embassy-daisy/` is a reusable Embassy board support package.
- `firmware/` is the Daisy Seed application and remains outside the root Cargo
  workspace.
- `kicad/` will hold Daisy-specific KiCad source files, such as the carrier or
  control board.

The local Cargo workspace currently supports Daisy Seed 1.1 audio at 48 kHz
with 32-frame blocks. See the package READMEs for API and flashing details.
