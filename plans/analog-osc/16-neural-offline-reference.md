# Neural VCO Offline Reference and Distillation

**Order:** 16 · **Depends on:** plans 03 and 06; useful after conventional
candidates exist · **State:** `[ ]` optional reference branch.

## DSP background

An autoregressive neural model predicts each sample from controls and recent
history. It can represent behavior that is hard to write as equations, but it
may be expensive, delayed, unstable outside training data, or aliased. Here it
is first an offline teacher. Distillation means using its outputs to fit a
smaller explicit model, table, filter, or residual rather than assuming the
network itself belongs in the product.

## Objective

Reproduce the public neural VCO work as a desktop/offline reference ceiling,
then test whether its learned behavior can be distilled into the compact phase,
filter, residual-table, or gray-box models.

Neural inference is not a presumed production solution.

## Initial scope

- Reproduce inference using the authors' published code and weights.
- Verify outputs against their Monologue test data.
- Render the same static and sweep cases used by the common harness.
- Measure quality, cumulative pitch error, latency/context requirements,
  FLOPs, memory, and real-time factor on supported desktop processors.
- Do not add TensorFlow or Python dependencies to synth-core or firmware.

## Environment isolation

Place training/inference tooling in a separate research environment with a
locked dependency file. The Rust repository exchanges neutral artifacts:

- WAV renders.
- JSON manifests and metrics.
- Optional compact tensor/weight export for a later native desktop prototype.

Downloaded datasets and weights remain outside Git and are verified by
checksum/license metadata.

## Reproduction protocol

1. Download the CC BY 4.0 dataset and LGPL-licensed code from their canonical
   sources.
2. Reproduce pretrained inference before training anything.
3. Validate frequency, waveform shape, initialization context, and output
   normalization against the paper/companion examples.
4. Re-run metrics on the paper's split where possible.
5. Evaluate free-running durations much longer than the training window to
   expose cumulative pitch/phase mismatch.
6. Render unseen pitches and continuous sweeps.
7. Record exact environment and model checksum.

## Architecture experiments

The causal inference shape is:

```rust
for sample in output {
    let controls = conditioning.at_current_time();
    *sample = network.predict(&history, controls);
    history.push(*sample);
}
```

Because each prediction affects the next, small errors can accumulate. Tests
must cover long free runs, resets, modulation, and controls outside the training
grid rather than only teacher-forced one-step accuracy.

Evaluate only after reproduction:

- Published 64-, 32-, and 16-unit recurrent variants.
- A model trained jointly on saw, triangle, and square.
- Separate per-waveform models.
- Longer/shorter context and state initialization.
- Optional lightweight native inference only if the 16-unit model offers
  meaningful value.

Do not optimize neural architecture indefinitely. The main purpose is to learn
what deterministic variation compact models fail to capture.

## Distillation experiments

From neural and target renders:

- Fit plan 09 phase/filter parameters and compare residual error.
- Derive plan 10 low-rank residual tables from neural predictions.
- Fit gray-box parameters to neural-generated pitch sweeps.
- Analyze which harmonics/time features remain unexplained.

The preferred output of this work is often a better compact model or target
profile, not a neural runtime.

## Aliasing caveat

The autoregressive model learns the bandwidth and aliasing in its training
recordings/downsampling chain. It has no independent mathematical
bandlimiting guarantee. Evaluate folded energy at every pitch and compare with
the source recordings as well as BLEP-based candidates.

## Desktop real-time option

A native desktop plugin/model may be considered only if:

- Long free-running pitch is stable.
- Continuous frequency/PW control is artifact-free.
- It beats compact models in blinded target matching.
- It meets the desktop polyphonic budget with margin.
- Model initialization does not require hidden target audio at note start.

Otherwise keep it offline.

## Acceptance and stop rules

The reproduction succeeds when published behavior and approximate metrics are
recovered. The branch provides value if it exposes target structure not
captured by compact models or improves their fitted profiles.

Stop production-inference work if smaller networks do not beat the compact
model, if low-frequency/cumulative pitch errors remain, or if compute is
incompatible with desired polyphony.

## Deliverables

- Locked reproduction environment and checksum manifest.
- Canonical paper-model render set.
- Long-run, sweep, alias, CPU, and memory report.
- Distillation report into plans 09, 10, and 12.
- Explicit decision: offline teacher, desktop runtime, or archive.
- Live audition adapter only if a bounded native model meets the desktop
  real-time criteria; otherwise provide clearly labeled offline listening
  renders and do not expose it as playable.

## References

- Simionato and Fasciani, Towards Neural Emulation of Voltage-Controlled
  Oscillators:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Companion page and audio examples:
  <https://riccardovib.github.io/NeuralOSC_pages/>
- Authors' code and pretrained weights:
  <https://github.com/RiccardoVib/NeuralOSC>
- Analog and Synthetic VCO dataset:
  <https://zenodo.org/records/15196138>
- DDSP, for interpretable differentiable DSP context:
  <https://arxiv.org/abs/2001.04643>
- Compact target model:
  plans/analog-osc/09-target-conditioned-phase-filter.md
- Measured residual model:
  plans/analog-osc/10-measured-wavetable-residual.md
- Live audition eligibility:
  plans/analog-osc/04-desktop-audition-and-pass-through-filter.md
