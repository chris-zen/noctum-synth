# Noctum Micro firmware

Hardware firmware for Daisy Seed 1.1. Package name: `noctum-micro`.

- Instrument specs: [Micro 4](../../../docs/src/models/micro-4.md),
  [Micro 1](../../../docs/src/models/micro-1.md)
- Build, flash, feature flags, diagnostics, and profiling: see the
  [Daisy Seed](../../../docs/src/hardware/daisy.md) hardware guide

Model feature sets: [Makefile](../Makefile). Filter defaults: [`src/model.rs`](src/model.rs).

| Piece | Location |
| --- | --- |
| On-target factory-bank layout + flash | [`tools-micro/`](tools-micro/) |
| Host compress | [`tools-host/`](tools-host/) |
| Factory bench | `bench-factory-banks` → `make bench-factory-banks-micro-*` |
