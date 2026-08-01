# Isolated Oscillator Experiment Framework

## Objective

Create a reproducible desktop research path in which oscillator candidates can
be added, rendered, compared, and removed without changing the production
EngineOscillator, patch format, factory presets, or hardware feature defaults.
Stable bounded candidates must also be transferable into the separate playable
desktop audition facade without making that facade part of normal firmware.

This plan is infrastructure only. It does not implement a new synthesis method.

## Implementation status

The minimum candidate-ready framework is implemented behind the
`oscillator-research` feature:

- `synth-core/src/dsp/oscillator_research.rs` defines stable model descriptors,
  deterministic cases, semantic events, model-owned parameter metadata, a
  model-family-neutral interface, and a no-allocation caller-owned-buffer
  runner.
- The deterministic registry includes production baseline, table BLEP,
  PolyBLEP/PolyBLAMP, and the retained asset-backed wavetable prototype.
- `synth-tools/src/bin/analog_osc_research.rs` lists models and writes float WAV
  renders plus self-describing versioned JSON artifacts under
  `target/analog-osc`.
- `synth-core/tests/analog_osc_research.rs` verifies registry order, repeated
  bit identity, common baseline/wavetable execution, typed failures, and a
  fully stateful adapter that does not contain `AnalogOscillator`.

Dynamic case-matrix orchestration, target lookup, and Oscillator Lab UI wiring
remain later increments; individual candidate plans can already use the common
runner and artifact schema.

## Current integration points

- synth-core/src/dsp/analog_oscillator.rs provides AnalogOscillator, the private
  OscillatorKernel trait, typed engine aliases, phase/sync metadata, and slop.
- synth-core/src/dsp/wavetable.rs demonstrates an explicitly constructed
  alternative typed kernel with immutable external data.
- synth-core/examples/sample_rate_quality.rs provides an offline spectral
  starting point.
- synth-app/src/ui/analysis/osc_design.rs constructs the baseline oscillator
  directly and owns its own serializable view configuration.
- hardware/daisy/firmware/src/bin/bench-dsp.rs and
  plans/DAISY_SAMPLE_RATE_QUALITY_REPORT.md define measured embedded gates.

## Isolation boundary

Add a desktop research adapter with these semantic operations:

- Return a stable model identifier, display name, revision, and capability set.
- Configure sample rate, waveform, frequency, shape/pulse width, seed, and
  model-specific parameters.
- Reset all state or selected lanes deterministically.
- Render one sample or a requested mono analysis block.
- Report latency, required warm-up, state bytes, immutable asset bytes, and
  supported sample rates.

The adapter is used by offline examples and the isolated Oscillator Lab evolved
from Osc Design. The same immutable metadata and
semantic operations feed the feature-gated desktop audition facade in plan 14.
It must not replace the normal production voice engine, which continues to
instantiate its existing typed kernel alias when research features are off.

The research registry should initially live behind the desktop/std build
boundary. Heavy models may allocate during construction or offline rendering,
but allocation is forbidden inside a render call. Models intended for later
embedded use must additionally expose a fixed-state implementation.

## Model identity and parameters

Every candidate receives:

- A lowercase stable ID such as baseline-table-blep-v1.
- A monotonically updated model revision.
- A typed configuration owned by the model.
- A serializable desktop parameter map with explicit defaults and ranges.
- A capability declaration for saw, triangle, pulse, PWM, audio-rate PWM,
  hard sync, free-running phase, target profiles, stochastic variation, and
  real-time safety.

Unknown saved parameter keys are ignored with a warning; missing keys use the
model revision's documented defaults. Research configurations do not enter the
normal synth patch or ParamId namespace.

## Artifact layout

Generated material should be organized under a gitignored research output root,
for example target/analog-osc:

- renders/model-id/case-id.wav
- metrics/model-id/case-id.json
- plots/model-id/case-id/
- listening/comparison-id/
- fits/target-id/model-id/
- benchmarks/platform/model-id/

Each result JSON records:

- Git commit and dirty-worktree marker.
- Model ID, revision, complete parameters, and seed.
- Target ID and manifest checksum when applicable.
- Waveform, note/frequency, pulse width, sample rate, render length, and
  warm-up.
- RMS/peak normalization policy.
- Metric implementation revision.
- Wall-clock and, where available, cycle measurements.

Large recordings, public datasets, trained weights, and generated tables must
not be committed accidentally. Commit only compact manifests, fit parameters
approved for source control, and summarized reports.

## Baseline guard

Before any candidate work:

1. Render deterministic regression vectors for current table BLEP and
   PolyBLEP across saw, triangle, pulse, shape morph, PWM, hard sync, note
   reset, and pitch changes.
2. Record hashes and numerical tolerances. Use bit identity when the baseline
   code path is unchanged; use an explicit maximum error only for approved
   refactors.
3. Add a test proving that merely registering desktop experiments does not
   alter EngineOscillator selection or binary behavior.
4. Keep existing factory-corpus and Daisy benchmark entry points unchanged.
5. Make baseline the default Oscillator Lab selection and default saved config.

## Render case schema

Define reusable cases rather than hard-coded loops:

- Static notes: E0 through E6 from the public dataset plus MIDI 84, 96, and
  108 for high-frequency alias stress.
- Sample rates: 24, 44.1, 48, 96, and 192 kHz where supported.
- Waveforms: saw, triangle, pulse; SawTri remains a synth compatibility case.
- Pulse widths: 10, 25, 50, 75, and 90 percent.
- Dynamic cases: pitch sweep, PWM sweep, audio-rate PWM, hard sync ratios,
  note reset/free run, and abrupt/continuous frequency changes.
- Stochastic cases: disabled, fixed seed, and a long-run statistics render.

Cases can request a target recording, a baseline comparison, or a pure aliasing
reference.

## Verification

- Registry enumeration is deterministic.
- Repeated renders with the same seed are byte-identical.
- Baseline hashes remain unchanged.
- No render path allocates after construction for models marked real-time safe.
- Invalid parameters are clamped or rejected consistently and never yield
  NaN/Inf.
- Model failure is isolated to the requested render and reported in the UI.
- The framework works with one scalar lane even if production remains four-lane
  SIMD.
- Running framework tests does not require downloading external datasets.

## Completion criteria

- A new dummy model can be registered and compared without editing production
  voice code.
- Baseline and wavetable adapters run through one case harness.
- A live-capable adapter can be handed to the audition facade without entering
  patch or firmware defaults.
- Artifacts are self-describing and reproducible.
- The normal synth, existing presets, and firmware features remain unchanged.

## References

- Current oscillator: synth-core/src/dsp/analog_oscillator.rs
- Current BLEP implementation: synth-core/src/dsp/blep.rs
- Existing wavetable adapter pattern: synth-core/src/dsp/wavetable.rs
- Existing quality harness: synth-core/examples/sample_rate_quality.rs
- Prior architecture direction: plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md
- Prior wavetable report: plans/DAISY_WAVETABLE_PROTOTYPE_REPORT.md
- Playable audition and pass-through filter:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
- Välimäki, Pekonen, and Nam, perceptually informed integrated polynomial
  waveform synthesis:
  <https://pubmed.ncbi.nlm.nih.gov/22280720/>
- Pekonen thesis:
  <https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/>
