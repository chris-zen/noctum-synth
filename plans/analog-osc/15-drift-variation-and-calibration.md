# Drift, Variation, and Calibration Model

**Order:** 15 · **Depends on:** plans 06 and a retained deterministic parent ·
**State:** `[ ]` planned.

## DSP background

Deterministic character is the repeatable average cycle. Variation describes
differences between voices, units, and moments in time. Very slow correlated
movement sounds different from independent white noise; pitch, pulse width,
level, and shape may also move together. The model therefore separates fixed
per-unit offsets, fixed per-voice offsets, common slow drift, independent slow
drift, and short-term noise, all with deterministic seeded test modes.

## Objective

Replace undifferentiated analog randomness with a measured, controllable model
of static voice variation, common drift, independent drift, period jitter,
amplitude variation, and waveform-parameter variation.

The deterministic oscillator must remain testable with this layer disabled.

## Current baseline

AnalogOscillator currently owns oscillator slop consisting of per-note/static
detune and a slow random walk. This is musically useful and must remain the
production compatibility behavior until a later promotion decision.

This research plan does not reinterpret the existing Rev2 Osc Slop control or
change factory patches.

## Variation hierarchy

Represent variation through a small set of latent sources rather than
independent random modulation of every parameter:

### Static per unit/profile

- Overall calibration and nominal target coefficients.

### Static per voice/lane

- Pitch offset.
- Charge-current ratio or phase-warp offset.
- Pulse threshold/duty offset.
- Output gain and DC bias.
- Output-filter pole/zero tolerance.
- Nonlinearity threshold/asymmetry.

These values are seeded on model construction or explicit calibration, not on
every note.

### Slow common-mode process

Represents temperature/supply behavior shared across voices. Use one or two
bounded Ornstein-Uhlenbeck/low-pass stochastic components with measured time
constants.

### Slow independent per-voice process

A bounded low-rate random process can be expressed as a leaky update:

```rust
drift += rate * (target_from_seeded_rng - drift);
effective_cents = fixed_voice_offset + common_drift + drift;
```

This is illustrative, not a license to choose arbitrary noise. Update rate,
distribution, bounds, and correlations must come from long captures. The same
seed must reproduce the same test render.

Smaller bounded components representing local device variation.

### Short-term process

- Period jitter/phase-noise component.
- Cycle amplitude or threshold noise.
- Residual broadband noise only when measured.

Keep short-term timing noise very small for DCO-like targets.

## Correlation model

One latent component can affect several physical/model parameters:

- Current scale changes pitch and ramp slope.
- Comparator offset changes pulse width and edge time.
- Temperature changes common pitch, output bias, and perhaps filter poles.
- Voice calibration affects static pitch and waveform coefficients.

Store a compact loading matrix per target profile. Start diagonal/simple and
add correlations only when supported by reference covariance.

## Measurement and fitting

From long captures:

1. Estimate cycle boundary times and amplitude/shape descriptors.
2. Remove nominal pitch trajectory and deterministic mean cycle.
3. Separate between-condition/static offset from within-recording drift.
4. Estimate power spectra/autocorrelation of period, amplitude, and duty
   residuals.
5. Fit a small number of time constants and process variances.
6. Estimate cross-correlation among residual descriptors.
7. Validate synthesized long-run statistics on withheld recordings.

Multiple hardware units or voices are required to identify true unit/voice
distributions. A single Arturia instance cannot establish hardware tolerances.

## DCO versus VCO profiles

Keep target classes explicit:

- VCO profile: potentially larger slow pitch drift and temperature dependence.
- DCO profile: stable period timing; character may reside more in waveform,
  threshold, gain, and downstream analog variation.
- Software profile: reproduce only measured variation; do not add generic
  analog noise by default.

The Rev2 manual's Osc Slop is a user-controlled emulation amount. Keep that
musical control separate from unavoidable modeled unit variation.

## Runtime behavior

- Fixed seed produces reproducible renders.
- Provide zero, nominal, and exaggerated research amounts.
- Update slow processes at a reduced control rate with smooth interpolation.
- Period jitter perturbs phase increment without discontinuous phase jumps.
- Static voice parameters persist across notes; explicit recalibration/reseed
  changes them.
- Common and independent RNG streams are separate and deterministic.
- Model state remains bounded and survives sample-rate changes through
  time-constant-preserving coefficient updates.

## Integration

Implement as a wrapper/parameter source usable by plans 09, 10, and 12.
The layer emits parameter perturbations in the receiving model's physical or
fitted units. It must not assume all models expose identical internals.

For baseline comparison, retain an adapter that reproduces current slop
semantics exactly.

## Evaluation

- Long-run period, cents, amplitude, duty, and shape distributions.
- Autocorrelation/power spectrum across milliseconds to minutes.
- Common versus differential voice motion.
- Chord beating and stereo/voice consistency.
- Note retrigger behavior and repeatability.
- CPU/RNG cost at one and four lanes.
- Listening at realistic and exaggerated amounts.

Do not score stochastic renders with raw sample RMSE. Compare distributions,
correlations, and controlled listening.

## Acceptance and stop rules

Retain only measured components whose removal worsens statistical match or
listening realism. Prefer fewer interpretable processes over many random LFOs.

Stop when the available dataset cannot distinguish static variance from drift,
or common from per-voice variation. Mark those parameters unidentifiable and
wait for multi-unit recordings.

## Deliverables

- Versioned variation profile schema.
- Deterministic latent-process generator.
- Baseline-compatible slop adapter.
- Long-run analysis and comparison report.
- Target-specific VCO/DCO recommendations.
- Live audition controls for zero, nominal, and exaggerated fixed-seed
  variation through raw and filtered paths.

## References

- Current slop state: synth-core/src/dsp/analog_oscillator.rs
- Existing Rev2 behavior plan: plans/REV2_OSCILLATOR_PARITY_PLAN.md
- Sequential Prophet Rev2 User's Guide:
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
- Arturia Prophet V manual, public oscillator-instability description:
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
- Simionato and Fasciani analog VCO capture:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Public long-duration pitch/waveform recordings:
  <https://zenodo.org/records/15196138>
- Reference extraction:
  plans/analog-osc/06-reference-capture-and-identification.md
- Live audition contract:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
