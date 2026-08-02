# Coherent Gray-Box Oscillator Core

**Order:** 12 · **Depends on:** plans 01–10; plan 11 is recommended for the
wavetable comparison · **State:** `[ ]` planned.

## DSP background

An analog relaxation oscillator charges or discharges an energy-storage
element, such as a capacitor, until a comparator reaches a threshold and resets
or reverses the motion. Saw, triangle, and pulse can therefore be different
observations of shared state rather than unrelated ideal formulas. A gray-box
model preserves that topology but fits effective currents, thresholds, leakage,
and reset behavior from measurements instead of simulating every component.

## Objective

Explore a compact stateful oscillator whose saw, triangle, and pulse arise from
shared capacitor, current, comparator, threshold, and reset behavior instead of
three unrelated ideal formulae.

This is a topology-inspired model, not a claim to reproduce the Prophet Rev2
without its circuit and recordings.

## Hypothesis

A shared dynamical state produces convincing relationships that independent
waveform generators miss:

- Ramp curvature changes pulse threshold-crossing time.
- Charge-current asymmetry affects triangle symmetry and pitch.
- Comparator threshold and hysteresis affect duty and edge timing.
- Reset duration/overshoot affects saw and any waveform derived from it.
- Component variation changes several observable properties coherently.

## Topology variants

Implement two explicit gray-box families rather than one ambiguous universal
core:

### Saw-core family

- A current source charges a capacitor.
- A reset event discharges or restores the capacitor.
- Saw is a scaled/buffered capacitor voltage.
- Pulse is a comparator applied to the ramp.
- Triangle is a target-specific waveshaper or fold of the ramp.

This is the first choice for targets whose saw is visibly the primary core.

### Triangle-core family

- A Schmitt comparator controls positive/negative capacitor current.
- Triangle is the capacitor voltage.
- Square/pulse is the comparator output or a second threshold.
- Saw is derived by reset, rectification, or a target-specific waveshaper.

Use only when target evidence supports this topology.

The target manifest records which topology was fitted. Do not morph between
families at runtime.

## Continuous-time gray-box state

The smallest saw-core sketch integrates a charging current and resets when a
threshold is crossed:

```rust
capacitor_v += current(capacitor_v, controls) * dt / capacitance;
if capacitor_v >= high_threshold {
    capacitor_v = reset_voltage;
    emit_fractional_reset_event();
}
let saw = output_scale * capacitor_v + output_offset;
```

Production code must solve the fractional crossing time and bandlimit the
resulting jump. Triangle topology replaces the reset with a direction change;
pulse is derived from the same threshold/comparator state.

Use a small physically interpretable state:

- Capacitor voltage.
- Current direction/comparator state.
- Optional reset progress.
- Optional output-buffer/filter states.

The capacitor derivative can include:

- Positive and negative charge currents.
- Small voltage-dependent current term for ramp curvature.
- Leakage toward a rail or bias.
- Temperature/voice scale from plan 15.

Events occur when voltage crosses a threshold or reaches reset condition.
Locate every event at a sub-sample time using interpolation or a bounded
root solve. Update the state for the pre-event and post-event portions of the
sample so pitch does not depend on integer sample crossings.

Start with closed-form constant/affine-current updates. Add a numerical solver
only if measured residuals justify it.

## Antialiasing strategy

Physical modeling and bandlimiting remain separate concerns:

- Apply BLEP to comparator or reset amplitude jumps at the measured sub-sample
  event offset.
- Apply BLAMP to derivative discontinuities at triangle corners or finite reset
  boundaries.
- Compare a small local 2x/4x event-region integration pass against direct
  sub-sample updates.
- Never oversample an entire low-frequency cycle only to resolve one event.
- Validate sync and audio-rate modulation separately because they introduce
  new discontinuities.

The existing table BLEP is the first correction kernel. Nonlinear-phase
corrections from plan 13 can be evaluated later without changing the state
model.

## Waveform behavior

### Saw

Expose current curvature, reset target, reset duration, reset feedthrough,
output scale/bias, and optional output coupling. The ideal instantaneous reset
is a valid zero-setting used for ablation.

### Triangle

Expose positive/negative current ratio, upper/lower threshold, voltage
dependence, corner bandwidth, scale, and bias. Normalize pitch independently
from amplitude so fitting asymmetry cannot detune the requested note.

### Pulse

Generate pulse from threshold crossings of the same state. Pulse width changes
threshold or relative threshold position. Rising/falling thresholds and
hysteresis may differ when measured. Edge bandwidth and AC-coupled droop are
output-stage parameters, not substitutions for correct event timing.

## Fitting strategy

1. Select a topology from target evidence.
2. Fit low-frequency cycles first, where physical shape is visible and
   bandlimiting least ambiguous.
3. Fit threshold/current/reset parameters jointly across waveforms when the
   target provides coherent outputs.
4. Fit output filtering after core dynamics.
5. Fit pitch dependence and interpolate physical parameters in log2 frequency.
6. Validate on withheld notes, pulse widths, and dynamic sweeps.
7. Compare against the compact phase/filter model to determine whether shared
   state adds measurable or audible value.

Do not fit stochastic drift until deterministic state and event timing pass.

## Implementation and isolation

- Implement as a Tier 2 stateful research model from plan 02.
- Keep one model state per SIMD lane; begin with scalar lane code for clarity.
- Provide diagnostic traces for capacitor voltage, threshold, comparator state,
  raw discontinuity, and corrected output.
- Use a fixed maximum event count per sample. Reject pathological parameters
  that exceed it.
- Desktop is the first target. A later embedded form must have fixed state,
  bounded iterations, and no allocation.
- Do not route this model into production presets during exploration.

## Verification

- Requested pitch remains accurate across sample rates and initial phases.
- Event timing converges toward a high-rate continuous-time reference.
- State is finite and bounded for all allowed controls.
- Saw, triangle, and pulse remain phase coherent.
- BLEP/BLAMP reduces aliasing without changing the low-frequency physical shape.
- PWM and pitch sweeps are continuous.
- Reset and sync events report correct sub-sample offsets.
- Parameter variation affects correlated observables as designed.

## Acceptance and stop rules

Retain a topology when it beats the baseline and compact phase/filter model on
held-out target metrics or listening preference while remaining numerically
stable. Keep it desktop-only if valuable but too costly.

Stop adding circuit detail when a parameter cannot be identified from
recordings, when the model becomes target-independent guesswork, or when a
lower-cost fitted model matches equally well.

## Deliverables

- Saw-core and, only if justified, triangle-core prototypes.
- High-rate numerical reference and event-timing tests.
- Fitted target profiles with physical parameter units.
- Diagnostic plots and ablations.
- Quality/cost comparison with plan 09.
- Live desktop audition adapter after event count and solver work are bounded.

## References

- Olsen, Werner, and Germain, WDF relaxation oscillator with variable step
  size and polyBLEP:
  <https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf>
- D'Angelo, Virtual Analog Modeling of Nonlinear Musical Circuits:
  <https://research.aalto.fi/en/publications/virtual-analog-modeling-of-nonlinear-musical-circuits/>
- Werner, Virtual Analog Modeling of Audio Circuitry Using Wave Digital
  Filters:
  <https://pure.qub.ac.uk/en/publications/virtual-analog-modeling-of-audio-circuitry-using-wave-digital-fil/>
- Pekonen et al., measured analog waveform fitting:
  <https://link.springer.com/article/10.1155/2011/785103>
- Sequential Rev2 architecture and oscillator behavior:
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
- Existing sync/event metadata: synth-core/src/dsp/analog_oscillator.rs
- Reference preparation: plans/analog-osc/06-reference-capture-and-identification.md
- Live audition contract:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
