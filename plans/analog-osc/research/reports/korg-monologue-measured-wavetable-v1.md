# Korg Monologue measured wavetable representation study v1

## Decision

Retain measured wavetable interpolation as the next runtime candidate. Build a
full measured-table prototype before implementing residual compression.

This is an offline representation study, not yet a Rust runtime or listening
result. It uses only the 36 training pitches per waveform as stored knots and
evaluates all 36 validation/test pitches without fitting them. Every candidate
removes harmonics at or above 0.45 times the sample rate before cycle
reconstruction.

## Variants

- **Nearest table:** select the closest training-pitch spectrum. This is a
  useful robust bound but would step during pitch sweeps.
- **Complex interpolation:** linearly interpolate real and imaginary harmonic
  components in log2 frequency.
- **Baseline plus residual:** interpolate the complex difference between the
  measured cycle and production oscillator, then add it to the production
  oscillator at the requested pitch.
- **Canonical interpolation:** remove one global cycle rotation relative to the
  production source from every training knot before complex interpolation.
  Relative harmonic phase remains intact.

The full measured and residual variants produce nearly identical errors. At
the current dense two-semitone training grid, the residual does not reduce the
number of stored complex coefficients and adds another runtime source. It is
therefore deferred until a later compression study demonstrates an actual
memory or transition advantage.

## Held-out results

Medians are phase-aligned shape NRMSE and harmonic-magnitude NRMSE; lower is
better. Wins are against the production table-BLEP/PolyBLAMP baseline out of 36
held-out pitches.

| Wave | Representation | Shape median | Shape wins | Magnitude median | Magnitude wins |
| --- | --- | ---: | ---: | ---: | ---: |
| Saw | Production baseline | 0.04677 | — | 0.01553 | — |
| Saw | Nearest measured | 0.01294 | 36/36 | 0.00666 | 36/36 |
| Saw | Direct complex interpolation | 0.01344 | 32/36 | 0.00786 | 30/36 |
| Saw | Canonical complex interpolation | **0.01049** | **36/36** | **0.00402** | **36/36** |
| Triangle | Production baseline | 0.05275 | — | 0.03160 | — |
| Triangle | Nearest measured | 0.00680 | 36/36 | 0.00228 | 36/36 |
| Triangle | Direct complex interpolation | **0.00345** | **36/36** | **0.00219** | **36/36** |
| Pulse | Production baseline | 0.09774 | — | 0.03235 | — |
| Pulse | Nearest measured | 0.02337 | 34/36 | 0.01283 | 28/36 |
| Pulse | Direct complex interpolation | **0.01710** | **35/36** | **0.01165** | **30/36** |

Triangle is the cleanest result. Saw needs global-phase canonicalization: direct
Cartesian interpolation has four high-pitch shape regressions, while the
canonical form wins all held-out cases. That operation removes only cycle
rotation and does not discard relative harmonic phase.

Pulse improves substantially in aggregate but is not yet uniformly robust. Its
worst absolute error is the 21.858 Hz validation capture, where candidate shape
NRMSE is 0.281 versus 0.653 for baseline. Direct interpolation loses to baseline
on shape only at 774.194 Hz. Magnitude improves in 30/36 cases. The runtime
prototype must retain per-case reporting rather than hiding these tails behind
the median.

The endpoint test at pitch index 71 holds the nearest training knot because no
upper knot exists. It remains in the report but must not be mistaken for an
interpolation result. A production bank would need a guard knot above the
playable range or a documented endpoint policy.

## Phase interpretation

The importer anchors cycles at the steepest upward midpoint crossing. That is a
valid measured landmark, but it need not equal the production oscillator's
phase-zero convention. Saw canonical shifts cluster at the equivalent `-0.5`
and `+0.5` wrap boundary; triangle shifts are a stable roughly quarter-cycle
offset. Large fixed-phase NRMSE after canonicalization therefore does not mean
the waveform shape is wrong. Runtime initial/reset phase and sustained-timbre
comparison remain separate requirements.

## Storage implications

Storing all 1,025 complex `f32` bins for 36 pitches and three waveforms would
occupy about 865 KiB before schema overhead. Storing 2,048-sample `f32` cycles
is essentially the same size. A complete precomputed mip pyramid would be
larger. Desktop can accept this for the first prototype; later work must measure
safe-mip sparsity, f16/Q15 error, cache behavior, and low-rank/residual coding.

## Compiled desktop runtime

The first external bank and scalar Rust research model are now implemented as
`korg-monologue-measured-wavetable-v1`. The tracked manifest fixes source
hashes, phase policy, table layout, pitch coordinates, per-table harmonic
limits, binary SHA-256, and FNV checksum. The generated little-endian float32
bank is ignored under `target/analog-osc/banks/` and must be regenerated from
the verified source data.

The bank contains one 2,048-sample table for each of 36 training pitches and
three waveforms: 221,184 samples or 864 KiB. Each table used as the lower end of
a pitch interval is truncated for the next training pitch, so both endpoints
remain below the 0.45 × 48 kHz harmonic guard throughout interpolation. This
pitch-conditioned layout replaced the generic octave-spaced mip pyramid after
the latter proved unnecessarily dull between its coarse cutoff levels.

The runtime performs two periodic linear table lookups and one log-pitch
crossfade per sample. Pitch interpolation is continuous by construction; no
mip switch occurs inside the supported range. Sample rates below 43.2 kHz are
rejected because this bank's fixed 21.6 kHz source guard would no longer be
alias-safe. Higher rates are safe but do not restore harmonics removed for the
48 kHz bank.

### Held-out compiled results

| Wave | Shape median | Shape wins | Magnitude median | Magnitude wins |
| --- | ---: | ---: | ---: | ---: |
| Saw | **0.00925** | **36/36** | 0.01837 | 15/36 |
| Triangle | **0.00337** | **36/36** | **0.00244** | **36/36** |
| Pulse | **0.01536** | **35/36** | **0.01869** | **28/36** |

Triangle translates almost exactly from offline reconstruction to Rust. Saw's
phase-aligned shape improves in every case, but the conservative interval-safe
cutoffs reduce harmonic-magnitude similarity enough that the median no longer
beats baseline. Pulse retains a large aggregate shape improvement with one
low-frequency validation tail. These are listening questions, not reasons to
relax the alias guard silently.

Static non-harmonic residual sweeps cover seven pitches per waveform at 48 and
96 kHz. There are zero material gate failures. At 48 kHz the candidate's median
residual is approximately -98.4 dBc for saw, -98.2 dBc for triangle, and -97.9
dBc for pulse. The extreme low-frequency 96 kHz comparisons can be tens of dB
worse than an exceptionally clean baseline while remaining below -89 dBc; the
machine-readable report preserves those cases.

### Continuous shape and PWM extension

The shared research/live adapter now supports the full shape control without
inventing unmeasured width tables:

- Saw and triangle use the production phase-shift morph semantics, but both
  reads come from the measured pitch-conditioned table.
- Saw-triangle crossfades independently pitch-interpolated measured saw and
  triangle tables at a shared phase.
- Pulse width is a difference of two phase-shifted measured saw reads. A
  measured-pulse-minus-generated-square residual is blended out with increasing
  width, making shape zero exactly equal to the measured 50-percent pulse while
  avoiding a false claim that other widths were captured.

Static shape sweeps cover five pitches, shapes 0/0.5/0.9, four waveforms, and
48/96 kHz: 120 cases with zero material residual failures. At 48 kHz the median
candidate residual is -98.0 dBc for saw, -98.2 dBc for saw-triangle, -98.2 dBc
for triangle, and -98.6 dBc for pulse. The 96 kHz medians remain between -97.9
and -103.0 dBc. See
`korg-monologue-measured-wavetable-shape-sweeps-v1.json`.

The candidate is available in the live Params selector. Slop now follows the
production oscillator's effective per-lane drift frequency on every sample
without resetting measured phase. Detune and glide use the same continuous
frequency-update path. A 96,000-sample shaped-triangle sweep from 20.7 Hz to
1.2 kHz crosses the measured pitch intervals with a maximum adjacent-sample
step below 0.20, and an end-to-end synth-engine comparison confirms that slop
changes the audible measured output.

Hard sync now uses the master wrap's fractional sample offset rather than a
phase-zero snap. The measured slave advances through the remainder of the
event sample, and a fixed four-sample Pirkle-table BLEP residual is scaled to
the actual waveform-level reset jump. Repeated offsets are bit-deterministic;
offsets at opposite ends of the sample remain observably distinct; and all
five measured shape paths stay finite and below an absolute output of 4.0 in
the integration stress case. The live synth path also produces observably
different Osc 1 output when sync is enabled. This is a compatibility result,
not a Monologue matching result, because the source dataset has no hard-sync
recordings. Out-of-range pitches continue to use production fallback.

### Compact dynamic gate

The combined-feature gate is complete. Its first run found and fixed a
wrong-sign PWM DC-compensation term that affected non-neutral pulse widths;
the measured 50-percent pulse and prior blind files were unchanged. After the
fix, deterministic 48 kHz renders have lower disagreement with filtered 192 kHz
renders than baseline for pitch/shape, audio-rate PWM, hard-sync ratios, and a
combined stream. Hard sync remains the worst measured case at 0.182 NRMSE.
The tested full-engine profiles remain comfortably real-time on this desktop,
with the combined one-voice profile using 7.46% of a 48 kHz frame at p99. The
candidate is frozen as a desktop real-time experiment; see
`korg-monologue-measured-wavetable-dynamic-v1.md` and its JSON.

## Blind listening gate — complete

The newly randomized nine-case package is generated at
`target/analog-osc/listening/korg-monologue-measured-wavetable-v1`. It uses the
lowest, middle, and highest eligible test row for each waveform after excluding
every pitch heard in either Plan 04 blind package. Test rows are odd-indexed and
were never used to construct the even-indexed measured tables.

ABX was 6/9 (`p = 0.25391`), which does not establish reliable overall
discrimination. Target matching selected the candidate in 6/9 cases, but all
three saw choices were explicitly reported as identical at 0.1 confidence.
Triangle favored the candidate 2/3; pulse favored baseline 2/3. See
`korg-monologue-measured-wavetable-listening-v1.md` and its decoded JSON.

The unchanged candidate advances to bounded runtime-cost and dense
pitch-transition stress as an exploratory desktop model, not a production
promotion. Do not tune tables or harmonic guards against these revealed
answers.

## Artifacts and reproduction

The machine-readable per-case result is
`korg-monologue-measured-wavetable-v1.json`. It records source hashes, split
policy, all variants, and endpoint cases.

```text
python3 plans/analog-osc/research/scripts/evaluate_measured_wavetable_models.py
python3 -m unittest plans/analog-osc/research/scripts/test_measured_wavetable_models.py
python3 plans/analog-osc/research/scripts/generate_measured_wavetable_bank.py
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/evaluate_measured_wavetable_runtime.py
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py --candidate-model korg-monologue-measured-wavetable-v1 --profile plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-sweeps-v1.json
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py --candidate-model korg-monologue-measured-wavetable-v1 --profile plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-shape-sweeps-v1.json --sample-rates 48000,96000 --frequencies-per-waveform 5 --shapes 0,0.5,0.9 --waveforms saw,saw-triangle,triangle,square
python3 plans/analog-osc/research/scripts/generate_measured_wavetable_listening_set.py
# Complete target/analog-osc/listening/korg-monologue-measured-wavetable-v1/responses-template.json first.
python3 plans/analog-osc/research/scripts/analyze_measured_wavetable_listening_set.py
```
