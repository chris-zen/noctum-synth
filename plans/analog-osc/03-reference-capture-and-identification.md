# Reference Capture and Model Identification

## Objective

Build trustworthy, named reference datasets and derive the deterministic and
stochastic features needed by every target-conditioned oscillator experiment.
Prevent one screenshot, one pitch, or an unknown signal chain from becoming the
definition of analog character.

## Implementation status

The first reproducible reference pipeline is implemented:

- `plans/analog-osc/research/targets/korg-monologue-v1.json` pins provenance,
  published checksums, per-wave pitch mappings, capture limitations, and the
  train/validation/test split.
- `scripts/analog_osc_reference.py` downloads, checksum-verifies, inspects, and
  extracts all three published waveforms without manual file editing.
- Derived NPZ files contain phase-normalized robust median cycles, measured
  fundamental frequencies, source conditioning, and complex harmonics.
- Companion JSON summaries contain scalar measurements and the checksum of
  each derived NPZ.
- `scripts/test_analog_osc_reference.py` verifies phase-domain extraction and
  the deterministic data split with a non-bin-centered synthetic oscillator.
- `plans/analog-osc/research/reports/korg-monologue-import-v1.md` records validation
  results and the evidence boundary of the published data.

Downloaded and derived arrays remain under `target/analog-osc/reference` and
are intentionally ignored by Git.

## Initial targets

### Korg Monologue analog VCO

Use the public Analog and Synthetic VCO dataset as the first real-hardware
reference:

- Saw, triangle, and square.
- 72 pitches from E0 to E6.
- Original recording at 96 kHz; published training data at 48 kHz.
- Phase-aligned files.
- CC BY 4.0.

The published v1 pickle files have 131,072 samples per pitch. Empirical
inspection found the saw grid approximately one octave above the triangle and
square grids; therefore each file has an explicit nominal first MIDI note and
all fitting uses the extracted measured fundamental rather than trusting the
label alone.

This target lacks swept pulse width, sync, and multiple physical units, but it
is sufficient to validate frequency-dependent deterministic models.

### Arturia Prophet-5 V

Create a separate software-reference dataset using the virtual cable:

- Capture without ADC/DAC or audio-interface coupling.
- Record saw, triangle, pulse at multiple widths, Saw+Tri, pitch sweeps, PWM,
  sync where available, and output-level sweeps.
- Record plugin version, preset/init state, oscillator level, sample rate,
  oversampling/quality setting, host, and every supposedly bypassed stage.
- Treat the result as Arturia output character, not Prophet Rev2 hardware.

### Future hardware

Each hardware unit is its own target profile. At minimum record model, serial
or anonymous unit ID, warm-up time, tuning/calibration state, interface,
sample rate, input gain, and date. Capture a loopback transfer measurement so
interface high-pass, low-pass, and phase response can be estimated or explicitly
retained as part of the target.

## Manifest and storage

Each target has a versioned manifest containing:

- Stable target ID, display name, source kind, and license.
- Device/plugin version and unit identifier.
- Capture chain, rates, bit depth, clocking, and channel.
- Waveform, requested note, measured fundamental, pulse width, level, and
  modulation state per file.
- Warm-up and discarded transient duration.
- Whether the data is raw, de-embedded, resampled, normalized, or phase aligned.
- Checksums of source files and processing configuration.

Large audio stays outside Git. Commit the manifest, download/capture script
configuration, compact derived coefficients, and summarized reports. Cache
derived data by source checksum.

## Capture matrix

For a new target:

- Notes: every semitone E0-E6 when practical; otherwise E and A in each octave
  plus additional transition notes.
- Saw, triangle, and 50-percent pulse/square for every note.
- Pulse widths: 10, 25, 50, 75, 90 percent at least once per octave.
- At least eight seconds per static condition for long-run variability.
- Output levels: nominal plus two lower and two higher settings when level can
  alter saturation or waveform.
- Dynamic: logarithmic pitch sweep, pulse-width sweep, PWM at several rates,
  hard-sync ratios, note reset/free-running behavior.
- Silence/noise-floor capture and hardware loopback where applicable.

Capture at 96 kHz or higher for identification even if a runtime target is
48 kHz. Preserve the original file before downsampling.

## Deterministic extraction

For every static recording:

1. Estimate instantaneous fundamental and reject startup/tuning transients.
2. Segment cycles using continuous phase rather than nearest integer sample
   periods.
3. Align polarity and phase to a documented waveform landmark.
4. Estimate a robust median/trimmed-mean cycle for deterministic shape.
5. Preserve complex harmonic coefficients, not magnitude alone.
6. Measure DC, RMS, peak, crest factor, duty cycle, slope symmetry, curvature,
   edge time, overshoot, and post-edge droop.
7. Estimate pitch-conditioned linear transfer from an ideal antialiased source
   and separately fit phase-domain geometry.
8. Retain residual cycles for stochastic analysis.

Fits use log2 frequency as the independent pitch coordinate. Start with
piecewise linear or low-order polynomial interpolation; only use higher-order
splines when held-out notes improve without oscillatory coefficients.

Pulse fits include pulse width as a second coordinate and keep rising/falling
edge parameters separate.

## Stochastic extraction

Separate:

- Static unit/voice offset.
- Slow common drift.
- Slow independent drift.
- Short-term period jitter/phase noise.
- Cycle-amplitude variation.
- Threshold/duty variation.
- Residual broadband noise.

Estimate these from cycle timestamps and residuals after subtracting the
deterministic phase-aligned waveform. Do not infer drift from a normalized
single cycle. Preserve correlations among pitch, amplitude, duty, and shape.

## Identification split

- Training: alternating notes/conditions.
- Validation: interleaved unseen notes and pulse widths.
- Test: a fixed withheld octave subset and all dynamic sweeps.
- Never tune against the final listening set.
- Report interpolation error separately from error on fitted points.

The compact model and neural branch must use the same split where the datasets
overlap.

## Verification

- Re-running extraction from identical source checksums reproduces coefficients.
- Phase alignment does not hide pitch error or cumulative drift.
- Hardware loopback analysis reveals whether observed low-frequency droop may
  belong to the interface.
- Reconstruction from stored complex harmonics matches the deterministic cycle.
- Fits remain finite and stable between measured notes and widths.
- Target normalization is reversible or fully documented.

## Completion criteria

- Monologue target is importable without manual file editing.
- Arturia target has a complete static and dynamic capture manifest.
- Derived cycles and features can be consumed by the compact, wavetable,
  gray-box, and neural plans.
- A report clearly separates oscillator-source evidence from output-chain
  coloration and stochastic behavior.

The Monologue-import portion is complete. Automated acquisition and Rust
extraction for Arturia and future physical targets are specified in
`plans/analog-osc/15-automated-synth-reference-capture.md`. For Prophet-5 V the
static three-wave capture uses oscillator 2, with oscillator 1 disabled.

## References

- Public Analog and Synthetic VCO dataset:
  <https://zenodo.org/records/15196138>
- Simionato and Fasciani paper and capture method:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Companion code and pretrained-model repository:
  <https://github.com/RiccardoVib/NeuralOSC>
- Pekonen et al., frequency-dependent analog saw identification:
  <https://link.springer.com/article/10.1155/2011/785103>
- Esqueda, Kuznetsov, and Parker, Differentiable White-Box Virtual Analog
  Modeling:
  <https://dafx.de/paper-archive/2021/proceedings/papers/DAFx20in21_paper_39.pdf>
- Arturia's public description of capacitor discharge, filtering, and
  instability:
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
- Sequential Prophet Rev2 reference behavior:
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
