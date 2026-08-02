# Korg Monologue reference import v1

## Result

The public Korg Monologue saw, triangle, and square datasets import and extract
reproducibly. All three source files match the MD5 checksums published by
Zenodo. Each file contains 72 rows of 131,072 samples at the published 48 kHz
training rate.

Run the complete pipeline from the repository root:

```bash
python3 scripts/analog_osc_reference.py download --waveform all
python3 scripts/analog_osc_reference.py inspect --waveform all
python3 scripts/analog_osc_reference.py extract --waveform all
```

Source and derived files are cached under
`target/analog-osc/reference/korg-monologue-v1` and are excluded from Git.

## Verified measurements

Default extraction uses 2,048 phase bins, up to 1,024 cycles per pitch, and 256
non-DC harmonics. Results from extractor revision 1 are:

| Waveform | Measured frequency span | Median pitch error | 257-bin harmonic reconstruction median NRMSE |
| --- | ---: | ---: | ---: |
| Saw | 41.03–2400.00 Hz | -1.67 cents | 0.166% |
| Triangle | 20.60–1263.16 Hz | -0.61 cents | 0.016% |
| Square | 20.58–1263.16 Hz | -0.30 cents | 0.229% |

The saw file is empirically about one octave above the triangle and square
files. Its manifest therefore starts at nominal MIDI 28 while the other two
start at MIDI 16. These mappings exist for diagnostics only: models must use
the measured frequency stored for every row. The largest endpoint deviations
from the nominal equal-tempered grids are retained rather than silently tuned
away.

## Evidence boundary

This dataset supports deterministic, pitch-dependent waveform and spectral
modeling. It does not establish all properties of the oscillator itself:

- Every pitch was independently normalized, so absolute output level and its
  frequency dependence cannot be recovered.
- The signal includes the published MOTU M4 recording path; no loopback is
  available to de-embed that path.
- The phase-aligned rows repeat to numerical precision. Extracted period jitter
  and cycle-amplitude variation are consequently zero or near float precision,
  and must not be treated as measurements of hardware stability.
- Only one unit and fixed-width square are represented. There are no pulse-width
  sweeps, PWM, sync, level sweeps, or dynamic pitch sweeps.

Accordingly, this target can train and validate the deterministic branches in
plans 04–08. Drift, jitter, level dependence, PWM, sync, and unit variation need
separate captures before plan 09 can be fitted honestly.

## Provenance

- Dataset: <https://doi.org/10.5281/zenodo.15196138>
- Paper: <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Companion implementation: <https://github.com/RiccardoVib/NeuralOSC>
