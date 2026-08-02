# Nonlinear-Phase BLEP and LP-BLIT Exploration

**Order:** 13 · **Depends on:** plans 01–06; compare against plans 09–12 ·
**State:** `[ ]` planned.

## DSP background

Classic BLEP replaces an instantaneous digital step with a short bandlimited
correction. A linear-phase correction is symmetric in time; a causal analog
signal path is not. Nonlinear-phase BLEP changes the correction's timing and
ringing. BLIT instead constructs a bandlimited impulse train and integrates it
into a waveform; LP-BLIT gives direct control over source bandwidth. Both are
anti-aliasing families, so any claimed “character” must survive level matching
and comparison against the same legal harmonic bandwidth.

## Objective

Compare two literature-backed source-generation alternatives that go beyond
the current linear-phase table BLEP:

1. Nonlinear-phase, causal filter-derived BLEP/BLIT basis functions.
2. LP-BLIT using a Hammerich pulse with separately controlled cutoff and
   stop-band roll-off.

This experiment asks whether edge phase and intentional source bandwidth
improve perceived character. It does not claim to reproduce a named oscillator
without a target-conditioned stage.

## Branch A: nonlinear-phase correction basis

The renderer still starts with a naïve discontinuity, but schedules a fitted
causal correction at the fractional edge time:

```rust
if let Some(edge) = detect_edge(previous_phase, phase) {
    correction.add(edge.fraction, edge.jump, nonlinear_phase_kernel);
}
let output = naive_wave(phase) + correction.next_sample();
```

The correction buffer is bounded and must add overlapping events rather than
discarding an earlier edge.

Construct impulse/step correction responses from stable analog prototype
filters transformed into short parallel first/second-order digital sections.

Candidate prototypes:

- Butterworth for smooth magnitude and monotonic transition.
- Bessel-like response for controlled time behavior.
- A low-order asymmetric/minimum-phase fit to a measured target edge.

Requirements:

- Accept arbitrary sub-sample discontinuity offsets.
- Support positive/negative steps and rising/falling target-specific variants.
- Provide integrated forms for BLEP and, if practical, BLAMP.
- Define exact latency/causality and startup state.
- Bound tail length or recursive decay for deterministic rendering.
- Preserve stable behavior under pitch and sample-rate changes.

Compare the current linear-phase table correction, the paper's recursive
nonlinear-phase construction, and a minimum-phase transformed finite table.

## Branch B: LP-BLIT

Implement the Hammerich pulse formulation with:

- Fundamental frequency.
- Source cutoff.
- Stop-band roll-off parameter.
- Safe harmonic/alias constraint derived from sample rate.

Generate:

- Saw through one leaky integration.
- Pulse/rectangular from appropriately shifted pulse trains/integrated odd
  components.
- Triangle through a second integration or an equivalent stable formulation.

Use the paper's second-order leaky integrator/zero-DC guidance as the initial
reference. Explicitly measure low-frequency amplitude and droop introduced by
the integrator so it is not confused with a fitted analog output stage.

## Modulation and transitions

- Pitch changes must not leave stale recursive correction tails.
- Pulse-width changes must preserve both edge timing and DC.
- Audio-rate PWM and sync are separate stress cases.
- Parameter changes in cutoff/roll-off use bounded smoothing or state
  transformation; naïve coefficient jumps are not acceptable.
- Compare event-local scalar computation with SIMD-friendly fixed work.

## Isolation

- Implement both as research models/kernels selected only in the desktop
  registry.
- Keep SawMethod publicly limited to the existing BLEP/PolyBLEP values during
  exploration; do not overload it with unrelated models.
- Do not replace the accepted sparse PolyBLAMP triangle implementation.
- Leave existing firmware feature selection unchanged.

## Evaluation

For each waveform:

- Low-frequency time shape and edge causality.
- Pre-ringing, post-ringing, overshoot, and settling.
- Integrated and worst-component alias levels.
- Legal-harmonic magnitude and phase.
- Frequency and PWM sweeps.
- Sensitivity to sample rate and parameter transitions.
- CPU/state compared with current table BLEP and PolyBLEP.
- Listening comparison before and after the synth filter.

A third run may add the target-conditioned output filter from plan 09. This
tests whether the new basis provides value underneath a fitted model.

## Acceptance and stop rules

Retain a method when it offers a repeatable listening or target-metric advantage
over the current BLEP at a known cost. It may remain an optional character
model even if alias metrics differ.

Reject a method if its apparent analog character is only uncontrolled aliasing,
if recursive tails click under modulation, or if LP-BLIT source roll-off can be
reproduced more cheaply and accurately by the compact post-filter.

## Deliverables

- Nonlinear-phase correction kernel and design script.
- LP-BLIT model with saw, triangle, and pulse.
- Parameter-transition and modulation tests.
- Edge/time/spectral ablation report.
- Desktop and available-hardware cost report.
- Recommendation on whether either method belongs in later integrated models.
- Live desktop audition adapters for retained bounded variants.

## References

- Pekonen and Holters, Nonlinear-Phase Basis Functions in Quasi-Bandlimited
  Oscillator Algorithms:
  <https://dafx.de/paper-archive/2012/papers/dafx12_submission_15.pdf>
- Kraft and Zölzer, LP-BLIT:
  <https://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_59.pdf>
- Välimäki, Pekonen, and Nam, Perceptually Informed Synthesis of Bandlimited
  Classical Waveforms:
  <https://pubmed.ncbi.nlm.nih.gov/22280720/>
- Stilson and Smith, Alias-Free Digital Synthesis of Classic Analog Waveforms:
  <https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101?rgn=main%3Bview%3Dfulltext>
- Pekonen thesis:
  <https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/>
- Current BLEP/BLAMP implementation: synth-core/src/dsp/blep.rs and
  synth-core/src/dsp/analog_oscillator.rs
- Existing triangle performance decision:
  plans/TRIANGLE_WAVEFORM_OPTIMIZATION_PLAN.md
- Live audition contract:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
