# Korg Monologue Phase + Filter Profile v1

## Decision — superseded by listening and ablation analysis

Retain v1 only as a reproducible negative research result; do not promote it.
Its original fixed-cycle time and complex-harmonic metrics improve, but blind
target matching failed and the later full ablation found that those metrics
over-reward an arbitrary capture trigger/cycle origin. The v2 objective must
align away global cycle phase and compare with the compiled production BLEP
baseline. See `korg-monologue-phase-filter-ablation-curves-v1.md`.

Profile content SHA-256:
`a962e0e4685dd866ab3f215a01cdf4604878bd546b0eb53ef8e5968a1dc3f37d`.

## How to read these results

We are comparing generated oscillator cycles with cycles recorded from a real
Korg Monologue. In this original report, the fitter's baseline is an ideal
geometric waveform rather than the compiled production BLEP oscillator, and
the cycles share a fixed capture-trigger origin: the metric does not optimize
away an arbitrary whole-wave time shift. The later ablation report corrects
both interpretation problems.

- **RMS (root mean square)** is a practical measure of signal strength. A
  waveform's ordinary average is usually near zero because its positive and
  negative halves cancel. RMS measures both halves and lets us express errors
  relative to the target's strength.
- **NRMSE (normalized root mean square error)** is the typical distance between
  the generated and recorded waveforms divided by the target RMS. Lower is
  better. `0` is an exact match, `0.05` is roughly a 5% error relative to the
  target signal strength, and `1.0` means the error is as large as the target
  itself.
- **Time-domain NRMSE** compares the visible waveform sample by sample. It
  catches curved ramps, pulse-plateau droop, misplaced corners, overshoot, and
  differences in edge shape.
- **Complex-harmonic NRMSE** compares the harmonics that make up the sound,
  including both their strength and timing/phase. Two signals can have similar
  spectrum magnitudes yet different waveform shapes, so magnitude alone is not
  sufficient.
- **Median** is the middle result across all tested notes: half are better and
  half are worse. It describes typical performance without allowing one bad
  capture to dominate the result. **Maximum** records the worst tested note so
  failures cannot be hidden by a good median.
- **Held-out** notes were not used to fit the model. Improvement on them is
  important evidence that the model learned behavior across pitch instead of
  merely memorizing its training recordings.
- **Wins**, such as `28 / 36`, count the held-out notes where the candidate has
  lower error than the baseline. This exposes how consistently an improvement
  is distributed.
- **Correlation** measures how closely two shapes move together. `1` means
  essentially the same shape, `0` means no useful relationship, and `-1` often
  means the same shape with inverted polarity. It is useful alongside NRMSE but
  does not replace it because it can overlook level differences.
- **Minimum phase derivative** checks that the fitted phase bending always
  moves time forward. A positive value means the waveform timing never folds
  backward; the enforced safety threshold for this model is `0.08`.
- **Alias metrics** measure unwanted digital frequencies that are not present
  in the intended oscillator. A target match is not acceptable if it adds
  metallic digital artifacts. Static residual evaluation is now complete; one
  borderline high-saw case remains.
- **Runtime cost, state, and asset size** measure CPU time, working memory, and
  stored coefficient data. They do not say whether the model sounds good, but
  they determine which hardware can run it.

The current held-out median time errors are 15.5% to 10.5% for saw, 7.2% to
0.9% for triangle, and 13.3% to 5.3% for pulse when moving from the baseline to
the candidate. The complex-harmonic errors improve by similar amounts. This is
evidence that analog character here involves within-cycle timing and
pitch-dependent filtering, not merely rounded discontinuities. It is not yet
proof of a better musical instrument: alias tests, smooth sweeps, and
level-matched listening remain necessary. Those later tests were completed and
did not support promotion; see the superseding decision above.

## Model

The fitted signal path is:

1. A waveform-specific, two-term periodic Fourier phase map.
2. Production table-BLEP for saw/pulse edges or PolyBLAMP for triangle corners.
3. One first-order low-pass.
4. One first-order high-pass.
5. One real pole-zero section.
6. Fitted gain and DC offset.

The fitter estimates 36 pitch knots per waveform from the training split. The
runtime interpolates phase coefficients, gain, and DC linearly in log2 pitch;
physical pole and zero frequencies are geometrically interpolated before their
digital coefficients are calculated. The audio loop performs no allocation,
file access, or iterative solve.

## Held-out results

All errors are normalized to the centered target RMS. `Wins` counts held-out
pitches whose full-model time error is lower than the ideal geometric baseline.

| Waveform | Baseline time median | Model time median | Baseline complex median | Model complex median | Wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| Saw | 0.1546 | 0.1045 | 0.1543 | 0.1037 | 28 / 36 |
| Triangle | 0.0717 | 0.0086 | 0.0717 | 0.0086 | 34 / 36 |
| Pulse | 0.1328 | 0.0534 | 0.1323 | 0.0533 | 35 / 36 |

Median relative time-error reductions are 42.1% for saw, 84.0% for triangle,
and 56.4% for pulse. Minimum interpolated phase derivatives on the held-out
sets are 0.511, 0.839, and 0.505 respectively, comfortably above the enforced
0.08 monotonicity margin.

The result is not uniformly better: eight held-out saw pitches, two triangle
pitches, and one pulse pitch regress. The worst errors occur around regions
where independently fitted parameters change quickly or reach bounds. Several
saw pole/zero/low-pass values reach the 60 kHz ceiling; the lowest pulse fit
reaches the 800 Hz low-pass and 0.2 gain bounds. Those are identifiability
warnings and a reason not to increase model order yet.

## Runtime prototype

A release desktop smoke render at 48 kHz, 220 Hz saw, 4,096 warm-up samples,
and 262,144 measured samples reported:

| Model | ns / processed sample | Mutable state | Immutable profile |
| --- | ---: | ---: | ---: |
| Production baseline adapter | 22.97 | 352 bytes | 0 bytes |
| Target-conditioned scalar | 44.05 | 96 bytes | 4,320 bytes |

This single run establishes bounded desktop-scale cost, not a hardware
benchmark. The candidate remains scalar and `real_time_safe = false`. A
`no_std` check with `wide-1`, the research feature, and Pass Through succeeds.

The compiled release renderer was also evaluated at all 36 held-out pitches per
waveform. Target cycles were sampled at the runtime oscillator's base phase;
no per-case delay was fitted. Runtime median time NRMSE was 0.1067 for saw,
0.0098 for triangle, and 0.0497 for pulse. The maximum absolute difference from
the mathematical predictor's per-case NRMSE was 0.0596, 0.0058, and 0.0666.
The machine-readable cases are in
`korg-monologue-phase-filter-runtime-v1.json`.

## Static pitch and residual sweeps

Seven log-spaced held-out pitches per waveform were rendered at 48 and 96 kHz.
A four-term Blackman-Harris FFT excludes guarded intended harmonics and reports
the remaining deterministic implementation residual relative to legal
harmonic energy. More-negative dBc values are better. This is an alias warning
metric for deterministic software, not a claim that every non-harmonic bin in
a hardware recording would be aliasing.

| Rate | Waveform | Baseline median | Candidate median | Candidate change |
| ---: | --- | ---: | ---: | ---: |
| 48 kHz | Saw | -85.23 dBc | -85.60 dBc | -2.53 dB |
| 48 kHz | Triangle | -97.20 dBc | -98.03 dBc | -3.03 dB |
| 48 kHz | Pulse | -84.82 dBc | -87.50 dBc | -3.77 dB |
| 96 kHz | Saw | -85.51 dBc | -87.75 dBc | -5.45 dB |
| 96 kHz | Triangle | -100.38 dBc | -102.52 dBc | +0.32 dB |
| 96 kHz | Pulse | -87.07 dBc | -93.20 dBc | -6.13 dB |

The candidate is typically equal or cleaner. Pulse passes every case. Triangle
has one nominal 3.98 dB regression at 96 kHz, but both results are below
-160 dBc and therefore numerical-floor territory. Saw has phase-sensitive
outliers because several extracted frequencies have exact integer sample
periods: the fitted phase offset and baseline then exercise different fixed
sub-sample edge positions. Replacing the first PolyBLEP implementation with a
table-BLEP linear-edge plus smooth-curvature decomposition removed the large
spikes without changing the fitted geometry. One top-range 48 kHz saw case
remains just over the warning boundary: -69.80 dBc versus -73.49 dBc at
2.4 kHz. That is the only material static-sweep failure under the current
`>3 dB worse and above -70 dBc` warning rule.

A dense two-second triangle pitch sweep from 20.7 Hz to 1.17 kHz also remains
finite and below the 0.20 adjacent-sample continuity threshold. Saw/pulse
dynamic edge-event and audio-rate PWM sweeps remain pending.

## Verification

- Python fitter tests: phase-map monotonicity, log-frequency interpolation,
  finite cycles, checked-in profile checksum, and generated-Rust identity.
- Rust research integration tests: deterministic output, finite output for all
  fitted waveforms, stable registry metadata, ablation parameter persistence,
  and rejection of the unfitted saw-triangle morph.
- Research CLI: profile identity and checksum are embedded in JSON artifacts;
  WAV rendering succeeds with the common harness.
- Compiled-runtime evaluator: all 108 held-out waveform/pitch cases render via
  the release Rust executable and are compared directly with the derived target
  cycles.
- Static residual evaluator: 42 baseline/candidate cases across 48 and 96 kHz,
  plus an analytic clean-harmonic and injected-spur test for the metric.

## Important limitations and next gate

- The initial profile covers pitch only. Pulse-width captures were not present,
  so only the measured 50% pulse is target-qualified.
- Saw-triangle morph and audio-rate PWM are unsupported.
- The top-range 48 kHz saw residual needs either a further edge-kernel treatment
  or an explicit acceptance decision after listening.
- Audio-rate PWM, saw/pulse dynamic event sweeps, and parameter trajectory plots
  remain. Level-matched listening and static objective ablations are complete.
- `phase-amount` and `filter-amount` are research-only ablation controls. They
  are not patch parameters and do not appear in the live Params selector.

Do not add a desktop audition adapter for v1. The next action is a v2 objective
using the compiled production baseline, level normalization, global-cycle phase
alignment, and a harmonic-magnitude term, followed by a training-only refit.

## Reproduction

```text
python3 plans/analog-osc/research/scripts/fit_target_conditioned_oscillator.py --max-nfev 240
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_oscillator.py
cargo test -p synth-core --features oscillator-research --test analog_osc_research
cargo run -p synth-tools --bin analog_osc_research -- --list
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_runtime.py --all-held-out
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_ablations.py
```
