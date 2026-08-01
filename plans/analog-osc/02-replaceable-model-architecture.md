# Replaceable Oscillator Model Architecture

## Objective

Prepare a safe architecture for comparing and playing multiple oscillator
approaches while preserving the existing production path. Support both:

- Phase-evaluated kernels such as BLEP, nonlinear-phase BLEP, LP-BLIT, and
  wavetables.
- Fully stateful models such as capacitor/comparator cores, WDF circuits, and
  autoregressive neural models.

Do not force every experiment into the current phase-sampling trait, and do not
replace the production engine with dynamic dispatch.

## Implementation status

The minimum two-tier seam is implemented. Existing phase kernels keep their
typed production path. The feature-gated `OscillatorResearchModel` interface
accepts semantic cases/events and lets stateful models own their complete
history. `RegisteredResearchModel` adapts existing phase models without
changing production voice code. Stable live models derive their IDs, names,
revisions, and capabilities from `ExperimentalOscillatorModel`; analysis-only
models cannot enter the Params selector merely by appearing in the research
registry.

The integration test's stateful probe demonstrates independent phase/history,
reset, hard-sync event handling, and deterministic rendering. A real gray-box
candidate will be the first persistent stateful registry member. Oscillator Lab
wiring remains pending.

## Current state and limitation

The private OscillatorKernel trait in
synth-core/src/dsp/analog_oscillator.rs already allows a typed kernel to provide
saw, pulse, triangle, per-sample preparation, and transition behavior.
EngineOscillator is selected statically by a Cargo feature, while
RuntimeOscillatorKernel chooses table BLEP or PolyBLEP for analysis.

This is a good platform-specialization seam, but it assumes that
AnalogOscillator owns the authoritative phase and most waveform state. A
gray-box or neural model may own its own integrator, threshold state, latency,
history, and event timing. Expanding the current trait until it accommodates
every possibility would make the stable oscillator harder to reason about.

## Chosen architecture: two tiers

### Tier 1: retain typed phase kernels

Keep the existing AnalogOscillator generic path for methods that can naturally
use its phase, sync, shape, and correction lifecycle. Evolve the private trait
only when at least two concrete phase kernels need the same addition.

Candidate Tier 1 models:

- Existing table BLEP and PolyBLEP.
- Nonlinear-phase BLEP/BLIT.
- LP-BLIT if expressed as a phase/event kernel.
- Ideal and measured wavetables.
- Target-conditioned phase warp plus post-filter, if the filter state is owned
  by a small wrapper kernel/model.

Production continues to use a compile-time EngineOscillatorKernel alias.

### Tier 2: desktop research model adapter

Introduce an OscillatorResearchModel interface at the desktop boundary. It is
used by offline analysis and, for models declaring bounded real-time support,
by the feature-gated desktop audition source. It owns all synthesis state and
returns:

- Output sample or block.
- Wrap mask and sub-sample wrap offset when meaningful.
- Optional diagnostics such as capacitor voltage, comparator output, phase,
  active mip, filter state, and model latency.

Inputs are semantic controls and events, not internal implementation details:

- Waveform and shape/pulse width.
- Per-lane frequency.
- Note-on/reset mask.
- Hard-sync mask and sub-sample event offset.
- Enabled mask.
- Fixed seed and model-specific parameter block.

Tier 2 initially uses a closed runtime enum in the desktop registry. Enum
dispatch is acceptable in analysis; heavy models already dominate its cost.
Avoid trait objects in synth-core's production loop.

Stateful candidates can remain Tier 2 indefinitely. A winner intended for
hardware is later adapted to a statically selected production type in a
separate promotion change.

### Phase-0 playable facade

Before implementing the experimental models, execute only the minimal playable
subset described in plan 14:

- Under an experimental-oscillators desktop feature, replace each internal
  EngineOscillator field in Oscillators with a closed source enum whose default
  Baseline variant wraps EngineOscillator.
- With that feature disabled, retain the existing direct EngineOscillator
  fields and calls so production firmware pays no dispatch or storage cost.
- Share model IDs, capabilities, parameters, and immutable profiles with the
  offline registry.
- Keep selection in application research state rather than patches.
- Switch at an audio-block boundary after all-notes-off; do not translate live
  state among unrelated models.
- Use the real oscillator mixer, filter or Pass Through model, VCA, pan,
  effects, and output path for listening.

This is deliberately narrower than the full multi-engine plan. It provides
playability without committing to patch persistence, engine-specific public
parameters, voice-lifetime migration, or firmware deployment.

## Common controls versus model parameters

Common controls must preserve synth semantics:

- Waveform: Saw, SawTri, Triangle, Pulse.
- Shape: existing normalized synth control.
- Frequency and sample rate.
- Phase reset/free run and sync events.
- Slop amount only when evaluating compatibility behavior.

Research parameters belong to namespaced model configurations, for example:

- Target profile and phase-warp strength.
- Output-coupling pole frequencies.
- LP-BLIT cutoff and roll-off.
- Capacitor leakage, current asymmetry, reset time, threshold, and hysteresis.
- Wavetable target, interpolation, and residual amount.
- Nonlinear drive and ADAA order.

Do not add these to ParamId, MIDI, SysEx, factory presets, or the main parameter
view during exploration.

## State, assets, and real-time contract

Every model reports:

- Fixed mutable state bytes per lane/voice.
- Immutable shared asset bytes.
- Scratch bytes and whether they are construction-only.
- Algorithmic latency.
- Warm-up length.
- Whether its cost is bounded independently of pitch and parameters.
- Whether it is no_std compatible.

Immutable banks and fitted profiles use validated handles, following the
existing WavetableBank ownership pattern. Models never read blocking storage or
perform file I/O during rendering.

Heavy desktop assets may use Arc-owned immutable data in the app adapter.
Promotable core kernels use static slices or explicit caller-owned storage.

## Switching behavior

Oscillator Lab may switch models by constructing/resetting the newly selected
analysis model. Its parameters and waveform displays do not affect live audio.
The playable synth chooses one engine for both oscillators through the global
selector in the Params Oscillators header; there is no load-from-Oscillator-Lab
operation.

Desktop audition switching follows plan 14: all-notes-off, block-boundary model
replacement, complete common-parameter reapplication, then new notes. Phase 0
does not morph sustaining notes.

If patch-owned runtime selection is later approved:

- Select the model at note start and keep it fixed for the note lifetime.
- Preconstruct all permitted model storage.
- Crossfade or let old voices finish; never reinterpret unrelated state.
- Branch once per block where possible.
- Preserve old patches as baseline-model patches.

That broader work belongs with plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md and
is outside the Phase-0 research audition architecture.

## Implementation sequence

1. Freeze baseline regression vectors.
2. Extract the desktop semantic control/event types without moving production
   phase or slop behavior.
3. Add adapters for current AnalogOscillator and WavetableOscillator.
4. Add the closed desktop model registry and capability metadata.
5. Implement the Baseline-only playable facade and prove full-voice bit
   identity, then add Pass Through according to plan 14.
6. Reuse model IDs and metadata in Oscillator Lab and the offline harness while
   keeping their instances and parameters independent from the playable synth.
7. Add one trivial stateful test model to prove reset, sync-event, latency, and
   diagnostics plumbing.
8. Confirm non-research builds still use the same typed alias and feature
   behavior.
9. Add new methods only through independent experiment branches.

## Verification

- Baseline output remains bit-identical after adapter introduction.
- A stateful model can own phase/history without modifying AnalogOscillator.
- Unsupported waveform or event capabilities are visible, not silently
  approximated.
- Model-specific configuration round-trips with defaults by revision.
- No experiment identifier enters production patches.
- Oscillator Lab edits cannot control live audio. Only the global Params-view
  engine selector replaces desktop research sources, for both oscillators.
- Baseline selected through the audition facade is bit-identical through the
  complete voice path.
- A release firmware build omits all desktop registry code and experimental
  assets unless explicitly promoted.

## Completion criteria

- Phase-kernel and stateful-model experiments coexist in one analysis registry.
- Real-time-safe candidates can be played through the same registry and real
  synth signal path.
- Existing compile-time platform specialization remains available.
- Production defaults, patch compatibility, and factory behavior are
  unchanged.
- Adding a new research model requires only its implementation, adapter,
  metadata, and tests.

## References

- Current typed kernel seam: synth-core/src/dsp/analog_oscillator.rs
- Existing immutable bank pattern: synth-core/src/dsp/wavetable.rs
- Existing engine-level direction: plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md
- Minimal playable research layer and raw filter:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
- Existing platform constraints: docs/src/hardware/daisy.md
- Existing core/no_std direction: plans/SYNTH_CORE_NO_STD_PLAN.md
- Pekonen thesis, covering multiple oscillator implementation families:
  <https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/>
- D'Angelo, Virtual Analog Modeling of Nonlinear Musical Circuits:
  <https://research.aalto.fi/en/publications/virtual-analog-modeling-of-nonlinear-musical-circuits/>
- Olsen, Werner, and Germain, stateful relaxation oscillator plus polyBLEP:
  <https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf>
