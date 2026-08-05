# Korg Monologue Phase/Filter Objective Ablation Curves v1

## Status and decision

Complete for all 54 validation cases: 18 pitches each for saw, triangle, and
pulse, with baseline, phase-only, filter-only, and combined renders.

The v1 candidate remains rejected for live promotion. The analysis found a
more fundamental problem than a single bad model block: the original objective
is too sensitive to the arbitrary point called the start of a waveform cycle,
and its baseline was a geometric waveform rather than the actual production
BLEP oscillator heard in the blind tests. Revise the objective and refit before
changing model topology.

Machine-readable results are in
`korg-monologue-phase-filter-ablation-curves-v1.json`. The complete curves are
in `../plots/korg-monologue-phase-filter-ablation-curves-v1.svg`.

## What was compared

Each compiled Rust variant was rendered at every validation pitch and compared
with the corresponding measured median cycle. The test split remains untouched.
All target-distance metrics are lower-is-better.

- **Native time NRMSE** compares samples directly and includes gain and DC.
- **Level-matched time NRMSE** removes DC and independently normalizes signal
  strength, matching the listening-file preparation more closely.
- **Phase-aligned time NRMSE** also ignores one whole-wave time shift. This
  preserves the waveform's internal shape while discarding where its cycle was
  arbitrarily declared to begin.
- **Complex-harmonic NRMSE** compares harmonic level and phase at the fixed
  trigger alignment. For complete periodic cycles it carries almost the same
  information as fixed-phase time NRMSE.
- **Harmonic-magnitude NRMSE** compares harmonic strengths while ignoring
  phase. It captures brightness more directly but cannot fully describe shape.
- **Non-harmonic residual** checks deterministic unwanted spectral energy. It
  is an implementation-quality warning, not a distance from the analog target.

## Critical phase-origin finding

The production baseline's best whole-wave alignment differs from the measured
cycle origin by approximately half a cycle for saw and pulse and one quarter of
a cycle for triangle. That is mainly a phase convention: on a sustained,
free-running oscillator, rotating the entire periodic wave in time does not
change its timbre.

Fixed-phase level-matched NRMSE therefore reports median baseline errors of
1.74 for saw, 1.40 for triangle, and 2.00 for pulse. After ignoring the global
cycle shift, those medians fall to 0.049, 0.051, and 0.104. The original metric
was rewarding the candidate for matching the capture trigger/cycle origin—an
inaudible property in this listening task—and substantially overstated the
audible advantage over production.

This does not mean harmonic phase is irrelevant. Relative phase between
harmonics changes waveform shape and can affect sound. Only the linear phase
term corresponding to moving the complete wave in time should be treated as a
nuisance variable.

## Validation medians

### Shape after ignoring global cycle phase

| Waveform | Baseline | Phase-only | Filter-only | Combined |
| --- | ---: | ---: | ---: | ---: |
| Saw | 0.0493 | 0.0580 | 0.0434 | 0.0407 |
| Triangle | 0.0507 | 0.0383 | 0.0225 | 0.0042 |
| Pulse | 0.1039 | 0.1042 | 0.0332 | 0.0336 |

The combined model retains a modest median shape advantage for saw and a large
objective advantage for triangle and pulse. It is no longer the dramatic
across-the-board improvement implied by fixed-phase error.

### Harmonic magnitudes

| Waveform | Baseline | Phase-only | Filter-only | Combined |
| --- | ---: | ---: | ---: | ---: |
| Saw | 0.0209 | 0.0241 | 0.0436 | 0.0425 |
| Triangle | 0.0311 | 0.0098 | 0.0223 | 0.0034 |
| Pulse | 0.0299 | 0.0299 | 0.0399 | 0.0399 |

For saw, both filter-bearing variants are roughly twice as far from the target
harmonic magnitudes as baseline. This agrees with the broad listening order,
where baseline and phase-only led and the filtered variants trailed. Pulse has
many pitch crossovers: its two listening pitches favor filtering, while the
median across all validation pitches does not. Triangle strongly favors the
combined model objectively but produced opposite listening outcomes at its two
diagnostic pitches.

### Non-harmonic residual

| Waveform | Baseline | Phase-only | Filter-only | Combined |
| --- | ---: | ---: | ---: | ---: |
| Saw | -82.67 dBc | -80.10 dBc | -84.69 dBc | -83.99 dBc |
| Triangle | -98.41 dBc | -98.59 dBc | -98.93 dBc | -98.77 dBc |
| Pulse | -85.43 dBc | -84.54 dBc | -88.55 dBc | -87.59 dBc |

The fitted filters generally reduce non-harmonic residual. Nothing here
explains the perceptual rejection as an obvious aliasing failure.

## Agreement with the six blind rankings

`Top agreement` counts cases where the metric's lowest-error variant was the
listener's first choice. Spearman correlation compares the complete four-way
ordering; `1` is identical, `0` has no ordering relationship, and `-1` is
reversed.

| Metric | Top agreement | Mean rank correlation | Median rank correlation |
| --- | ---: | ---: | ---: |
| Native time NRMSE | 4 / 6 | 0.30 | 0.50 |
| Level-matched time NRMSE | 4 / 6 | 0.33 | 0.60 |
| Phase-aligned time NRMSE | 3 / 6 | 0.40 | 0.80 |
| Complex-harmonic NRMSE | 4 / 6 | 0.33 | 0.60 |
| Harmonic-magnitude NRMSE | 3 / 6 | 0.67 | 0.80 |

Harmonic magnitude best tracks the overall ordering, while phase-aligned shape
is second. Fixed-phase metrics choose one more exact winner but order the saw
174.5 Hz case exactly backwards and have weaker overall correlation. Six cases
from one listener are too few to optimize metric weights; this is diagnostic
evidence only.

## Consequences for v2

1. Use the compiled production BLEP oscillator—not a geometric ideal—as the
   baseline in fitting and evaluation.
2. Normalize level consistently with the listening protocol before measuring
   deterministic shape.
3. Optimize away a single global cycle shift before time/complex-phase error.
   Keep relative harmonic phase; discard only the inaudible linear-delay term.
4. Add a harmonic-magnitude or perceptually weighted spectral term so the saw
   filter regression cannot hide behind trigger-phase improvement.
5. Fit and report saw, triangle, and pulse independently. Do not infer one
   global phase/filter amount from the six revealed validation rankings.
6. Refit only on training captures, use validation for model selection, and
   reserve the untouched test pitches for a newly randomized blind acceptance
   set.

This v2 objective and clean refit have now been implemented. See
`korg-monologue-phase-filter-v2.md`; no waveform-specific patch was made to the
v1 coefficients.

## Reproduction

```text
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_ablations.py
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_ablations.py
```
