# Full Circuit WDF and State-Space Oscillator

**Order:** 17 · **Depends on:** measured evidence from plans 06–15 and a known
target schematic/topology · **State:** `[ ]` gated, high-cost reference branch.

## DSP background

A state-space model writes component voltages and currents as coupled equations.
A wave digital filter (WDF) reformulates a circuit in travelling-wave variables
that can improve numerical robustness. Both can reproduce interactions omitted
by a gray-box model, but discontinuous switching and nonlinear devices still
need careful solvers and antialiasing. This branch begins only if simpler models
leave a perceptually important, repeatable error that circuit structure can
plausibly explain.

## Objective

Evaluate component-level WDF/state-space modeling only when a defensible
reference circuit is available. Use a published relaxation oscillator first to
validate the modeling and event-antialiasing toolchain; do not invent a
Prophet Rev2 schematic.

## Entry criteria

Do not start a named-target circuit model until at least one exists:

- Public schematic with component values and device identities.
- Legal access to a measured unit plus enough circuit information for a
  gray/white-box model.
- SPICE/macromodel reference that can be validated against recordings.

The Sequential manual establishes behavior and architecture, not the internal
oscillator circuit needed for white-box reproduction.

## Phase A: published reference reproduction

Reproduce the relaxation oscillator from Olsen, Werner, and Germain:

- Resistor/capacitor network.
- Schmitt/comparator behavior.
- Fixed-step reference.
- Variable-step/network-variable-preserving method.
- Sub-sample discontinuity estimation and polyBLEP output correction.

Validate period, capacitor state, stored energy/passivity behavior, aliasing,
and convergence against a high-rate numerical/SPICE reference. This phase
proves infrastructure rather than target sound.

## Phase B: target circuit model

A state-space model has the conceptual form below, where `x` is internal
capacitor/current state, `u` is the control input, and `y` is audio output:

```text
dx/dt = f(x, u, fitted_components)
y     = g(x, u, fitted_components)
```

Switches and nonlinear devices make `f` piecewise or implicit. The discrete
solver must locate threshold events within the sample and remain stable across
all supported rates; simply applying forward Euler is not an acceptable final
model.

For an accepted circuit:

1. Derive modified nodal/state-space or WDF structure from the schematic.
2. Identify energy-storage state, switching surfaces, nonlinear devices, and
   observable outputs.
3. Choose trapezoidal, Möbius, or another passive/stable discretization.
4. Solve delay-free/nonlinear loops with bounded iteration or a validated
   analytic/quasi-analytic solution.
5. Handle threshold events at sub-sample time.
6. Apply explicit BLEP/BLAMP/ADAA where the physical numerical model alone is
   not bandlimited.
7. Compare against SPICE over static, sweep, and modulation conditions.
8. Fit uncertain components from recordings rather than arbitrary tweaking.

## Differentiable parameter identification

When component tolerances or inaccessible values dominate error:

- Build an offline differentiable version or differentiable surrogate.
- Optimize bounded physical component parameters against raw/feature losses.
- Use multiple notes, levels, and waveforms simultaneously.
- Validate fitted components on held-out dynamic cases.
- Reject solutions that violate plausible component ranges or trade one
  waveform against another.

The real-time model need not be differentiable after fitting.

## Numerical and antialiasing requirements

- State remains finite and passive/stable for the permitted parameter range.
- Pitch error is separated from waveform error.
- Switching surfaces use deterministic sub-sample localization.
- Step-size changes preserve physical network variables/energy.
- Nonlinear loop iterations have a hard maximum and a defined failure path.
- High-rate reference convergence is documented.
- Output discontinuities receive explicit correction; physical modeling does
  not automatically eliminate sampling aliases.

## Implementation tiers

### Desktop reference

- Double-precision option.
- High-rate and adaptive/local step reference.
- Diagnostic access to all states.
- Iterative solves allowed with bounded watchdog/reporting.

### Desktop real-time

- Single precision after validation.
- Precomputed topology matrices.
- Fixed bounded solver.
- No allocation or logging in render.

### Embedded candidate

Only after value is demonstrated:

- Reduced state/topology.
- Analytic or very small fixed-iteration solver.
- Local event oversampling rather than whole-cycle oversampling.
- Explicit cycle and state-byte budget.

## Isolation

- Implement as a Tier 2 research model.
- Keep circuit topology, components, and fitted profiles separate from the
  baseline oscillator.
- No circuit assets or solver code enter firmware by default.
- Circuit diagnostics are exposed only to Osc Design/research tools.

## Evaluation

- SPICE/high-rate state and output error.
- Target complex-harmonic/time error.
- Pitch accuracy across sample rates.
- Stability under sweeps, PWM, sync, and extreme valid parameters.
- Alias energy before/after correction.
- Iteration/event count distributions.
- Desktop real-time factor, CPU, state, and code size.
- Comparison with the gray-box and compact target-conditioned models.

## Acceptance and stop rules

Retain the full circuit model only if topology-specific behavior measurably or
audibly exceeds the compact/gray-box alternatives. Keep it as an offline
reference if real-time cost is high.

Stop if the schematic is unavailable, fitted components are unidentifiable,
or circuit complexity does not improve held-out target matching.

## Deliverables

- Published relaxation-oscillator reproduction.
- Reusable WDF/state-space validation harness.
- SPICE/high-rate comparison suite.
- Optional named-target circuit model with provenance.
- Cost/quality comparison and placement decision.
- Live desktop audition adapter only after the solver has bounded real-time
  work; otherwise retain an offline reference renderer.

## References

- Olsen, Werner, and Germain, Network Variable Preserving Step-size Control in
  Wave Digital Filters:
  <https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf>
- Werner, Virtual Analog Modeling of Audio Circuitry Using Wave Digital
  Filters:
  <https://pure.qub.ac.uk/en/publications/virtual-analog-modeling-of-audio-circuitry-using-wave-digital-fil/>
- D'Angelo, Virtual Analog Modeling of Nonlinear Musical Circuits:
  <https://research.aalto.fi/en/publications/virtual-analog-modeling-of-nonlinear-musical-circuits/>
- Esqueda, Kuznetsov, and Parker, Differentiable White-Box Virtual Analog
  Modeling:
  <https://dafx.de/paper-archive/2021/proceedings/papers/DAFx20in21_paper_39.pdf>
- Holters, ADAA for stateful systems:
  <https://www.dafx.de/paper-archive/2019/DAFx2019_paper_4.pdf>
- Sequential Prophet Rev2 User's Guide:
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
- Gray-box precursor: plans/analog-osc/12-coherent-gray-box-core.md
- Live audition eligibility:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
