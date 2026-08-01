# Antialiased Oscillator Nonlinearity

## Objective

Determine whether subtle, measured nonlinear behavior at the oscillator output
adds useful target character, and compare antiderivative antialiasing with
local oversampling. Prevent an attractive saturator from disguising incorrect
source geometry or reintroducing objectionable aliasing.

## Scope

This plan covers oscillator-local or output-buffer nonlinearities only:

- Soft rail compression.
- Positive/negative asymmetry.
- Comparator-edge saturation where a memoryless approximation is valid.
- Small level-dependent harmonic changes.

Oscillator mixer, filter, VCA, and effects saturation are separate systems.
Do not duplicate their color inside the oscillator without measurements.

## Measurement prerequisite

Use level-sweep captures from plan 03. A nonlinear stage is justified only when:

- Harmonic ratios or time shape change repeatably with output/oscillator level.
- The behavior cannot be explained by a linear level-dependent capture path.
- The feature persists across multiple cycles and is above the noise floor.

Fit deterministic geometry and linear output filtering first. Analyze their
residual versus level before selecting a nonlinear function.

## Candidate functions

Start with explicit functions whose antiderivatives are stable and cheap:

- Asymmetric polynomial soft clip inside a bounded range.
- Tanh-like rational saturator.
- Arctangent-like saturator.
- Piecewise smooth rail compression with continuous first derivative.

Parameters are drive, positive/negative threshold, knee/curvature, output gain,
and optional bias. Keep the smallest function that improves held-out data.
Hard clipping is a stress/control case, not the intended default.

## Antialiasing variants

### Baseline

Apply the function directly at native rate to quantify newly generated aliasing.

### First- and second-order ADAA

Implement antiderivative antialiasing with:

- Numerically stable handling when adjacent input samples are nearly equal.
- The exact linear-through behavior required by the chosen formulation.
- Explicit half-sample/algorithmic delay accounting.
- Analytic antiderivatives or verified symbolic/offline-generated forms.

### Oversampled reference

Use a high-quality offline 8x or higher oversampled renderer as the reference.
Compare practical 2x and 4x oversampling with fixed half-band filters.

### Stateful extension

Only if nonlinearity is embedded in a feedback/stateful circuit, use the
stateful ADAA literature or keep it inside the high-rate circuit solver.
Do not apply a memoryless ADAA formula inside an arbitrary feedback loop.

## Placement experiments

For each parent oscillator model, compare:

- Before output filtering.
- After output filtering.
- Split: a small core asymmetry before filtering and buffer saturation after.

The target measurement decides placement. Record gain normalization at each
position so louder is not mistaken for better.

## Implementation and isolation

- Expose the nonlinear stage as a research wrapper that can process any model
  output.
- Disabled mode must be bit-transparent.
- Stages share one configuration schema but no global synth parameter IDs.
- Cache fixed coefficients/antiderivative parameters.
- Mark latency so overlays and complex-harmonic comparisons align correctly.
- A hardware-promotable variant has fixed work and no allocation.

## Evaluation

- Alias energy and worst folded component under static notes and sweeps.
- Harmonic/intermodulation response over input level.
- Error against high-rate reference.
- Error against measured target at fitted and held-out levels.
- DC shift and gain.
- CPU and state at native, 2x, 4x, ADAA1, and ADAA2.
- Listening with raw oscillator and through the synth filter.

Stress with high notes, pulse edges, PWM, hard sync, and two-oscillator
intermodulation where the stage is shared.

## Acceptance and stop rules

Retain a nonlinear stage only if it improves measured target behavior or
repeatable listening preference after level matching. ADAA or oversampling must
reduce its aliases enough that the result is not merely brighter through
folding.

Stop if the target is effectively linear at oscillator level, if the required
nonlinearity belongs in the mixer/filter, or if a high-order function
overfits one level.

## Deliverables

- Direct, ADAA1, ADAA2, 2x, and 4x comparison implementations.
- High-rate reference renderer.
- Stable antiderivative tests including near-equal samples.
- Target level-sweep fit and ablation report.
- Cost/quality recommendation per parent oscillator model.
- Live audition wrapper for every retained parent/stage combination whose cost
  is bounded.

## References

- Bilbao, Esqueda, Parker, and Välimäki, Antiderivative Antialiasing for
  Memoryless Nonlinearities:
  <https://research.aalto.fi/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/>
- Aalto open manuscript:
  <https://research.aalto.fi/files/27135145/ELEC_bilbao_et_al_antiderivative_antialiasing_IEEESPL.pdf>
- Holters, Antiderivative Antialiasing for Stateful Systems:
  <https://www.dafx.de/paper-archive/2019/DAFx2019_paper_4.pdf>
- Esqueda, Aliasing Reduction in Nonlinear Audio Signal Processing:
  <https://research.aalto.fi/en/publications/aliasing-reduction-in-nonlinear-audio-signal-processing/>
- Zheleznov and Bilbao, Interpolation Filters for Antiderivative Antialiasing:
  <https://dafx.de/paper-archive/2024/papers/DAFx24_paper_33.pdf>
- Target level captures:
  plans/analog-osc/03-reference-capture-and-identification.md
- Existing nonlinear/sample-rate characterization:
  synth-core/examples/sample_rate_quality.rs and
  plans/DAISY_SAMPLE_RATE_QUALITY_REPORT.md
- Live audition contract:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
