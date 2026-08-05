# Korg Monologue gray-box saw-core v1

## Decision

Plan 12 is closed as a **negative topology result**. The bounded saw-core is
retained in the research registry for reproducibility, but it does not meet the
target-quality acceptance rule and is not routed into the live desktop engine
or production presets.

The Monologue evidence supports testing a saw-core first: saw is the visible
primary ramp. It does not justify a triangle-core. Saw, triangle, and square
were captured independently with different phase policies, so cross-waveform
phase is not an identifiable fit target; the prototype therefore shares state
structurally without claiming measured simultaneous phase coherence.

## Prototype

`korg-monologue-gray-box-saw-core-v1` is a scalar Tier 2 model with:

- one normalized capacitor state and affine voltage-dependent charge current;
- exact closed-form state updates and fractional threshold/reset times;
- a fixed maximum of two state events per sample for the validated frequency
  and parameter domain;
- saw from capacitor voltage, pulse from a voltage comparator, and triangle
  from a fold of the same capacitor;
- causal table BLEP for amplitude steps and the existing polyBLAMP triangle
  corner correction where its variable-rate assumption remains valid;
- fixed-size correction/filter state, no allocation, and diagnostic frames for
  capacitor, threshold, comparator, raw/corrected output, and event offset.

The research CLI can write those frames with `--trace-csv`. The tracked plot is
[`../plots/korg-monologue-gray-box-saw-core-v1.svg`](../plots/korg-monologue-gray-box-saw-core-v1.svg).

## Fit and held-out result

The reproducible fitter uses the lowest measured cycle of each independent
waveform for the low-frequency fit and withholds eight notes per waveform. The
physical profile and all 24 cases are in
[`../profiles/korg-monologue-gray-box-saw-core-v1.json`](../profiles/korg-monologue-gray-box-saw-core-v1.json).

| Metric | Gray-box | BLEP baseline |
| --- | ---: | ---: |
| Held-out phase-aligned shape NRMSE, median | **0.4152** | **0.0864** |
| Held-out wins | **2 / 24** | **22 / 24** |
| Median relative improvement | **−445%** | reference |

The optimizer drove current curvature to its −0.85 bound, saw cutoff to the
60 kHz bound, and pulse cutoff to the 800 Hz bound. An adversarial rerun raised
the budget from 180 to 800 evaluations; it stopped after 298 on parameter-step
tolerance while retaining those active bounds and high gradient optimality.
That condition is recorded in the profile and is not presented as a uniquely
identified physical estimate. The boundary solution and held-out loss are
evidence that the affine shared core is too restrictive rather than evidence
for another round of unconstrained circuit detail.

Plan 09 is much stronger on the same target: its compiled medians are 0.0097
saw, 0.0046 triangle, and 0.0332 pulse. Plan 11's corresponding held-out
medians are 0.0288, 0.0034, and 0.0271. Plan 12 therefore loses to both prior
candidates as well as the baseline.

## Event and alias checks

The fit requested a 0.00098-cycle finite reset. Its first bounded causal BLAMP
implementation materially increased the 5,777 Hz saw residual. The adversarial
pass removed that regressing correction; the runtime profile selects the
zero-duration reset ablation and uses the established table BLEP. This is
recorded explicitly in the JSON rather than silently changing the fit.

At 48 kHz and 5,777 Hz, enabling event correction changed the CLI residual as
follows (more negative is better):

| Output | Naive | Corrected | Result |
| --- | ---: | ---: | --- |
| Saw | −7.62 dBc | −12.06 dBc | 4.44 dB better |
| Pulse | −13.11 dBc | −18.98 dBc | 5.87 dB better |
| Triangle at fitted curvature | −27.34 dBc | −27.34 dBc | guarded fallback |

The variable-rate triangle BLAMP helped the linear-current ablation by about
1 dB but regressed the fitted high-curvature case. The runtime now falls back
to the stable naive fold above absolute curvature 0.5. This limitation is one
more reason not to promote the model.

## Cost and boundedness

One release CLI run at 48 kHz, 440 Hz saw, 262,144 measured samples reported:

| Model | ns/sample | Mutable bytes | Immutable bytes |
| --- | ---: | ---: | ---: |
| Production baseline | 35.64 | 352 | 0 |
| Plan 09 v2 | 41.36 | 104 | 4,320 |
| **Plan 12 gray-box** | **97.12** | **104** | **80** |
| Plan 11 measured wavetable | 230.96 | 1,232 | 8,957,952 |

The timing is a single-host comparison, not an embedded promotion benchmark.
The model is bounded and compact but about 2.4× the Plan 09 scalar cost while
being substantially less accurate.

Tests cover closed-form period normalization, fractional event timing against
the analytic continuous-time reference, maximum-frequency event bounds,
shared capacitor state across outputs, deterministic renders, hard-sync sample
accounting, diagnostic range, parameter persistence, correlated
curvature/comparator behavior, and pitch at 44.1/48/96 kHz. The research
integration suite passes 26/26.

## UI disposition

The registry, `--list` output, artifact provenance, parameter metadata, and CSV
diagnostic surface include the model. Live Params/Oscillator Lab audition was
not added: Plan 12 permits that adapter only after boundedness **and** its
retention rule requires beating the baseline and Plan 09. The candidate fails
that rule by a wide margin, so exposing it as a playable engine would imply a
status the evidence does not support.

## Reproduction

```bash
python3 plans/analog-osc/research/scripts/fit_gray_box_oscillator.py
cargo test -p synth-core --features oscillator-research gray_box --lib
RUST_MIN_STACK=16777216 cargo test -p synth-core --features oscillator-research --test analog_osc_research
cargo run --release -p synth-tools --bin analog_osc_research -- --list
cargo run --release -p synth-tools --bin analog_osc_research -- \
  --model korg-monologue-gray-box-saw-core-v1 --waveform saw \
  --frequency 997 --samples 256 --trace-csv /tmp/gray-box.csv
python3 plans/analog-osc/research/scripts/plot_gray_box_diagnostics.py \
  /tmp/gray-box.csv plans/analog-osc/research/plots/korg-monologue-gray-box-saw-core-v1.svg
```
