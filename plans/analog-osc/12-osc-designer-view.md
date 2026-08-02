# Oscillator Lab (Evolved Osc Design View)

## Objective

Evolve the existing Osc Design tab into a safe Oscillator Lab for choosing,
tuning, overlaying, and exporting research oscillator models and references.
Ordinary view changes must not alter the live synth engine or production patch
schema. The view has no load-to-synth action; live engine selection belongs to
the single global dropdown in the Params Oscillators header described in plan
14.

## Current state

synth-app/src/ui/analysis/osc_design.rs currently provides:

- A right-aligned selector for every live-capable oscillator model already
  exposed by the Params Oscillators dropdown.
- Saw, Saw+Tri, triangle, and pulse selection.
- Selection through the complete-engine descriptors enabled in the normal
  desktop research build; legacy method controls remain in single-engine
  builds.
- Shape, MIDI note, sample rate, cycles, and live rendering.
- Waveform and FFT displays.
- WAV export.
- Serializable OscDesignViewConfig, including a backward-compatible selected
  analysis model.

The view constructs an independent preview instance through the same
complete-engine descriptor/dispatcher as live audition. It enumerates engines
compiled into the current build; a wavetable preview obtains its generated
static banks through `WavetableEngine`, not application startup injection.
The view never sends engine-control messages. Therefore changing this selector
changes only the displayed waveform and spectrum; the Params dropdown remains
the sole way to choose what is played.

This minimal selector/rendering increment was intentionally pulled forward.
Model B, target overlays, diagnostics, metrics, and export remain later Plan 12
work and should not block implementation of the next oscillator family.

## Layout

Organize the view into four compact regions.

### Source bar

- Model A selector.
- Optional Model B selector and enable toggle.
- Optional reference target/recording selector.
- Waveform, note/frequency, sample rate, cycles, phase/reset, seed.
- Render, live, cancel/background status, and export.

Group models by:

- Baseline: table BLEP, PolyBLEP.
- Spectral: nonlinear-phase, LP-BLIT, measured wavetable/residual.
- Fitted: target-conditioned phase/filter.
- Stateful: gray-box, WDF.
- Reference: neural/offline render.

Unsupported capabilities disable relevant controls with an explanation.
Offline-only and unstable models remain useful here even though they cannot
appear in the playable engine selector. A model becomes playable only after it
declares bounded live support and supplies a separately tested live adapter.

### Common controls

Keep current waveform/note/rate controls. Add:

- Pulse width displayed as percent while preserving the synth shape mapping.
- Free-run versus deterministic reset.
- Static versus sweep case.
- Stochastic layer off/on and seed.
- Level normalization mode: none, peak, RMS, fundamental.

The default remains the current table-BLEP baseline and existing control values.

### Model-specific inspector

Render controls from typed model metadata:

- Group, label, units, range, default, logarithmic/linear scale.
- Read-only fitted values where appropriate.
- Reset model defaults and load named target profile.
- Bypass individual layers for ablation.
- Show estimated latency, warm-up, state bytes, asset bytes, and capability
  badges.

Do not put experimental controls in the normal synth Parameters view.

### Analysis area

Support synchronized overlays:

- Time waveform for Model A, Model B, and target.
- Difference/residual waveform.
- Spectrum and complex-harmonic magnitude/phase error.
- Alias/residual spectrum with legal harmonics separated.
- Optional model diagnostics such as capacitor voltage, comparator state,
  correction tail, selected mip, and IIR response.

Provide solo, overlay, and difference display modes with stable colors and
level-normalization status always visible.

## Reference handling

- Select references by target manifest and case, not arbitrary unlabeled WAV.
- Display provenance, source rate, requested/measured pitch, normalization, and
  processing flags.
- Phase-align only when the selected metric requires it; preserve an unaligned
  view for pitch/latency analysis.
- Permit drag/drop WAV as an untrusted temporary reference, clearly marked and
  excluded from reproducible reports until a manifest is created.

## Rendering architecture

- Lightweight real-time-safe models may render synchronously with the current
  live throttle.
- Heavy WDF/neural/table-generation work renders on a cancellable background
  job using immutable request snapshots.
- Publish a result only if its request generation still matches current UI
  state.
- Never generate tables, fit models, load files, or train networks on the UI
  thread.
- Cache results by model revision, parameters, case, target checksum, and
  metric revision.
- Limit retained samples/results so repeated exploration has bounded memory.

## Relationship to playable audition

Oscillator Lab and the playable synth may share model IDs, capability metadata, and
target-profile definitions, but they do not share mutable oscillator state or
parameter snapshots. Oscillator Lab is for visual and numerical exploration. The
Params view is for playing: its one global engine selector applies to Osc 1 and
Osc 2, and the existing filter selector can choose Pass Through for the raw
source. Selecting an engine never modifies the loaded patch.

## Configuration compatibility

Extend OscDesignViewConfig with serde defaults and a schema version:

- Preserve current waveform, saw method, shape, note, sample rate, FFT, and
  zoom settings.
- Add Model A/B IDs and versioned model-specific configuration.
- Add selected target/case, normalization, seed, overlay mode, and visible
  diagnostics.
- If a saved model is missing, fall back to baseline and show a warning.
- Never rewrite normal synth patches when saving analysis state.

Keep SawMethodConfig for loading existing settings even after the broader model
selector exists.

## Export

One export action writes a self-contained research result:

- Model and target manifests.
- Complete parameters and seed.
- WAVs for A/B/reference at original analysis level.
- Metric JSON/CSV.
- Optional PNG/SVG plots.
- Commit/dirty status and timestamp.

Keep the existing quick Save WAV action, but include model ID and a collision-
safe case identifier in the filename.

## Implementation sequence

1. **Complete:** adapt current BLEP/PolyBLEP rendering to the same closed model
   dispatcher used by live audition, with independent preview state.
2. **Minimal increment complete:** add the right-aligned Model A selector using
   shared playable-model IDs/names and availability. Capability badges and
   broader offline-only registry models remain pending.
3. Add Model B overlay/difference.
4. Add manifest-backed reference overlay.
5. Add model-specific inspector and ablation controls.
6. Add common metrics and result export.
7. Add background jobs and diagnostic traces for heavy/stateful models.
8. Perform a UI usability pass with low-frequency and high-frequency cases.

## Verification

- Existing config loads to the baseline model with identical controls.
- Baseline render remains numerically identical.
- Switching/resetting analysis models sends no live control messages.
- No Oscillator Lab action sends live engine-control messages.
- Stale background results never replace newer requests.
- Missing models/targets degrade safely.
- Model A/B/reference are aligned according to the selected metric and
  normalization is disclosed.
- Heavy renders do not block UI input or audio.
- User changes currently present in analysis/oscilloscope files are preserved.

## Completion criteria

- Every registered model is selectable without a model-specific branch in the
  main view.
- Every live-capable registered model can independently appear in the single
  Params-view engine selector and be heard through Pass Through or an existing
  filter.
- Two models and one target can be compared from the same deterministic case.
- The view exposes enough diagnostics to tune fitted and stateful models.
- A comparison can be exported and reproduced from its manifest.

## References

- Current Osc Design implementation:
  synth-app/src/ui/analysis/osc_design.rs
- Analysis module/config:
  synth-app/src/ui/analysis/mod.rs and
  synth-app/src/ui/analysis/config.rs
- Shared spectrum renderer:
  synth-app/src/ui/analysis/spectrum.rs
- Current oscillator API:
  synth-core/src/dsp/analog_oscillator.rs
- Research registry:
  plans/analog-osc/01-isolated-experiment-framework.md and
  plans/analog-osc/02-replaceable-model-architecture.md
- Target manifests:
  plans/analog-osc/03-reference-capture-and-identification.md
- Common evaluation:
  plans/analog-osc/13-evaluation-and-hardware-selection.md
- Playable desktop audition and raw filter:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
