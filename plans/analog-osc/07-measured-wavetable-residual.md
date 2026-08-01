# Measured Wavetable and Residual Model

## Execution status — offline representation selected

The first training-only representation study is complete for the Monologue
dataset. Nearest measured tables, direct complex interpolation, production plus
interpolated residual, and global-phase-canonicalized variants were evaluated
on every validation and test pitch with a 0.45-Nyquist harmonic guard.

Measured representations materially improve held-out deterministic target
metrics for all three waveforms. Canonical complex interpolation is the robust
saw choice; direct complex interpolation is selected for triangle and the
initial fixed-width pulse prototype. Full measured and baseline-plus-residual
forms are nearly numerically identical, so residual complexity is deferred
until it demonstrates real compression or transition value. A versioned
864-KiB external bank and scalar desktop research runtime are now implemented.
Compiled held-out shape improves in 36/36 saw, 36/36 triangle, and 35/36 pulse
cases; static 48/96 kHz residual sweeps have no material failures. Saw harmonic
magnitude exposes the cost of conservative interval-safe cutoffs, so the model
advances to blind listening rather than live selection. The nine-case blind
gate is complete: ABX was 6/9 and target matching selected the candidate in 6/9
cases, but saw was explicitly indistinguishable and the meaningful
triangle-plus-pulse choices split 3/3. The unchanged candidate advances to
bounded cost and transition stress as an experiment, not as a production
promotion. See
`plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.md`.

Given the remaining research budget, exhaustive characterization was deferred
in favor of the higher-value musical test. A minimal desktop live adapter is
now available from the existing oscillator header dropdown as **Measured
Wavetable (Monologue)**. The app loads and checksum-validates the external bank
at startup and hides the option when the bank is absent. Both oscillators share
the selection. Continuous shape is now implemented: saw and triangle use
phase-shifted measured-table morphing, saw-triangle interpolates the two
measured sources, and PWM uses a bandlimited difference of measured saws plus a
fading residual that is exactly the measured pulse at 50 percent. Unsupported
out-of-range pitches fall back safely to production.

Slop, detune, and glide continuity are now implemented in the live adapter.
The measured oscillators follow the production oscillator's effective
per-lane drift frequency without phase resets; detune and glide already enter
through that same continuous frequency setter. A dense shaped-triangle sweep
from 20.7 Hz to 1.2 kHz stays on the measured path and has no material sample
step, and a live synth-engine comparison verifies that enabling slop changes
the audible measured output.

Hard sync is now implemented for every measured waveform and shape path. The
adapter retains the master oscillator's fractional wrap time, resets the slave
to its correct end-of-sample phase, measures the actual before/after waveform
jump, and applies an amplitude-scaled four-sample post-edge correction from the
existing Pirkle BLEP table. The correction state is fixed-size, additive for
overlapping events, allocation-free, and bypassed on production-fallback
lanes. Integration tests cover live routing, deterministic offsets 0 through
1, and bounded output for saw, saw-triangle, triangle, and pulse at neutral and
narrow widths. This provides playable compatibility; no target-specific sync
claim is made because the Monologue dataset has no sync captures.

The compact combined-feature characterization is complete. It covers dynamic
pitch/shape, 110 Hz PWM, fractional sync ratios, a combined event stream, and
full-engine Pass Through timing. The first run found and fixed the PWM
shifted-saw DC-compensation sign; shape zero and the prior blind set were
unchanged. Native 48 kHz output now agrees more closely with a filtered 192 kHz
render than baseline in all four cases. Hard sync is the worst measured case at
0.182 NRMSE and remains the principal limitation. Full-engine p99 cost on this
desktop consumes at most 7.46% of one 48 kHz frame in the combined one-voice
stress profile. Freeze this implementation as a desktop real-time experimental
candidate and move cross-candidate work forward; maximum-polyphony soak and
embedded qualification remain deferred. See
`plans/analog-osc/research/reports/korg-monologue-measured-wavetable-dynamic-v1.md`.

## Objective

Build a data-driven oscillator that preserves measured waveform magnitude and
phase across pitch while retaining predictable bandlimiting and low runtime
cost. Compare full measured mipmaps with a hybrid ideal-wave-plus-residual
representation.

## Variants

### A. Full measured mipmapped tables

- Derive one deterministic phase-aligned cycle per measured waveform/pitch.
- Resample into periodic tables while preserving the measured landmark phase.
- Bandlimit each runtime mip offline.
- Interpolate between neighboring pitch-conditioned source tables and between
  adjacent safe mips.
- Store saw and triangle directly.
- For pulse, begin with measured 50-percent square; add sparse pulse-width
  planes only after the static model is validated.

### B. Ideal source plus measured residual

- Render the current antialiased ideal source at the target condition.
- Compute a periodic residual against the deterministic measured cycle.
- Store a lower-bandwidth or low-rank residual basis.
- Render baseline plus interpolated residual.

This variant may preserve target detail with smaller tables and make the
underlying anti-alias guarantee easier to understand.

### C. Complex harmonic template

- Store compact complex harmonic coefficients per pitch/width knot.
- Reconstruct tables offline at load/boot time or render additively on desktop.
- Use this as an analysis reference even if direct additive runtime is too
  expensive.

## Table preparation

For every source cycle:

1. Remove undocumented normalization, DC, and phase operations from the import
   path; any intentional removal is part of the target profile.
2. Enforce periodic boundary consistency without hiding a real reset feature.
3. Transform to complex harmonics.
4. Remove components above a conservative runtime Nyquist guard.
5. Reconstruct each mip with a documented window/transition policy.
6. Preserve target-dependent harmonic phase.
7. Validate reconstruction against the original deterministic cycle.

Do not generate low mips by naïve time-domain decimation of a sharp table.

## Pitch and pulse-width interpolation

- Use log2 pitch coordinates.
- Interpolate complex spectra or aligned time-domain cycles only after checking
  phase consistency.
- Crossfade adjacent mips without temporarily selecting an unsafe richer table.
- Pulse-width options are evaluated in this order:
  1. Sparse measured two-dimensional width/pitch grid.
  2. Threshold-derived pulse from a measured ramp plus measured edge residual.
  3. Difference of shifted measured bandlimited steps/saws.
- Reject interpolation that creates transient DC, amplitude dips, or spectral
  bursts during sweeps.

## Memory representations

Measure, do not assume:

- Direct f32.
- f16 if platform conversion is cheap and error is acceptable.
- Q15 plus per-table scale.
- Delta/residual coding with fixed bounded decode.
- Low-rank basis plus per-pitch coefficients.

The current prototype's Q15 Gibbs clipping and compiled-f32 results are prior
evidence, not final conclusions. Desktop may use much larger banks. Hardware
placement is decided after quality.

Immutable table data uses a validated bank handle. Runtime rendering never
reads blocking QSPI/storage and never generates tables.

## Dynamic character

Static measured tables intentionally represent the deterministic mean cycle.
Cycle-to-cycle drift and variation are added through plan 09. Optional
level-dependent residual planes may be investigated only if reference captures
show repeatable nonlinear level dependence.

## Isolation

- Extend or parallel the existing WavetableBank prototype without changing its
  current behavior.
- Give measured banks their own schema and stable target/profile ID.
- Load heavy desktop banks only in the research registry and Osc Design.
- Do not add measured assets to firmware images by default.
- Keep generated/downloaded source audio outside Git.

## Evaluation

- Deterministic target error at measured and held-out pitches.
- Complex harmonic magnitude/phase error.
- Aliasing at every MIDI note and exact-bin stress frequencies.
- Pitch sweep and mip-transition artifacts.
- Pulse-width sweep behavior.
- Hard sync and reset handling.
- Bank bytes, working-set/cache behavior, state, and render cost.
- Listening against baseline, target-conditioned compact model, and gray-box
  model.

## Acceptance and stop rules

Retain the full-table or residual variant when it materially improves target
match over baseline without unsafe mip transitions. Select representation from
the quality/memory/CPU Pareto frontier, not one global hardware assumption.

Stop adding pitch/width planes when held-out improvement is smaller than table
and cache cost. Do not use tables to imitate stochastic variation.

## Deliverables

- Deterministic table/residual generator.
- Versioned measured-bank schema and validator.
- Full-table, residual, and complex-harmonic reference renderers.
- Interpolation/mip stress report.
- Per-target memory and CPU report.
- Listening samples and promotion recommendation.
- Live desktop audition adapter using prevalidated immutable banks.

## References

- Existing implementation: synth-core/src/dsp/wavetable.rs
- Existing quality/listening tools:
  synth-core/examples/sample_rate_quality.rs and
  synth-core/examples/wavetable_listening_samples.rs
- Prior prototype plan/report:
  plans/DAISY_WAVETABLE_PROTOTYPE_PLAN.md and
  plans/DAISY_WAVETABLE_PROTOTYPE_REPORT.md
- Simionato and Fasciani, measured frequency-dependent VCO waveforms:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Public Monologue dataset:
  <https://zenodo.org/records/15196138>
- NeuralOSC companion code:
  <https://github.com/RiccardoVib/NeuralOSC>
- Pekonen et al., measured phase/time and spectral oscillator matching:
  <https://link.springer.com/article/10.1155/2011/785103>
- Reference preparation: plans/analog-osc/03-reference-capture-and-identification.md
- Live audition contract:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
