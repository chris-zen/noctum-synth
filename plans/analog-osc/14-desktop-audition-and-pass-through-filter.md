# Desktop Experimental Audition and Pass-Through Filter

## Implementation status

Started on 2026-07-26. The first playable infrastructure slice now provides:

- An exact, zero-latency `FilterType::PassThrough`, feature-gated out of fixed
  firmware builds and exposed by the existing desktop filter selectors.
- A desktop-only closed oscillator source facade whose default variant wraps
  the unchanged production `EngineOscillator`.
- One shared engine selector for production-baseline, table-BLEP, and
  PolyBLEP/PolyBLAMP sources, controlled outside patch/MIDI/SysEx state and
  applied consistently to Osc 1 and Osc 2.
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

    one selected engine used by Osc 1 and Osc 2
      + existing sub oscillator and noise
      -> existing oscillator mixer
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

### Feature isolation

Add a desktop research feature such as experimental-oscillators:

- Enabled by synth-app research builds.
- Disabled by normal synth-core defaults unless explicitly requested.
- Disabled by Daisy firmware and benchmarks unless a later qualification plan
  opts in.
- When disabled, Oscillators stores the existing EngineOscillator fields and
  compiles the current direct typed calls without runtime model dispatch.

Use a conditional internal source type:

- Production: EngineOscillator.
- Desktop research: a closed ExperimentalOscillatorSource enum whose Baseline
  variant wraps EngineOscillator and whose other variants wrap concrete
  research models.

The enum implements the semantic operations Oscillators already needs:

- Apply waveform, shape, enabled mask, frequency, slop, note trigger/reset.
- Render one sample and report wrap/sub-sample metadata for sync.
- Reset one lane.
- Report capabilities and model diagnostics.

Do not expose the enum as a general plugin ABI. New models are added
deliberately and remain feature-gated.

### Selection and configuration

Add application/session state for one research engine ID, applied to both Osc 1
and Osc 2. Future engines may own a target/profile ID and model-specific
settings, but Phase 0 exposes only the common engine choice.

Model selection is not serialized into Patch, ParamId, MIDI, SysEx, or factory
programs. It may be saved in a clearly separate desktop research preference;
loading or saving a synth patch never changes it.

The baseline model is the default and fallback. Missing/disabled models fall
back to baseline with a visible warning.

Model/config changes are delivered at an audio-block boundary through a typed
control message. Preload and validate large immutable assets outside the audio
thread. The render callback performs no allocation, file I/O, table generation,
or model fitting.

### Safe switching

For Phase 0:

1. Send all-notes-off.
2. Reconstruct the selected source state at a block boundary.
3. Reapply the current OscillatorsParams, sample rate, glide target, modulation
   settings, and free-run/note-reset policy.
4. Resume on the next played note.

Do not attempt to translate live phase/history among unrelated models.
A later optional short output fade may hide the control transition, but
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
3. Add the desktop feature-gated source facade with Baseline only.
4. Prove the baseline facade is bit-identical to the current playable engine.
5. Add session selection/control messages and safe all-notes-off switching.
6. Register live-capable models in the global Params-view selector; Oscillator Lab
   may reuse metadata but has no control-path connection.
7. Add one simple alternate source, initially existing PolyBLEP or wavetable,
   and play it through PassThrough and an existing filter.
8. Add repeatable event recording/export without expanding the engine-selection
   UI.
9. Require every later oscillator plan to provide a live-audition adapter when
   it reaches a stable render milestone.

Do not begin the broader patch-owned multiple-engine architecture during this
phase.

## Verification

### Baseline preservation

- With experimental-oscillators disabled, production source types and output
  remain unchanged.
- With it enabled and Baseline selected, oscillator, voice, and engine golden
  vectors are bit-identical.
- Existing patches load/save identically and always use their existing synth
  parameters.
- Firmware without the research features contains no experimental models,
  assets, or pass-through dispatch.

### Pass-through filter

- Random finite WideF32 inputs return bit-identically under all filter control
  and modulation combinations.
- Impulse response is one followed by zeros; frequency response is flat 0 dB
  with zero phase/latency.
- Switching to/from PassThrough is safe and stored common controls are restored
  when returning to another model.
- The signal immediately before Filter equals its output in PassThrough tests.

### Audition

- Osc 1 and Osc 2 can independently select supported models.
- MIDI note, velocity, pitch bend, glide, waveform, shape, PWM, sync, mix, sub,
  noise, slop, VCA, pan, effects, and master routing remain correct or are
  explicitly reported unsupported.
- Model changes happen only at block boundaries and allocate nothing in render.
- A/B event replays are deterministic for fixed seeds.
- Heavy models that cannot run live are marked offline-only and cannot be
  selected for live audition.

## Completion criteria

- The current oscillator and at least one alternate model can be played from
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
- Broader deferred architecture:
  plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md
- Existing multi-filter architecture:
  plans/MULTI_MODEL_FILTER_EXPERIMENT_PLAN.md
