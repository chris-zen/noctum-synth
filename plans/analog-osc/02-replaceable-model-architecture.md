# Replaceable Oscillator Model Architecture

## Objective

Prepare a safe architecture for comparing and playing multiple oscillator
approaches while preserving the existing production path. Support both:

- Phase-evaluated kernels such as BLEP, nonlinear-phase BLEP, LP-BLIT, and
  wavetables.
- Fully stateful models such as capacitor/comparator cores, WDF circuits, and
  autoregressive neural models.

Do not force every experiment into the current phase-sampling trait. Live
runtime selection is confined to one closed, feature-gated engine owner; it
does not leak dynamic dispatch or feature gates through the voice and app.

## Implementation status

The minimum two-tier seam is implemented. Existing phase kernels keep their
typed production path. The feature-gated `OscillatorResearchModel` interface
accepts semantic cases/events and lets stateful models own their complete
history. `RegisteredResearchModel` adapts existing phase models without
changing production voice code. Stable live selection now comes from
`OscillatorEngineType::ALL`, `SawMethod`, and `BankId`, while research metadata
remains independent. Analysis-only models cannot enter the Params selector
merely by appearing in the research registry. The temporary per-source facade
has been removed; live audio and Osc Design both use the complete retained
engine boundary below.

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

### Live oscillator engines

The live boundary is one retained `OscillatorEngines` owner per voice. It is a
complete pre-filter source section, not an adapter for an individual Osc 1 or
Osc 2 waveform. Each concrete engine owns its own Osc 1, Osc 2, sub oscillator,
noise source, mix, timing/sync behavior, and all private state. A BLEP engine
therefore produces the complete source section with BLEP techniques; a
wavetable engine produces the complete section with wavetable techniques.
Future physical-model and granular engines use the same boundary.

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

`OscillatorEngineType` variants are feature-gated. The UI and application do
not name variants or use `#[cfg]`; they enumerate the enabled descriptors:

```rust
pub enum OscillatorEngineType {
    #[cfg(feature = "osc-blep")]
    Blep,
    #[cfg(feature = "osc-wavetable")]
    Wavetable,
}

impl OscillatorEngineType {
    pub const ALL: &'static [(&'static str, Self)] = &[
        #[cfg(feature = "osc-blep")]
        ("blep", Self::Blep),
        #[cfg(feature = "osc-wavetable")]
        ("wavetable", Self::Wavetable),
    ];
}
```

The `OscillatorEngines` module owns every feature-gated field and match. At
least one `osc-*` engine feature is required. A single-engine build retains one
concrete engine and dispatches directly; a desktop all-engine build retains all
enabled engines and uses a closed match to render the selected one.

Selection remains application/session state, applies one engine to the complete
source section, and is never serialized into patches, MIDI, SysEx, or factory
programs. It is deliberately narrower than patch-owned engine selection:
playability does not yet commit to per-note engine identity, engine-specific
public parameters, or firmware deployment.

## Common controls versus model parameters

`OscillatorEngineParams` contains controls every complete engine understands:

- Osc 1/Osc 2 waveform, enable state, base shape, pitch/tuning, and level/mix.
- Sub oscillator and noise controls.
- Sample rate, reset/free-run, hard sync, glide, and slop.

Effective frequency and common modulation are supplied at render time, not
written into retained base parameters every sample:

```rust
pub struct OscillatorRenderContext {
    frequency_hz: WideF32,
    shape_modulation: f32,
    reset_mask: u8,
    hard_sync: Option<SyncEvent>,
}
```

Initial modulation support is limited to controls common to every engine.

Research parameters belong to namespaced model configurations, for example:

- Target profile and phase-warp strength.
- Output-coupling pole frequencies.
- LP-BLIT cutoff and roll-off.
- Capacitor leakage, current asymmetry, reset time, threshold, and hysteresis.
- Wavetable target, interpolation, and residual amount.
- Nonlinear drive and ADAA order.

Engine-specific settings are experimental session state. For example, the
wavetable bank is owned by `WavetableEngine`; it is neither modulatable nor a
`ParamId`, MIDI, SysEx, or factory-preset value. Application configuration may
store a stable engine or bank ID and resolve it through the enabled registry at
startup, falling back to the build default when the feature is absent.

## State, assets, and real-time contract

Every model reports:

- Fixed mutable state bytes per lane/voice.
- Immutable shared asset bytes.
- Scratch bytes and whether they are construction-only.
- Algorithmic latency.
- Warm-up length.
- Whether its cost is bounded independently of pitch and parameters.
- Whether it is no_std compatible.

Immutable banks and fitted profiles use validated handles. `WavetableEngine`
owns a `WavetableBankRegistry`; no bank is loaded from disk or threaded through
`main`, renderer constructors, UI state, or engine APIs. The initial registry
uses generated Rust `static [f32]` data, so it is valid in `no_std`, allocation
free, and directly available to validated bank handles. A raw byte blob is
deferred because it needs explicit alignment and decoding before it can become
`&[f32]`.

## Switching behavior

Oscillator Lab may switch models by constructing/resetting the newly selected
analysis model. Its parameters and waveform displays do not affect live audio.
The playable synth chooses one engine for both oscillators through the global
selector in the Params Oscillators header; there is no load-from-Oscillator-Lab
operation.

Desktop audition switching follows plan 14: all-notes-off, block-boundary
selection, common-parameter synchronization, then new notes. It does not
reconstruct engines. The inactive engine retains phase, correction history,
wavetable position, noise state, and other private state; activation must not
reset it. Phase 0 does not morph sustaining notes.

If patch-owned runtime selection is later approved:

- Select the model at note start and keep it fixed for the note lifetime.
- Preconstruct all permitted model storage.
- Crossfade or let old voices finish; never reinterpret unrelated state.
- Branch once per block where possible.
- Preserve old patches as baseline-model patches.

That broader patch-owned work is outside this research audition architecture.

## Implementation sequence

1. Freeze baseline regression vectors.
2. Extract the desktop semantic control/event types without moving production
   phase or slop behavior.
3. Add adapters for current AnalogOscillator and WavetableOscillator.
4. Add the closed desktop model registry and capability metadata.
5. Implement the retained complete-engine owner and prove the BLEP-only build
   is bit-identical through the full voice path, then add Pass Through
   according to plan 14.
6. Reuse model IDs and metadata in Oscillator Lab and the offline harness while
   keeping their instances and parameters independent from the playable synth.
7. Add one trivial stateful test model to prove reset, sync-event, latency, and
   diagnostics plumbing.
8. Confirm single-engine builds retain only their selected implementation and
   desktop builds expose only their enabled engine descriptors.
9. Add new methods only through independent experiment branches.

## Verification

- Baseline output remains bit-identical after adapter introduction.
- A stateful model can own phase/history without modifying AnalogOscillator.
- Unsupported waveform or event capabilities are visible, not silently
  approximated.
- Engine-specific session configuration round-trips by stable ID and falls
  back safely when its engine feature is unavailable.
- No experiment identifier enters production patches.
- Oscillator Lab edits cannot control live audio. Only the global Params-view
  engine selector selects the complete source section.
- BLEP-only output is bit-identical through the complete voice path.
- A release firmware build omits every disabled engine and its assets.

## Completion criteria

- Phase-kernel and stateful-model experiments coexist in one analysis registry.
- Real-time-safe candidates can be adapted as complete live engines and played
  through the real synth signal path.
- Existing compile-time platform specialization remains available through
  per-engine features.
- Production defaults, patch compatibility, and factory behavior are
  unchanged.
- Adding a new live engine requires its complete source implementation, feature,
  descriptor, session configuration, and tests.

## References

- Current typed kernel seam: synth-core/src/dsp/analog_oscillator.rs
- Existing immutable bank pattern: synth-core/src/dsp/wavetable.rs
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
