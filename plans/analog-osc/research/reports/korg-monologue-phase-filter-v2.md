# Korg Monologue target-conditioned oscillator v2

## Decision

Retain v2 as a reproducible analysis result, but close it for promotion. Do not
expose it in the live Params selector or promote it to production.

The new objective fixes the two main v1 problems: it starts from the actual
table-BLEP/PolyBLAMP production oscillator, and it does not reward an arbitrary
whole-cycle phase offset. The compiled v2 runtime improves phase-aligned shape
over the production baseline in all 108 held-out waveform/pitch cases. Static
residual sweeps have no material alias/regression failures. Triangle is the
cleanest result. Saw and pulse still have a meaningful offline-predictor versus
runtime mismatch in harmonic magnitude, and pulse has no robust aggregate
magnitude improvement over baseline. The completed fresh blind gate produced
6/9 correct ABX answers (`p = 0.25391`) and selected v2 as closer in 4/9 cases
versus 5/9 for baseline. It therefore did not establish reliable
discrimination or a target-match advantage.

## What phase zero means

The synth continues to reset every waveform to phase `0.0`. This is a valid,
simple engine convention. It affects reset transients, note attacks, hard sync,
and the first sample, but it does not change the steady periodic timbre of a
free-running oscillator.

Recorded cycles can begin at any oscilloscope trigger point, so comparison v2
optimizes away one whole-cycle rotation as a nuisance. It keeps relative phase
between harmonics, which determines waveform shape, and stores no fitted global
phase offset. A Rust integration test verifies that setting both v2 character
amounts to zero reproduces the production saw, triangle, and pulse from phase
zero.

## Model and fitting change

- Source: the exact checked-in 4096-point table-BLEP saw/pulse implementation
  and the production PolyBLAMP triangle convention.
- Geometry: a monotonic two-term Fourier phase map anchored at phases 0 and 1.
- Color: first-order low-pass, first-order high-pass, and one pole-zero section.
- Objective: centered, unit-RMS, phase-aligned waveform shape plus harmonic-
  magnitude error with weight 0.65.
- Training: only even-index training pitches. Validation and test coefficients
  are interpolated in log-frequency and are never fitted directly.
- Level: gain is derived after the shape fit. The measured DC value is retained
  independently.
- Listening: no revealed v1 preference or ranking constrains any v2 coefficient.

The exact policy, knots, per-case results, data hashes, BLEP-table hash, and
profile checksum are in `../profiles/korg-monologue-phase-filter-v2.json`.

## Offline fit results

Medians below are NRMSE; lower is better. Counts are candidate wins over the
production baseline out of 36 held-out pitches per waveform.

| Wave | Shape baseline | Shape v2 | Shape wins | Magnitude baseline | Magnitude v2 | Magnitude wins |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Saw | 0.04677 | 0.01264 | 35/36 | 0.01553 | 0.00816 | 35/36 |
| Triangle | 0.05275 | 0.00473 | 36/36 | 0.03160 | 0.00333 | 36/36 |
| Pulse | 0.09774 | 0.03523 | 36/36 | 0.03235 | 0.03239 | 22/36 |

All 36 saw fits converged within the configured evaluation budget. Triangle
converged in 25/36 and pulse in 31/36; the remaining cases reached the budget
with finite, low-error solutions. Triangle gain spans 0.035–1.235 and pulse
gain 0.081–9.484, indicating parameter compensation and weak identifiability
at some pitches. More optimizer iterations alone should not be interpreted as
a remedy.

## Compiled Rust validation

The release renderer was evaluated at every validation and untouched test
pitch using averaged exact-period cycles. The metric removes only whole-cycle
rotation.

| Wave | Runtime shape median | Shape wins | Runtime magnitude median | Magnitude wins |
| --- | ---: | ---: | ---: | ---: |
| Saw | 0.00973 | 36/36 | 0.01991 | 16/36 |
| Triangle | 0.00464 | 36/36 | 0.00340 | 36/36 |
| Pulse | 0.03316 | 36/36 | 0.03583 | 18/36 |

Triangle closely follows the offline predictor. Saw and pulse retain their
shape advantage but do not reproduce the predictor's magnitude advantage. The
largest runtime/predictor magnitude-NRMSE deltas are 0.0390 for saw and 0.0404
for pulse. Possible causes include fitting an ideal periodic frequency-domain
filter response while running causal native-rate IIR state, and differences
between a dense phase grid and native sample/event placement. A future v3 fit
should optimize the actual runtime recurrence directly if listening shows this
branch is worth continuing.

The machine-readable runtime result is
`korg-monologue-phase-filter-runtime-v2.json`.

## Static residual gate

Seven log-spaced held-out pitches per waveform were checked at 48 and 96 kHz
using the deterministic non-harmonic residual metric. There are zero material
gate failures and zero cases more than 3 dB worse than baseline. Median
candidate-minus-baseline residual ranges from -0.34 to -3.89 dB at 48 kHz and
from -1.00 to -4.69 dB at 96 kHz, depending on waveform. Negative is better.

This is not proof of every dynamic condition, but it is sufficient for the
static listening gate. Full results are in
`korg-monologue-phase-filter-sweeps-v2.json`.

## Completed listening gate

A newly randomized, level-matched blind set used three fresh test pitches per
waveform and compared the measured target, production baseline, and compiled
v2. Baseline was judged closer for saw and pulse in 2/3 cases each. V2 was
judged closer for triangle in 2/3, but the three-case ABX result was also only
2/3 and included an incorrect high-confidence answer. That is an interesting
hypothesis, not an acceptance result.

Do not tune v2 after these revealed answers. Broad dynamic pitch/PWM sweeps and
live audition are stopped. Preserve triangle for later cross-candidate
comparison and move current implementation work to an independent topology.
See `korg-monologue-phase-filter-listening-v2.md` and its machine-readable JSON.

## Reproduction

```text
python3 plans/analog-osc/research/scripts/fit_target_conditioned_oscillator_v2.py
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_runtime_v2.py
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py --candidate-model target-conditioned-phase-filter-v2 --profile plans/analog-osc/research/profiles/korg-monologue-phase-filter-v2.json --output plans/analog-osc/research/reports/korg-monologue-phase-filter-sweeps-v2.json
python3 plans/analog-osc/research/scripts/analyze_target_conditioned_listening_set_v2.py
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_oscillator_v2.py plans/analog-osc/research/scripts/test_target_conditioned_sweeps.py
cargo test -p synth-core --features oscillator-research --test analog_osc_research
```
