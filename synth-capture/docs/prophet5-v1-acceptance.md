# Prophet-5 V acceptance (oscillator-static-v1)

Tracked stub for plan-15 manual acceptance. Full WAVs/NPZs stay under ignored
capture paths (e.g. `~/dev/analog-synth/plans/analog-osc/research/captures/…`).

## Environment (2026-08-01 smoke; invalidated 2026-08-02)

| Item | Value |
| --- | --- |
| Result | **invalid — must recapture with adapter revision 7** |
| Host OS | macOS 15 (darwin 24.6.0) |
| Plugin | Prophet-5 V standalone (operator-reported; pin in project `plugin_version`) |
| MIDI | IAC Driver Bus 2 (exact name) |
| Audio | BlackHole 2ch @ 96 kHz float32, channel 0 |
| MIDI config | `Noctum-Characterisation.promidi` (absolute CC) |
| Adapter revision | 5 (obsolete) |
| Protocol | `oscillator-static-v1` (226 cases) |

Manual session centers confirmed: Fine Tune `0.000`, Pulse Width `50%`, Filter
Env Amount `5.0`.

## Checklist

| Step | Status |
| --- | --- |
| Absolute Learn / `.promidi` import | pass |
| `devices` / `new` / `doctor` (osc 2, three waves) | pass |
| Interrupt + resume without rewriting completed | pass (tooling exercised in automated tests + live ops) |
| 226 complete + `verify` | pass (`plans/analog-osc/research/captures/prophet5-v1`) |
| `extract` → three NPZs × 75 notes | pass |
| Full pitch-grid cycle coherence | **fail**; adjacent spectral cosine drops below the 0.90 bank gate |

## Notes

- The original spot check sampled too little of the pitch grid. Full overlays
  show pitch-dependent corruption across saw, triangle, and pulse in the source
  WAVs, before wavetable construction.
- Adapter revision 5 mislabeled generic Detune as dispersion and did not reset
  the plugin's actual dispersion, modulation, arp/chord, or FX parameters.
- The old bank was also built with a 96 kHz playback reference then exposed at
  48 kHz. It is incompatible at that rate and must not be reused.
- Extraction revision 2 and the bank coherence gate prevent these failures from
  silently producing a releaseable bank, but they cannot repair old audio.
- Pitch often reads a few cents sharp; within protocol limit.
- Lost MIDI map after reinstall previously corrupted mid-run state; remapping +
  fresh project recovered.
- Cursor/sandbox may need mic permission for BlackHole input.

## Prior work citation

Protocol/extraction comparability follows Simionato & Fasciani’s public Korg
Monologue dataset ([DOI 10.5281/zenodo.15196138](https://doi.org/10.5281/zenodo.15196138),
CC-BY-4.0; [DAFx 2025 paper](https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf)).
This Prophet5 acceptance run is software-model capture, not that dataset.
