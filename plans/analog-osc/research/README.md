# Analog oscillator reference data

This directory contains committed metadata and tooling inputs only. Downloaded
recordings and derived arrays belong under `target/analog-osc/reference` and
must not be committed.

The initial targets are deliberately separate:

- `korg-monologue-v1.json` describes the public real-hardware dataset from
  Simionato and Fasciani.
- `prophet5-v1.capture.json` is a capture specification for the
  software reference. It is not evidence about Prophet Rev2 hardware.

Import and inspect one Monologue waveform:

```bash
python3 scripts/analog_osc_reference.py download --waveform saw
python3 scripts/analog_osc_reference.py inspect --waveform saw
python3 scripts/analog_osc_reference.py extract --waveform saw
```

Use `--waveform all` to process saw, triangle, and square. The importer refuses
to unpickle a file until its MD5 digest matches the value published by Zenodo.
Python 3.9+ and NumPy are required for inspection and extraction; downloading
uses only the standard library.

Extraction writes a compressed NPZ with phase-normalized median cycles,
complex harmonics, and per-pitch measurements, plus a JSON summary recording
the source checksum and extraction settings. The source pickle remains
unchanged.

The published files contain 131,072 samples for each of 72 pitches. Inspection
also shows that the saw file starts approximately one octave above the triangle
and square files. Those per-file nominal mappings are pinned in the manifest,
but extraction always measures the fundamental and downstream fitting must use
that measured value. See `reports/korg-monologue-import-v1.md` for the verified
import results and limitations.

Current Plan 04 evidence is organized as follows:

- `reports/korg-monologue-phase-filter-v1.md`: original v1 fit and runtime
  evidence, now explicitly superseded for promotion.
- `reports/korg-monologue-phase-filter-listening-v1.md`: first blind ABX and
  two-choice target-match result.
- `reports/korg-monologue-phase-filter-ablation-listening-v1.md`: four-way
  phase/filter ranking diagnostic.
- `reports/korg-monologue-phase-filter-ablation-curves-v1.md`: full validation
  curves, metric/listening comparison, and v2 decision.
- `plots/korg-monologue-phase-filter-ablation-curves-v1.svg`: visual curves;
  open squares mark the six diagnostic listening cases.
- `profiles/korg-monologue-phase-filter-v2.json`: phase-invariant,
  production-source v2 coefficients, provenance, and split metrics.
- `reports/korg-monologue-phase-filter-v2.md`: v2 objective, phase-zero policy,
  fit/runtime results, residual gate, completed listening gate, and decision.
- `reports/korg-monologue-phase-filter-runtime-v2.json`: compiled release
  runtime results for all validation and untouched test pitches.
- `reports/korg-monologue-phase-filter-sweeps-v2.json`: 48/96 kHz static
  residual comparison against the production baseline.
- `reports/korg-monologue-phase-filter-listening-v2.md`: fresh test-split blind
  protocol, completed listener result, and negative promotion decision.
- `reports/korg-monologue-phase-filter-listening-v2.json`: decoded per-case
  responses, aggregate statistics, and immutable input hashes.
- `reports/korg-monologue-measured-wavetable-v1.md`: Plan 07 training-only
  representation comparison, phase findings, storage estimate, and runtime
  selection.
- `reports/korg-monologue-measured-wavetable-v1.json`: per-pitch metrics for
  nearest, complex-interpolated, canonicalized, and residual representations.
- `banks/korg-monologue-measured-bank-v1.json`: versioned external-bank schema,
  source provenance, layout, pitch guards, and binary checksums.
- `reports/korg-monologue-measured-wavetable-runtime-v1.json`: compiled runtime
  results for every held-out pitch.
- `reports/korg-monologue-measured-wavetable-sweeps-v1.json`: compiled 48/96 kHz
  static non-harmonic residual gate.
- `reports/korg-monologue-measured-wavetable-shape-sweeps-v1.json`: 120-case
  measured hybrid shape/PWM residual sweep at 48/96 kHz.
- `reports/korg-monologue-measured-wavetable-dynamic-v1.md`: compact dynamic
  outcome, PWM defect correction, desktop placement, and limitations.
- `reports/korg-monologue-measured-wavetable-dynamic-v1.json`: native 48 kHz
  versus filtered 192 kHz comparisons plus full-engine percentile timings.
- `reports/korg-monologue-measured-wavetable-listening-v1.md`: completed blind
  protocol, result interpretation, and bounded-continuation decision.
- `reports/korg-monologue-measured-wavetable-listening-v1.json`: decoded
  responses, aggregate statistics, and immutable package hashes.
- `target/analog-osc/listening/korg-monologue-measured-wavetable-v1`: ignored,
  reproducible nine-case blind package generated from previously unheard test
  rows.

References:

- Dataset DOI: <https://doi.org/10.5281/zenodo.15196138>
- Companion code: <https://github.com/RiccardoVib/NeuralOSC>
- Paper: <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
