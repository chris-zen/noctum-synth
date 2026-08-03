# Analog Oscillator Research Master Plan

## How to use this programme

Read the numbered documents in order. The number is the dependency order, not
an importance score: a later experiment may be more promising, but it relies on
measurement, audition, or data work established earlier. A completed plan is
not necessarily a successful oscillator; negative results are retained because
they prevent the same failed hypothesis from being repeated.

Status notation used throughout:

- `[x]` completed and verified.
- `[~]` a usable increment exists, with explicitly listed work remaining.
- `[ ]` not yet executed.
- **Retained** means useful evidence or a comparison candidate.
- **Promoted** means approved for a production or hardware tier. Nothing is
  promoted merely because it sounds interesting in one test.

## DSP primer

An oscillator repeats a waveform. Its **phase** is the position within one
repeat, normally represented from 0 to 1. Saw and pulse waves contain abrupt
steps; triangle waves contain abrupt changes of slope. Those sharp features
require harmonics far above the played note.

Digital audio can represent frequencies only below the **Nyquist frequency**,
which is half the sample rate. Harmonics above Nyquist fold into unrelated,
usually inharmonic frequencies; this is **aliasing**. BLEP corrects sampled
steps, while BLAMP corrects slope changes. These methods suppress aliasing but
do not, by themselves, reproduce the curved ramps, droop, asymmetric edges,
frequency-dependent phase, or small nonlinearities of a physical oscillator.

A **wavetable** stores one periodic cycle. A **mip bank** stores progressively
less bright versions so the renderer can select a table whose harmonics remain
below Nyquist. An **IIR filter** is a small recursive filter whose state gives
frequency-dependent magnitude and phase. A **gray-box model** uses a simplified
physical topology with fitted parameters; a **black-box model** learns only the
input/output relation.

Measurements are split into training, validation, and blind test data. Fitting
uses training data, engineering choices may use validation data, and the blind
test is opened once for the promotion decision. An **ablation** disables one
component to discover what it contributes. **NRMSE** is RMS error normalized
to make unlike signal levels comparable. **ABX** asks whether an unknown X is A
or B; target matching separately asks which candidate is closer to a reference.
The final choice uses a **Pareto frontier**: candidates that are not beaten on
every important axis such as sound, aliasing, CPU, and memory.

## Status and purpose

This is an exploration programme, not an instruction to change the production
oscillator immediately. It organizes the literature-backed oscillator ideas
into independent experiments that can be implemented, measured, auditioned,
and discarded without destabilizing the current synth.

The programme must answer two separate questions:

1. Which methods reproduce the deterministic character of a chosen analog
   oscillator across saw, triangle, and pulse?
2. Which accepted method belongs on each eventual hardware tier?

Desktop processors are the current hard ceiling for exploration. Cortex-M7
cost remains an important measurement, but it must not prematurely exclude a
method that could target a faster embedded platform.

## Execution status

| Step | Status | Purpose |
| --- | --- | --- |
| [01](01-isolated-experiment-framework.md) | `[x]` minimum complete | Reproducible, isolated rendering and artifacts |
| [02](02-replaceable-model-architecture.md) | `[x]` minimum complete | Common research seam and closed live-engine owner |
| [03](03-evaluation-and-hardware-selection.md) | `[~]` minimum complete | Shared metrics, listening gates, and cost evidence |
| [04](04-desktop-audition-and-pass-through-filter.md) | `[~]` playable foundation complete | MIDI audition and an exact raw filter path |
| [05](05-oscillator-lab.md) | `[~]` first UI increment complete | Independent visual comparison surface |
| [06](06-reference-capture-and-identification.md) | `[~]` Monologue and Prophet-5 V static data available | Named, reproducible reference datasets |
| [07](07-automated-synth-reference-capture.md) | `[~]` phases 1–6 and live acceptance complete | Resumable MIDI/audio acquisition and extraction |
| [08](08-full-voice-characterisation.md) | `[ ]` planned | Protocols for oscillator, filter, modulation, and control laws |
| [09](09-target-conditioned-phase-filter.md) | `[x]` closed; retained evidence | Compact fitted phase/filter hypothesis |
| [10](10-measured-wavetable-residual.md) | `[x]` frozen desktop experiment | Measured deterministic waveform candidate |
| [11](11-multirate-measured-wavetables.md) | `[~]` Monologue + Prophet v2 banks built; combined gates open | Sample-rate-independent pitch-by-mip banks |
| [12](12-coherent-gray-box-core.md) | `[ ]` planned | Simplified physical oscillator state model |
| [13](13-nonlinear-phase-blep-and-lp-blit.md) | `[ ]` planned | Better causal edge and bandwidth models |
| [14](14-antialiased-nonlinearity.md) | `[ ]` planned | Color stages without uncontrolled aliasing |
| [15](15-drift-variation-and-calibration.md) | `[ ]` planned | Measured static and time-varying differences |
| [16](16-neural-offline-reference.md) | `[ ]` optional reference branch | Learned quality ceiling and distillation teacher |
| [17](17-full-circuit-wdf-model.md) | `[ ]` gated reference branch | Schematic-level physical model |

The minimum pre-candidate foundation is implemented:

- Plans 01 and 02 provide a feature-gated registry, common scalar model
  interface, semantic events, deterministic case runner, baseline/wavetable
  adapters, and a stateful-model contract verified by an independent probe.
- Plan 06 provides the verified Monologue dataset import and deterministic
  derived references; additional Arturia/hardware captures remain separate.
- Plan 03 provides versioned signal, comparison, spectral-residual, performance,
  WAV, and JSON artifact output with focused analytic tests.
- Plan 04 provides the stable live selector and Pass Through filter.
- The first Plan 05 increment is also complete: Osc Design has an independent,
  right-aligned selector for the same live-capable oscillator models exposed in
  Params, and renders them through the same closed dispatcher. This was pulled
  forward because visual inspection is research infrastructure for subsequent
  candidates; playing and live selection remain in Params.

This is sufficient to continue candidate work without inventing candidate-local
comparison machinery. The full Oscillator Lab overlays, automated whole-matrix runs,
listening-set/ABX management, and hardware promotion automation remain later
foundation increments.

Plan 09 now has a retained first Monologue fit and an analysis-only Rust
adapter. Its held-out medians beat the geometric baseline for saw, triangle,
and pulse. Runtime/reference parity and initial 48/96 kHz static residual sweeps
are complete; one borderline top-range saw case remains. The first completed
blind session scored 9/9 in ABX, but judged the production baseline closer to
the measured deterministic target in 8/9 cases. The candidate therefore fails
the perceptual promotion gate. A six-case phase-only/filter-only blind ranking
package from unused validation pitches is now complete. It finds no universal
component winner: filtering hurts saw, helps pulse, and interacts with phase in
a pitch-dependent way for triangle. Full validation-pitch ablation curves are
also complete. They reveal that fixed cycle-phase error rewards an inaudible
trigger/cycle-origin match and that the fit used a geometric rather than the
production BLEP baseline. The phase-invariant, production-baseline v2 objective,
clean refit, compiled-runtime evaluation, and 48/96 kHz static residual gate are
now complete. The engine still resets all waves at phase zero; evaluation
removes only the arbitrary whole-cycle rotation. Compiled v2 improves
phase-aligned shape in all 108 held-out cases and has no material static
residual failures. Triangle is also consistently better in harmonic magnitude;
saw and pulse have offline-predictor/runtime magnitude mismatch, and pulse is
approximately flat versus baseline on that metric. The fresh blind test-split
gate is now complete: ABX was 6/9 (`p = 0.25391`), and target matching selected
baseline in 5/9 cases versus v2 in 4/9. Plan 09 v2 is retained as reproducible
evidence but closed for promotion; no live adapter or broad dynamic sweep will
be built. Triangle remains only a hypothesis for a later cross-candidate test.

Plan 10 has now completed a training-only offline representation study.
Measured tables strongly improve held-out shape and magnitude metrics. Full
measured spectra and production-plus-residual spectra are effectively tied, so
the simpler full measured representation is selected for the first desktop
runtime. Saw uses global-phase canonicalization; triangle and fixed-width pulse
use the extracted landmark directly. The versioned 864-KiB external bank and
scalar desktop runtime are now implemented and checksum-validated. Compiled
shape improves in 107/108 held-out cases overall, with zero material static
residual failures at 48/96 kHz. Triangle is strongest; saw magnitude reveals a
conservative-bandwidth tradeoff. A fresh blind test on previously unheard test
pitches is complete. ABX was 6/9 (`p = 0.25391`), and target matching selected
the measured candidate in 6/9 cases. Saw choices were explicitly
indistinguishable at 0.1 confidence; triangle favored the candidate 2/3 and
pulse favored baseline 2/3. The unchanged model advances to bounded runtime-cost
and dense pitch-transition stress as an experimental desktop candidate, not a
production promotion. No bank or guard tuning is permitted from these revealed
answers.

To maximize the value of the remaining research budget, the exhaustive Plan 10
cost/transition matrix is deferred. The checksum-validated bank is now connected
to the existing desktop oscillator selector for direct musical audition, with
production fallback outside its supported waveform and pitch domain. This is an
experimental listening surface, not a promotion of the model.

Continuous shape is now implemented in that shared live/research adapter. Saw
and triangle retain the production phase-morph semantics using measured tables;
saw-triangle crossfades measured sources; pulse width is formed from two
phase-shifted measured saws while a residual preserves the exact measured
50-percent pulse endpoint. A 120-case static sweep across shapes 0, 0.5, and 0.9
at 48/96 kHz has zero material residual failures.

Slop, detune, and glide continuity are now connected. The measured adapter
tracks the production oscillator's effective per-lane drift frequency on every
sample while preserving measured-table phase; ordinary detune and glide keep
using the same phase-continuous frequency path. A dense 96,000-sample shaped
triangle sweep from 20.7 Hz to 1.2 kHz crosses all measured pitch intervals
without a material step, and an end-to-end live-engine comparison confirms
that slop changes the audible measured output.

Hard sync is now supported by the measured adapter for every waveform/shape
path. It preserves the master wrap's fractional sample position, resets the
measured slave to the phase reached during the remainder of that sample, and
adds a bounded four-sample Pirkle-table BLEP residual scaled to the actual
waveform jump. Live-engine routing, fractional-offset determinism, and bounded
output across saw, saw-triangle, triangle, and pulse/PWM are covered by the
research integration suite. This is compatibility behavior rather than a
Monologue target-match claim because the public captures contain no hard-sync
reference cases.

The compact Plan 10 dynamic gate is now complete. Its first run exposed and
fixed a wrong-sign PWM DC-compensation term; the neutral measured pulse and
earlier blind set were unaffected. After the fix, all dynamic renders are
bit-deterministic and the measured candidate has lower 48-versus-filtered-192
kHz disagreement than baseline for pitch/shape, audio-rate PWM, sync ratios,
and their combined case. Hard sync remains the worst case at 0.182 NRMSE. Full
synth p99 cost is 1.84% of one 48 kHz frame for a steady voice, 4.08% for four
voices, and 7.46% for the combined one-voice stress profile on this host. Plan
07 is therefore frozen as a desktop real-time experimental candidate, not a
production or embedded promotion.

Plan 11 now has Monologue and Prophet-5 V schema-v2 banks. The authoritative
Rust generator reconstructs 33 universal mip levels directly from the measured
complex spectra, the allocation-free renderer performs pitch-by-mip
interpolation from each lane's effective phase increment, and Osc Design
reports the one-semitone captured-range transition. Compiled sizes are
7,216,128 bytes (Monologue) and 7,416,576 bytes (Prophet); combined embed is
14,632,704 bytes under the 20 MiB cap. The Prophet bank comes from the verified
r7 capture after extraction revision 2. Combined held-out metrics, alias
sweeps, and the zero-miss 60-second soak remain open.

## Ground truth in the current repository

- The current table-BLEP/PolyBLEP and PolyBLAMP oscillator remains the
  compatibility baseline in synth-core/src/dsp/analog_oscillator.rs and
  synth-core/src/dsp/blep.rs.
- A private typed OscillatorKernel seam and a runtime BLEP/PolyBLEP analysis
  selector already exist.
- The current engine chooses one typed kernel at build time. The planned live
  seam is instead a complete feature-gated oscillator engine per voice: a
  single-engine build dispatches directly, while a desktop all-engine build
  selects among retained complete engines inside one closed owner.
- The retained wavetable prototype is in synth-core/src/dsp/wavetable.rs.
- The Osc Design view uses the same closed preview dispatcher as the live
  engine while keeping its selection independent from the played synth.
- The playable voice path already centralizes Osc 1/Osc 2 in
  synth-core/src/voice/oscillators.rs and passes their mix through the
  runtime-selectable Filter in synth-core/src/voice/mod.rs.
- Existing quality and listening tools are
  synth-tools/src/bin/sample_rate_quality.rs and
  synth-tools/src/bin/wavetable_listening_samples.rs.
- Daisy currently runs a 48 kHz codec with a 24 kHz internal pipeline for its
  measured production budget. Desktop experiments may use 44.1, 48, 96, or
  192 kHz.

Existing work that must be preserved and cross-checked:

- plans/REV2_OSCILLATOR_PARITY_PLAN.md
- plans/TRIANGLE_WAVEFORM_OPTIMIZATION_PLAN.md
- plans/PULSE_WAVEFORM_OPTIMIZATION_PLAN.md
- plans/DAISY_WAVETABLE_PROTOTYPE_PLAN.md
- plans/DAISY_WAVETABLE_PROTOTYPE_REPORT.md
- plans/DAISY_SAMPLE_RATE_QUALITY_REPORT.md

## Programme rules

1. Baseline immutability: every experiment retains a selectable current-output
   baseline. No experiment silently changes existing presets, MIDI/SysEx
   meanings, feature defaults, or EngineOscillator.
2. Research isolation: experimental assets, fit data, UI configuration, and
   engine-specific settings remain outside the production patch schema until a
   model is explicitly promoted. Session configuration may restore a selected
   engine or its selected bank but is neither modulatable nor patch state.
3. Reproducibility: every result records commit, model revision, target
   manifest, sample rate, seed, parameters, metrics, CPU, and memory.
4. Separate aliasing from character: alias energy, target-spectrum error, time
   shape, instability, and listening preference are reported independently.
5. Deterministic comparison: common phase, note, level, warm-up, and render
   length are used for every candidate. Stochastic layers can be disabled.
6. Desktop first, hardware second: candidates first need correctness and
   perceptual value. Hardware qualification determines placement, not whether
   an idea may be explored.
7. No hidden target conflation: Korg Monologue, Arturia Prophet-5 V, and any
   later hardware recordings remain separate named profiles. Arturia is not a
   Prophet Rev2 measurement.
8. Promotion is explicit: an experiment can be retained as desktop-only,
   promoted to a powerful embedded tier, promoted to Daisy, or archived.
9. Listen as well as inspect: once a model reaches a stable render milestone,
   it must be auditionable from MIDI through the desktop synth, both through a
   bit-transparent pass-through filter and through existing filters.

## Dependency map

The file numbers encode this default route:

```text
01 isolation -> 02 architecture -> 03 evaluation -> 04 audition -> 05 lab
                                                    |
06 references -> 07 automated capture -> 08 full-voice programme
       |
       +-> 09 compact fitted model (closed)
       +-> 10 measured wavetable (frozen) -> 11 multirate revision
       +-> 12 gray-box core -> 13 edge/bandwidth alternatives
                                |
                                +-> 14 nonlinearity -> 15 variation

16 neural reference and 17 circuit model start only when their entry gates
justify their extra complexity.
```

Plans 01–08 are shared infrastructure and evidence collection. Plans 09–13 are
independent deterministic oscillator hypotheses. Plans 14–15 are cross-cutting
character layers and must be evaluated both alone and on surviving parent
oscillators. Plans 16–17 are expensive reference ceilings, not presumed product
implementations.

## Execution policy

Follow the file numbers unless a plan's entry criteria explicitly permit
parallel work. The common evaluator in plan 03 applies at every later step; it
is numbered once rather than repeated after every candidate. Likewise, stable
real-time candidates return through plan 04 for musical audition and plan 05
for visual comparison.

Plan 09 was executed before all later candidate branches and is closed as a
well-documented negative promotion result. Plan 10 is the retained desktop
candidate. Plan 11 is its next required engineering revision. Plans 12 and 13
may then run independently. Do not add plans 14 or 15 to a parent oscillator
until ablation proves that the parent still needs that effect.

Promotion remains a separate decision after blind listening and cost data. A
candidate may be archived, retained for desktop research, assigned to a more
powerful embedded target, or qualified for Daisy.

Parallel exploration is allowed after the foundation work because every branch
must implement the same research adapter, use immutable target data, and write
results to a model-specific artifact directory.

## Expected final outputs

- Baseline-preserving desktop comparison and live audition tools.
- A Pass Through (Raw) filter model with exact unity gain and zero latency.
- Reproducible target manifests and derived features.
- At least four independently selectable oscillator candidates.
- Raw and filtered repeatable musical A/B recordings for live-capable models.
- Parameter presets that reproduce each named target.
- Objective reports and level-matched listening sets.
- CPU, state, table, and code-size profiles for desktop and available hardware.
- A written placement decision for each candidate: archive, desktop-only,
  powerful embedded, or Daisy.

## Core literature

- Jussi Pekonen, Filter-Based Oscillator Algorithms for Virtual Analog
  Synthesis:
  <https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/>
- Pekonen et al., Discrete-Time Modelling of the Moog Sawtooth Oscillator
  Waveform:
  <https://link.springer.com/article/10.1155/2011/785103>
- Pekonen and Holters, Nonlinear-Phase Basis Functions in Quasi-Bandlimited
  Oscillator Algorithms:
  <https://dafx.de/paper-archive/2012/papers/dafx12_submission_15.pdf>
- Kraft and Zölzer, LP-BLIT:
  <https://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_59.pdf>
- Olsen, Werner, and Germain, Network Variable Preserving Step-size Control in
  Wave Digital Filters:
  <https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf>
- Simionato and Fasciani, Towards Neural Emulation of Voltage-Controlled
  Oscillators:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Bilbao et al., Antiderivative Antialiasing for Memoryless Nonlinearities:
  <https://research.aalto.fi/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/>
- Arturia Prophet V manual:
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
- Sequential Prophet Rev2 User's Guide:
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
