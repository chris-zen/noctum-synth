# Target-Conditioned Phase and Filter Model

**Order:** 09 · **Depends on:** plans 01–07 · **State:** `[x]` v1 and v2
executed; **retained as evidence and closed for promotion**.

## DSP background

Phase warping changes how quickly the oscillator travels through different
parts of a cycle, bending an ideal ramp without changing its fundamental
period. A monotonic map is required so phase never runs backwards. Small IIR
sections then reproduce frequency-dependent brightness, low-frequency coupling,
and phase shift. Their coefficients vary with pitch by interpolation between
fitted knots. Bypass controls enable ablation: phase-only, filter-only, and
combined variants reveal which part actually helps.

## Execution status — v2 retained as evidence, closed for promotion

The first target-conditioned candidate is implemented as an analysis-only
research model. It uses a two-term monotonic Fourier phase map, the production
table-BLEP for saw/pulse edges, PolyBLAMP for triangle corners, and a first-order
low-pass, first-order high-pass, and pole-zero output section. Thirty-six pitch
knots per waveform are fitted from the Monologue training split and
interpolated in log-frequency.

Implemented deliverables:

- `scripts/fit_target_conditioned_oscillator.py`: deterministic bounded fitter,
  profile evaluator, JSON exporter, and Rust coefficient generator.
- `plans/analog-osc/research/profiles/korg-monologue-phase-filter-v1.json`:
  versioned fitted profile, per-case diagnostics, split metrics, source hashes,
  and content checksum.
- `synth-core/src/dsp/target_conditioned_oscillator.rs`: allocation-free scalar
  runtime with cached coefficients and phase/filter ablation controls.
- `synth-core/src/dsp/target_conditioned_profile.rs`: generated immutable Rust
  profile.
- `plans/analog-osc/research/reports/korg-monologue-phase-filter-v1.md`:
  current evidence, cost measurement, limitations, and next gate.

The held-out median time and complex-harmonic errors improve for saw, triangle,
and pulse, so the candidate is retained as a reproducible research result. The
compiled Rust runtime has also been checked against all 108 held-out
waveform/pitch cases. Static residual sweeps at 48 and 96 kHz pass for triangle
and pulse. Saw has one borderline 48 kHz case at the top of its fitted range.

The first blind session establishes an important negative result: all 9 ABX
trials were identified correctly, but the production baseline was judged
closer to the measured deterministic reference in 8 of 9 target-match cases.
The combined model therefore fails the perceptual promotion gate and remains
unavailable in the live Params selector. A phase-only/filter-only blind
diagnostic has been completed on six unused validation cases. Filtering ranks
poorly for saw but well for pulse, while triangle changes winner with pitch.
Full validation-pitch ablation curves reveal the deeper issue: fixed-phase
error rewards matching an arbitrary cycle origin, and the fitted comparison
baseline was geometric rather than the compiled production BLEP oscillator.
The v1 candidate is closed for promotion; the branch continues only through a
phase-invariant, production-baseline v2 objective and clean refit.
Saw-triangle morph and audio-rate PWM are explicitly unsupported in this
revision.

That v2 refit and compiled-runtime verification are now complete. It retains
the engine's phase-zero reset convention but removes whole-cycle rotation only
as a comparison nuisance. All 108 held-out compiled cases improve
phase-aligned shape over the production baseline, and the 48/96 kHz static
residual sweep has no material failures. Triangle also improves harmonic
magnitude in every held-out case. Saw and pulse expose runtime/predictor
magnitude mismatch, and pulse's aggregate magnitude result is approximately
flat versus baseline. The newly randomized blind test-split gate is complete:
6/9 ABX answers were correct (`p = 0.25391`), while target matching selected
baseline in 5/9 cases and v2 in 4/9. V2 therefore does not pass the perceptual
gate and will not advance to live audition, broad dynamic sweeps, or production.

The first reproducible listening package is generated under
`target/analog-osc/listening/korg-monologue-phase-filter-v1/`. It contains
named, blind ABX, and blind target-match files for nine held-out low/mid/high
cases. Technical validation and one complete subjective response set are
recorded in the listening report.

The completed follow-up diagnostic package is under
`target/analog-osc/listening/korg-monologue-phase-filter-ablation-v1/`. It asks
the listener to rank baseline, phase-only, filter-only, and combined variants
against the measured periodic reference for six previously unused validation
pitches. Test-split pitches were kept out of that diagnostic; three fresh test
pitches per waveform are now in the v2 acceptance package. The tracked v1
protocol and result are in
`plans/analog-osc/research/reports/korg-monologue-phase-filter-ablation-listening-v1.md`.

The 54-case objective analysis, listening-rank comparison, and plots are in
`plans/analog-osc/research/reports/korg-monologue-phase-filter-ablation-curves-v1.md`
and `plans/analog-osc/research/plots/korg-monologue-phase-filter-ablation-curves-v1.svg`.

The v2 fit policy, objective evidence, compiled parity, static residual gate,
limitations, and reproduction commands are in
`plans/analog-osc/research/reports/korg-monologue-phase-filter-v2.md`.
The completed v2 blind protocol, package validation, result, and decision are in
`plans/analog-osc/research/reports/korg-monologue-phase-filter-listening-v2.md`.

## Objective

Test the highest-return compact model from the literature: retain a proven
antialiased source, deform its phase/time geometry, and apply a
frequency-dependent low-order output filter fitted to recordings.

This experiment must determine how much analog target error can be removed
without a full circuit simulation or large sample bank.

## Hypothesis

Most deterministic single-cycle character relevant to the current comparison
can be represented by:

- Pitch- and pulse-width-dependent phase geometry.
- Separate rising/falling edge response.
- Low-order low-pass/high-pass/pole-zero coloration.
- Small asymmetric amplitude mapping.

The Arturia low-frequency pulse droop is expected to live mainly in the output
filter, not the BLEP correction. The model must therefore expose source and
output-stage plots separately.

## Model structure

Use this ordered pipeline:

1. Common phase/event generation.
2. Target-conditioned phase warp or piecewise geometric core.
3. Existing BLEP/BLAMP correction at the warped sub-sample event position.
4. Target-conditioned stable IIR output model.
5. Optional antialiased nonlinearity from plan 14.
6. Optional variation from plan 15.

All stages can be bypassed independently for ablation.

### Phase geometry

A compact two-term warp can be implemented conceptually as follows. The fitted
coefficients must be constrained so the derivative remains positive.

```rust
fn warp_phase(p: f32, a1: f32, a2: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    (p + a1 * (tau * p).sin() + a2 * (2.0 * tau * p).sin()).rem_euclid(1.0)
}
```

The actual normalization of `a1` and `a2` belongs in the fitted profile and
must match the offline predictor exactly.

Start with a monotonic periodic phase map controlled by a small Fourier or
piecewise polynomial basis. Requirements:

- Maps 0 to 0 and 1 to 1.
- Is monotonic under every interpolated parameter set.
- Preserves explicitly chosen reset/threshold landmarks.
- Supports distinct rising/falling pulse event adjustments.
- Coefficients vary smoothly with log2 frequency and pulse width.

If a general phase map cannot represent the target without excessive order,
use waveform-specific piecewise curves:

- Saw: ramp curvature and finite reset interval.
- Triangle: separate up/down curvature, slope ratio, and corner location.
- Pulse: threshold-derived duty plus separate edge timing; plateau droop stays
  in the filter stage.

### Linear coloration

Fit progressively:

1. One first-order IIR post-equalizer matching the Pekonen paper.
2. Two high-pass sections plus one pole/zero section.
3. At most three biquads if held-out error justifies them.

Poles must remain inside a conservative stability margin after interpolation.
Interpolate physical pole frequency, zero frequency, gain, and Q rather than
raw denominator coefficients. Convert to digital coefficients at the active
sample rate.

The fit objective combines complex harmonic error and phase-aligned time error.
Do not fit magnitude only, because edge placement and droop phase are audible
and visible.

### Nonlinearity

Keep disabled for the first deterministic fit. If residual harmonic structure
correlates with level, enable a low-parameter asymmetric saturator and use the
antialiasing strategies in plan 14. Do not let a nonlinear stage compensate for
an incorrect linear filter or phase map.

## Parameter fitting

For each waveform and target:

1. Fit measured pitch/width grid points independently with conservative bounds.
2. Inspect parameter trajectories for discontinuities or swapped solutions.
3. Fit low-order functions of log2 frequency and pulse width.
4. Validate at withheld notes and widths.
5. Reduce model order when two parameters are highly correlated or do not
   improve held-out complex-harmonic error.
6. Export a compact target profile with schema version, sample-rate-independent
   physical parameters, fit range, checksum, and provenance.

Fit saw, triangle, and pulse separately at first. A later shared-profile pass
may tie output coupling and gain parameters when the measurements support it.

## Desktop and hardware implementation

- Desktop profile loading may use immutable owned tables of fit knots.
- Runtime evaluation caches coefficients and only recomputes them when pitch,
  pulse width, target, or sample rate changes beyond a defined tolerance.
- Audio-rate PWM interpolates bounded physical parameters without redesigning
  the filter every sample; prepare a cheap coefficient trajectory or restrict
  which fitted parameters follow audio-rate width.
- A promotable implementation performs no allocation, file I/O, or unbounded
  iteration in render.
- Preserve the current SIMD phase engine where profitable; scalarize only the
  small per-lane fitting/interpolation work if desktop measurements justify it.

## Experiment matrix

- Targets: Monologue, Arturia Prophet-5 V, and an identity/ideal profile.
- Waveforms: saw, triangle, pulse.
- Static pitch and pulse-width cases from plan 03.
- Pitch sweeps and PWM sweeps.
- Sample rates: 44.1, 48, 96, 192 kHz.
- Ablations: phase only, IIR only, phase plus IIR, then optional nonlinearity.

## Acceptance and stop rules

Retain the model when:

- Held-out complex-harmonic and phase-aligned time errors both improve over the
  current BLEP baseline.
- Interpolation is click-free and stable across pitch and pulse-width sweeps.
- Alias metrics are not materially worse than the underlying BLEP source before
  optional nonlinearity.
- Listeners can distinguish it from baseline and prefer/match it to at least one
  named target in level-matched tests.
- Runtime cost is measured and bounded.

Stop increasing model order when held-out improvement is negligible, poles
approach instability, or parameters become non-identifiable. Archive a target
profile rather than generalizing it to other devices.

## Deliverables

- Standalone model and research-registry adapter.
- Reproducible fitter and versioned target profiles.
- Per-waveform ablation report.
- Pitch/width interpolation plots.
- Quality, CPU, state, and asset report.
- Listening set against baseline and target.
- Live desktop audition adapter through Pass Through and representative
  existing filters once render cost is bounded.

Initial-fit completion is not promotion. The v1 combined candidate failed its
first perceptual target-match gate, and the level-matched phase/filter
diagnostic is complete. Do not tune against either set of revealed choices.
The full validation-pitch objective ablation curves are also complete. The v2
objective and clean training-only refit now use the production BLEP baseline,
level normalization, global phase alignment, and a harmonic-magnitude term.
The fresh blind test-split target-match set is complete and v2 did not pass.
Do not tune against the revealed answers or proceed to broader dynamic sweeps
or a live desktop adapter. Preserve the artifacts and move to an independent
oscillator topology; triangle may return only in a later cross-candidate test.

## References

- Pekonen et al., Discrete-Time Modelling of the Moog Sawtooth Oscillator
  Waveform:
  <https://link.springer.com/article/10.1155/2011/785103>
- Pekonen thesis, especially phase-distortion and frequency-dependent
  post-processing models:
  <https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/>
- Arturia Prophet V manual, public description of capacitor discharge and
  low/high filtering:
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
- Target preparation: plans/analog-osc/06-reference-capture-and-identification.md
- Baseline oscillator and BLEP: synth-core/src/dsp/analog_oscillator.rs and
  synth-core/src/dsp/blep.rs
- Nonlinear extension: plans/analog-osc/14-antialiased-nonlinearity.md
- Live audition contract:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
