# Desktop Experimental Audition and Pass-Through Filter

## Implementation status

Started on 2026-07-26. The first playable infrastructure slice now provides:

- An exact, zero-latency `FilterType::PassThrough`, feature-gated out of fixed
  firmware builds and exposed by the existing desktop filter selectors.
- A temporary desktop-only closed oscillator source facade whose default
  variant wraps the unchanged production `EngineOscillator`.
- One shared experimental selector for production-baseline, table-BLEP, and
  PolyBLEP/PolyBLAMP sources.
- Block-boundary control messages that stop notes, reconstruct the selected
  source, and reapply common oscillator controls.
- A right-aligned oscillator-engine selector in the Oscillators header; raw
  audition uses Pass Through in the existing Low-Pass Filter header selector.
- Bit-identity regression coverage for the baseline facade and exactness/
  zero-tail coverage for Pass Through.

Still pending in this plan are deterministic event recording/export, fuller
golden vectors for dynamic oscillator cases and the complete voice path, and
live adapters for new analog-model candidates. Oscillator Lab remains independent;
there is deliberately no handoff from it into the playable synth.

## Objective

Make every viable experimental oscillator playable from MIDI through the real
synth voice path before judging or promoting it. Add a true pass-through filter
model so the same oscillator can be heard both raw at the filter boundary and
through every existing filter model.

This is Phase 0 of the broader multiple-engine architecture. It deliberately
does not add experimental choices to patches, MIDI/SysEx, factory data, or
production firmware defaults.

## Signal path

The playable comparison path is:

    one selected complete oscillator engine
      -> selected filter model or Pass Through
      -> existing VCA/envelope
      -> pan
      -> effects
      -> master output

Pass Through means the oscillator mix is unchanged at the filter output. It
does not bypass the VCA, pan, effects, output limiter, audio backend, or DAC.
The audition UI must show which downstream stages remain active. A documented
raw listening preset disables effects, uses a simple sustained VCA envelope,
centers pan, and leaves master processing unchanged.

## Current repository boundaries

- synth-core/src/voice/oscillators.rs owns both EngineOscillator instances,
  their common controls, sync, slop, glide, sub oscillator, noise, and mix.
- synth-core/src/voice/mod.rs sends the oscillator mix through Filter before
  VCA and pan.
- synth-core/src/dsp/filter/mod.rs already provides runtime FilterType
  selection while keeping that choice outside patch data.
- synth-app exposes the same filter selection in the Parameters and Filter
  Design views.
- The existing typed EngineOscillator and hardware feature selection remain the
  production baseline.

## Phase-0 oscillator source architecture

### Complete-engine boundary and feature isolation

Replace the temporary per-source facade with `OscillatorEngines`, one retained
owner per voice. It selects and renders a complete pre-filter source engine:
Osc 1, Osc 2, sub oscillator, noise source, source mix, timing/sync behavior,
and engine-private state. The voice receives its rendered source mix, then
continues through the unchanged filter, VCA, pan, effects, and output path.

The pattern follows `FilterType` and its private algorithm state in
`synth-core/src/dsp/filter/mod.rs`, with one intentional difference:
oscillator engines retain all enabled complete source sections so switching does
not discard their private state.

```rust
pub struct OscillatorEngines {
    selected: OscillatorEngineType,
    params: OscillatorEngineParams,
    #[cfg(feature = "osc-blep")]
    blep: BlepEngine,
    #[cfg(feature = "osc-wavetable")]
    wavetable: WavetableEngine,
}
```

Each `OscillatorEngineType` variant is feature-gated. `ALL` contains only the
variants compiled into this build. UI and application code enumerate `ALL` and
do not name variants or carry matching `#[cfg]` attributes. The
`OscillatorEngines` module is the sole owner of feature-gated fields and
dispatch. At least one engine feature is required.

A firmware build enables one engine feature and retains only that concrete
engine. A desktop all-engine build retains every enabled engine and makes a
closed match on selection. This is an accepted desktop cost, equivalent to the
existing multi-filter build, while avoiding feature-gate leakage through the
rest of the program.

### Selection and configuration

Add application/session state for one complete-engine ID. It is stored as a
stable string in desktop configuration, resolved against the build's enabled
`ALL` descriptors at startup, and falls back to the build default with a
visible warning when unavailable.

Engine selection and engine-specific settings are not serialized into Patch,
ParamId, MIDI, SysEx, or factory programs. A wavetable bank selection is owned
by `WavetableEngine`, not the app, renderer, UI state, or a public synth
parameter. Initial modulation applies only common controls shared by every
engine; engine-specific settings are not modulatable.

Compiled generated `static [f32]` data provides the initial wavetable banks.
`WavetableEngine` owns its validated registry, so there is no disk loading,
host asset injection, allocation, or table generation in the render path.

### Safe switching

For Phase 0:

1. Send all-notes-off.
2. Select the retained engine at a block boundary.
3. Synchronize current common parameters, sample rate, glide target, modulation
   settings, and free-run/note-reset policy.
4. Resume on the next played note.

Do not reconstruct or reset the selected engine. Its inactive state, including
phase, BLEP correction history, wavetable position, and noise state, remains
intact. A later optional short output fade may hide the control transition, but
cross-model sustaining-note morphing is out of scope.

### Capabilities

Every model declares support for saw, SawTri, triangle, pulse, shape, static
PW, audio-rate PWM, hard sync, note reset/free run, slop, and SIMD lanes.

- Unsupported controls are disabled or ignored deterministically with a visible
  audition warning.
- Never silently substitute another waveform or model.
- A model can still be auditioned on the supported subset.
- Formal comparison reports list unsupported features.

## Pass-through filter

### Public model

Add FilterType::PassThrough with display name Pass Through (Raw). It is a true
member of the existing filter-model selection system:

- Available in the main Filter selector.
- Available in Filter Design, where its response is a flat 0 dB reference.
- Selectable through the existing typed SetFilterType control path.
- Application/global selection only; not part of patches.
- Existing cutoff, resonance, poles, modulation, and oversampling controls
  remain stored but are ignored while selected.

### Exact behavior

For every finite input lane:

- Output is bit-identical to input.
- Gain is exactly 1.
- Latency is zero.
- No DC removal, saturation, smoothing, interpolation, oversampling, or dither.
- Reset/reset-lane and filter parameter setters have no audible state.
- Switching away creates/resets the newly selected filter exactly as the
  current runtime model switch does.

Place the bypass decision at the beginning of Filter processing, before cutoff,
resonance, key tracking, envelope, audio modulation, coefficient, or
oversampling work. This makes raw audition cheap and prevents ignored controls
from performing unnecessary DSP.

### Build isolation

Add a filter-pass-through feature:

- Included by desktop filter-all/research builds.
- Optional for hardware builds that need raw-output diagnosis.
- Omitted from the normal fixed-filter firmware so it introduces no
  per-sample branch or enum state there.

Keep the FilterType value stable/serializable across builds. is_implemented()
reports whether the feature exists. An unavailable saved application selection
falls back to the build's normal default filter and reports the fallback.

## Desktop audition controls

Use the existing compact controls:

- One right-aligned engine dropdown in the Params Oscillators header. It
  selects the same engine for Osc 1 and Osc 2.
- The right-aligned filter dropdown in the Low-Pass Filter header, including
  Pass Through for raw listening.

Do not add per-oscillator engine selectors, audition slots, load-from-Oscillator-Lab
buttons, or a separate Experimental Audition panel. Any future performance-safe
character macros require a separate UI decision.

## Repeatable musical comparison

Support recording the same event sequence through A and B:

- Capture timestamped note/control events or use a deterministic MIDI file.
- Reset RNG/model state from the snapshot seed.
- Render from silence with identical patch, tempo, sample rate, buffer size,
  VCA, pan, effects, and master settings.
- Export raw-pass-through and selected-filter versions.
- Preserve original level and additionally create labeled RMS/fundamental-
  matched listening copies.

Live playing remains essential, but recorded comparisons provide repeatability.

## Implementation order

1. Freeze baseline oscillator and full-voice regression vectors.
2. Add PassThrough and its exactness/feature-isolation tests.
3. Add the complete retained-engine owner with the BLEP engine only.
4. Prove that single-engine BLEP output is bit-identical to the current playable
   source section.
5. Add engine features, the feature-gated engine enum, session selection, and
   safe all-notes-off switching.
6. Register live-capable complete engines in the global Params-view selector;
   Oscillator Lab may reuse descriptors but has no control-path connection.
7. Add the wavetable engine with compiled banks and play it through PassThrough
   and an existing filter.
8. Add repeatable event recording/export without expanding the engine-selection
   UI.
9. Require every later oscillator plan to provide a live-audition adapter when
   it reaches a stable render milestone.

Do not begin the broader patch-owned multiple-engine architecture during this
phase.

## Verification

### Baseline preservation

- A BLEP-only build retains no other engine implementation or asset and remains
  bit-identical through oscillator, voice, and engine golden vectors.
- A desktop all-engine build exposes exactly its feature-enabled descriptors
  without feature gates in UI, renderer, or engine APIs.
- Existing patches load/save identically and always use their existing synth
  parameters.
- Firmware without a non-BLEP engine contains no such engine or asset.

### Pass-through filter

- Random finite WideF32 inputs return bit-identically under all filter control
  and modulation combinations.
- Impulse response is one followed by zeros; frequency response is flat 0 dB
  with zero phase/latency.
- Switching to/from PassThrough is safe and stored common controls are restored
  when returning to another model.
- The signal immediately before Filter equals its output in PassThrough tests.

### Audition

- One selection chooses the complete source engine for both Osc 1 and Osc 2,
  sub oscillator, and noise.
- MIDI note, velocity, pitch bend, glide, waveform, shape, PWM, sync, mix, sub,
  noise, slop, VCA, pan, effects, and master routing remain correct or are
  explicitly reported unsupported.
- Model changes happen only at block boundaries and allocate nothing in render.
- A/B event replays are deterministic for fixed seeds.
- Heavy models that cannot run live are marked offline-only and cannot be
  selected for live audition.

## Completion criteria

- The BLEP engine and at least one alternate complete engine can be played from
  MIDI through the complete desktop synth.
- Either can be heard through PassThrough and every implemented filter.
- Raw and filtered A/B recordings can be reproduced.
- No patch, MIDI/SysEx, factory, default firmware, or baseline-output behavior
  changes.

## References

- Current dual-oscillator owner: synth-core/src/voice/oscillators.rs
- Current voice signal path: synth-core/src/voice/mod.rs
- Current filter dispatch: synth-core/src/dsp/filter/mod.rs
- Desktop filter selectors:
  synth-app/src/ui/params_view.rs and
  synth-app/src/ui/analysis/filter_design.rs
- Current app-level filter control:
  synth-app/src/ui/app.rs and synth-core/src/engine.rs
- Research registry and model seam:
  plans/analog-osc/01-isolated-experiment-framework.md and
  plans/analog-osc/02-replaceable-model-architecture.md
- Oscillator Lab research UI: plans/analog-osc/12-osc-designer-view.md
- Common comparison protocol:
  plans/analog-osc/13-evaluation-and-hardware-selection.md
- Existing multi-filter architecture:
  plans/MULTI_MODEL_FILTER_EXPERIMENT_PLAN.md
