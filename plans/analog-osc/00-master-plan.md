# Analog Oscillator Research Master Plan

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

The minimum pre-candidate foundation is implemented:

- Plans 01 and 02 provide a feature-gated registry, common scalar model
  interface, semantic events, deterministic case runner, baseline/wavetable
  adapters, and a stateful-model contract verified by an independent probe.
- Plan 03 provides the verified Monologue dataset import and deterministic
  derived references; additional Arturia/hardware captures remain separate.
- Plan 13 provides versioned signal, comparison, spectral-residual, performance,
  WAV, and JSON artifact output with focused analytic tests.
- Plan 14 provides the stable live selector and Pass Through filter.

This is sufficient to start candidate Plan 04 without inventing candidate-local
comparison machinery. The full Oscillator Lab UI, automated whole-matrix runs,
listening-set/ABX management, and hardware promotion automation remain later
foundation increments.

Plan 04 now has a retained first Monologue fit and an analysis-only Rust
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
baseline in 5/9 cases versus v2 in 4/9. Plan 04 v2 is retained as reproducible
evidence but closed for promotion; no live adapter or broad dynamic sweep will
be built. Triangle remains only a hypothesis for a later cross-candidate test.

Plan 07 has now started with a training-only offline representation study.
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

To maximize the value of the remaining research budget, the exhaustive Plan 07
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

The compact Plan 07 dynamic gate is now complete. Its first run exposed and
fixed a wrong-sign PWM DC-compensation term; the neutral measured pulse and
earlier blind set were unaffected. After the fix, all dynamic renders are
bit-deterministic and the measured candidate has lower 48-versus-filtered-192
kHz disagreement than baseline for pitch/shape, audio-rate PWM, sync ratios,
and their combined case. Hard sync remains the worst case at 0.182 NRMSE. Full
synth p99 cost is 1.84% of one 48 kHz frame for a steady voice, 4.08% for four
voices, and 7.46% for the combined one-voice stress profile on this host. Plan
07 is therefore frozen as a desktop real-time experimental candidate, not a
production or embedded promotion.

## Ground truth in the current repository

- The current table-BLEP/PolyBLEP and PolyBLAMP oscillator remains the
  compatibility baseline in synth-core/src/dsp/analog_oscillator.rs and
  synth-core/src/dsp/blep.rs.
- A private typed OscillatorKernel seam and a runtime BLEP/PolyBLEP analysis
  selector already exist.
- The engine chooses one typed kernel at build time; it does not branch among
  research models in the production sample loop.
- The retained wavetable prototype is in synth-core/src/dsp/wavetable.rs.
- The Osc Design view currently renders AnalogOscillator directly from
  synth-app/src/ui/analysis/osc_design.rs.
- The playable voice path already centralizes Osc 1/Osc 2 in
  synth-core/src/voice/oscillators.rs and passes their mix through the
  runtime-selectable Filter in synth-core/src/voice/mod.rs.
- Existing quality and listening tools are
  synth-core/examples/sample_rate_quality.rs and
  synth-core/examples/wavetable_listening_samples.rs.
- Daisy currently runs a 48 kHz codec with a 24 kHz internal pipeline for its
  measured production budget. Desktop experiments may use 44.1, 48, 96, or
  192 kHz.

Existing work that must be preserved and cross-checked:

- plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md
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
   model parameters remain outside the production patch schema until a model
   is explicitly promoted.
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

## Workstreams

### Foundation

| Document | Outcome | Dependency |
| --- | --- | --- |
| 01-isolated-experiment-framework.md | Reproducible model registry, render harness, artifacts, and baseline guard | None |
| 02-replaceable-model-architecture.md | Safe seam for phase kernels and stateful models | 01 |
| 14-desktop-audition-and-pass-through-filter.md | Minimal feature-gated live source selection, raw filter path, and repeatable A/B playing | 01 and 02 |
| 03-reference-capture-and-identification.md | Target datasets and fitted deterministic/stochastic features | 01 |
| 15-automated-synth-reference-capture.md | Generic MIDI/audio capture projects, interruption-safe recording, and Rust extraction | 03 |
| 12-osc-designer-view.md | Evolve Osc Design into an isolated Oscillator Lab for model/reference exploration | 01 and 02 |
| 13-evaluation-and-hardware-selection.md | Shared quality, listening, CPU, memory, and promotion protocol | 01 |

### Main oscillator candidates

| Document | Research question |
| --- | --- |
| 04-target-conditioned-phase-filter.md | How far can a compact phase warp plus pitch/PW-dependent IIR go? |
| 05-coherent-gray-box-core.md | Does a shared capacitor/comparator/reset model give more convincing correlated behavior? |
| 06-nonlinear-phase-blep-and-lp-blit.md | Do causal edge kernels and controllable source bandwidth improve character without a full target model? |
| 07-measured-wavetable-residual.md | Can measured phase and spectral detail be retained with low runtime cost? |

### Cross-cutting character

| Document | Research question |
| --- | --- |
| 08-antialiased-nonlinearity.md | Which subtle nonlinear stages add useful color without reintroducing aliasing? |
| 09-drift-variation-and-calibration.md | Which measured static and stochastic variations improve polyphonic behavior? |

### Expensive/reference branches

| Document | Research question |
| --- | --- |
| 10-neural-offline-reference.md | Can the public neural VCO work serve as a teacher or desktop ceiling? |
| 11-full-circuit-wdf-model.md | Is a schematic-driven WDF/state-space oscillator worth its complexity? |

Each candidate may start as a standalone desktop adapter after workstream 01.
Workstream 02 establishes the common model seam. Workstream 14 then connects
stable real-time-safe candidates to the playable desktop voice through one
global Params-view selector, without changing patches or production firmware.
Oscillator Lab remains an independent analysis surface. That minimal audition layer
precedes the broader patch-owned architecture in
plans/OSCILLATOR_ENGINE_ARCHITECTURE_PLAN.md.

## Recommended execution order

1. Build the isolated harness and regression baseline.
2. Prepare the replaceable-model seam.
3. Implement the minimal desktop audition layer and Pass Through filter from
   workstream 14. Prove the current oscillator remains bit-identical, then play
   the existing PolyBLEP or wavetable alternative through raw and filtered
   paths.
4. Evolve Osc Design into the isolated Oscillator Lab for waveform/parameter
   exploration and reference comparison, without a load-to-synth handoff.
5. Import the public Monologue dataset, define the Arturia capture manifest,
   then implement the generic automated capture and Rust extraction tool from
   workstream 15.
6. Implement the target-conditioned model, measured wavetable model, and
   nonlinear-phase/LP-BLIT model independently.
7. Implement the coherent gray-box model after the first fitted target features
   reveal the required topology and frequency dependencies.
8. Add nonlinear coloration as an independently switchable stage.
9. Fit static variation and drift only after deterministic single-cycle error
   is understood.
10. Use the neural and full-circuit branches as reference ceilings, not default
   production candidates.
11. Run the common evaluation matrix and place surviving candidates on a
   quality/cost Pareto frontier.
12. Promote a model only in a separate implementation decision.

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
