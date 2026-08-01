# AGENTS.md

Noctum Synth — Prophet Rev2–inspired VA synth (Rust + Daisy).

| Path | Role |
| --- | --- |
| [`synth-core/`](synth-core/) | `#![no_std]` DSP library |
| [`synth-app/`](synth-app/) | Desktop harness |
| [`synth-tools/`](synth-tools/) | Host-only binaries |
| [`synth-capture/`](synth-capture/) | Host MIDI/audio capture + extraction |
| [`hardware/daisy/firmware/`](hardware/daisy/firmware/) | Micro firmware |
| [`hardware/daisy/embassy-daisy/`](hardware/daisy/embassy-daisy/) | Daisy BSP |

Authoritative Cursor rules: [`.cursor/rules/`](.cursor/rules/) (do not restate here).

`synth-core` stays `no_std`. Host tooling → `synth-tools` only.

## Commands & docs

- Harness, workspace tests (`RUST_MIN_STACK`), tooling entrypoints: [`README.md` § Development](README.md#development), [`README.md` § Running](README.md#running-the-development-harness)
- Oscillator characterisation (capture → extract → measured bank): [`synth-capture/docs/characterise-a-synth.md`](synth-capture/docs/characterise-a-synth.md)
- Core tests / `test-matrix`: [`synth-core/README.md` § Tests](synth-core/README.md#tests-and-benchmarks)
- Tool catalog & per-bin CLI: [`synth-tools/README.md`](synth-tools/README.md)
- mdBook: [`README.md` § Documentation](README.md#documentation) → [`docs/`](docs/)
