# AGENTS.md

Noctum Synth — Prophet Rev2–inspired VA synth (Rust + Daisy).

| Path | Role |
| --- | --- |
| [`synth-core/`](synth-core/) | `#![no_std]` DSP library |
| [`synth-app/`](synth-app/) | Desktop harness |
| [`synth-tools/`](synth-tools/) | Host-only binaries |
| [`hardware/daisy/firmware/`](hardware/daisy/firmware/) | Micro firmware |
| [`hardware/daisy/embassy-daisy/`](hardware/daisy/embassy-daisy/) | Daisy BSP |

Authoritative Cursor rules: [`.cursor/rules/`](.cursor/rules/) (do not restate here).

`synth-core` stays `no_std`. Host tooling → `synth-tools` only.

## Commands & docs

- Harness, workspace tests (`RUST_MIN_STACK`), tooling entrypoints: [`README.md` § Development](README.md#development), [`README.md` § Running](README.md#running-the-development-harness)
- Core tests / `test-matrix`: [`synth-core/README.md` § Tests](synth-core/README.md#tests-and-benchmarks)
- Tool catalog & per-bin CLI: [`synth-tools/README.md`](synth-tools/README.md)
- mdBook: [`README.md` § Documentation](README.md#documentation) → [`docs/`](docs/)
