# Arturia Prophet-5 V acceptance (oscillator-static-v1)

Tracked stub for plan-15 manual acceptance. Full WAVs/NPZs stay under ignored
capture paths (e.g. `~/dev/analog-synth/plans/analog-osc/research/captures/…`).

## Environment (2026-08-01 smoke)

| Item | Value |
| --- | --- |
| Result | **pass** |
| Host OS | macOS 15 (darwin 24.6.0) |
| Plugin | Arturia Prophet-5 V standalone (operator-reported; pin in project `plugin_version`) |
| MIDI | IAC Driver Bus 2 (exact name) |
| Audio | BlackHole 2ch @ 96 kHz float32, channel 0 |
| MIDI config | `Noctum-Characterisation.promidi` (absolute CC) |
| Adapter revision | 5 |
| Protocol | `oscillator-static-v1` (226 cases) |

Manual session centers confirmed: Fine Tune `0.000`, Pulse Width `50%`, Filter
Env Amount `5.0`.

## Checklist

| Step | Status |
| --- | --- |
| Absolute Learn / `.promidi` import | pass |
| `devices` / `new` / `doctor` (osc 2, three waves) | pass |
| Interrupt + resume without rewriting completed | pass (tooling exercised in automated tests + live ops) |
| 226 complete + `verify` | pass (`plans/analog-osc/research/captures/arturia-prophet5-v1`) |
| `extract` → three NPZs × 75 notes | pass |
| Spot-check cycles / pitch | pass (max \|cents\| ≈ 11 on smoke set; within ±50¢ gate) |

## Notes

- Pitch often reads a few cents sharp; within protocol limit.
- Lost MIDI map after reinstall previously corrupted mid-run state; remapping +
  fresh project recovered.
- Cursor/sandbox may need mic permission for BlackHole input.

## Prior work citation

Protocol/extraction comparability follows Simionato & Fasciani’s public Korg
Monologue dataset ([DOI 10.5281/zenodo.15196138](https://doi.org/10.5281/zenodo.15196138),
CC-BY-4.0; [DAFx 2025 paper](https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf)).
This Arturia acceptance run is software-model capture, not that dataset.
