# Korg Monologue Phase/Filter Ablation Listening Set v1

## Status

Generated, technically validated, and completed once by one anonymous listener
on 2026-07-27. The result identifies waveform-dependent behavior rather than a
single globally harmful component. This remains a diagnostic experiment, not a
promotion or final acceptance set.

The ignored listening package is at:

`target/analog-osc/listening/korg-monologue-phase-filter-ablation-v1/`

It contains 60 mono float32 WAV files and occupies approximately 44 MB. The
package is reproducible with
`plans/analog-osc/research/scripts/generate_target_conditioned_ablation_listening_set.py`.

## Question

The first blind session showed that the combined candidate was audible but
usually farther from the measured deterministic target than the production
baseline. This set asks which part caused that regression:

- unchanged production baseline;
- fitted phase deformation only;
- fitted output filtering only;
- fitted phase deformation plus output filtering.

The phase-only and filter-only variants use coefficients from the joint fit.
They isolate the contribution of each block, but they are not independently
refitted optimum models.

## Cases

The set uses two unused validation pitches per waveform. Selection was
deterministic and did not consult the first session's answers. Test-split
pitches remain untouched for a later fresh acceptance set.

| Waveform | Lower | Upper |
| --- | ---: | ---: |
| Saw | 174.545 Hz | 695.652 Hz |
| Triangle | 87.273 Hz | 347.826 Hz |
| Pulse (50%) | 87.432 Hz | 347.826 Hz |

## Listening procedure

Open `blind-target-ranking`, not `named` or `answer-key.json`. For each of the
six cases:

1. Listen to `reference.wav`.
2. Compare `choice-A.wav` through `choice-D.wav` at a fixed playback volume.
3. Rank all four choices from closest to farthest in
   `responses-template.json`, using each letter exactly once.
4. Add confidence and a short note when a difference is meaningful.

For example, `["C", "A", "D", "B"]` means C sounded closest and B farthest.
The reference is periodic interpolation of a normalized measured median cycle,
not a continuous hardware performance.

## Preparation and validation

- Sample rate: 48 kHz.
- Duration: 4 seconds per file.
- Level: independently matched to -18 dBFS centered RMS; DC is preserved.
- Fade: 20 ms raised-cosine fade at both ends.
- Cases: 6; files: 60.
- Observed post-fade centered RMS: 0.125481 to 0.125513.
- Maximum peak: 0.243040; no clipping.
- Every WAV is finite mono float32, hashes match the manifest, and every blind
  file is bit-identical to its keyed named source.
- Random seed: `20260728`.

## Evaluation rule

Report first-place counts and complete ranks per waveform and overall. A simple
rank score may summarize the six cases (3 points for closest through 0 for
farthest), but the individual case rankings remain primary because six cases
are not enough for a population-level claim.

- If phase-only ranks well while filter-only and combined rank poorly, revisit
  the filter fit.
- If filter-only ranks well while phase-only and combined rank poorly, revisit
  the phase map.
- If both isolated blocks rank well but the combination does not, investigate
  their interaction and joint gain/DC treatment.
- If the baseline remains dominant, close or substantially reformulate this
  compact model rather than increasing its order around the same objective.

Do not tune on these validation rankings. Any revision must be judged later on
the reserved test pitches with a newly randomized blind set.

## First listening result

Listener and playback-chain fields were left blank. Confidence averaged 0.79,
but the result cannot be generalized across listeners or playback systems.

### Revealed rankings

| Case | Closest to farthest | Confidence |
| --- | --- | ---: |
| Saw 174.545 Hz | baseline, phase-only, combined, filter-only | 0.80 |
| Saw 695.652 Hz | phase-only, baseline, filter-only, combined | 0.90 |
| Triangle 87.273 Hz | phase-only, filter-only, baseline, combined | 0.90 |
| Triangle 347.826 Hz | combined, filter-only, phase-only, baseline | 0.90 |
| Pulse 87.432 Hz | filter-only, combined, phase-only, baseline | 0.85 |
| Pulse 347.826 Hz | combined, filter-only, phase-only, baseline | 0.50 |

Here `combined` means phase plus filter. The response file remains unchanged in
the ignored package.

### Aggregate ranks

Rank score assigns 3 points to closest, 2 to second, 1 to third, and 0 to
farthest. It is a compact description, not an accuracy metric.

| Variant | First-place cases | Rank score (max 18) | Mean rank |
| --- | ---: | ---: | ---: |
| Phase-only | 2 | 11 | 2.17 |
| Filter-only | 1 | 10 | 2.33 |
| Combined | 2 | 9 | 2.50 |
| Baseline | 1 | 6 | 3.00 |

Six rankings do not establish a universal winner. An exploratory Friedman test
also finds no consistent global ordering (`p = 0.706`), which is unsurprising
given the small set and visibly waveform-dependent ranks.

### Waveform diagnosis

- **Saw:** baseline and phase-only tie for the best aggregate rank. Both
  filter-bearing variants tie for last. The fitted filtering is the clearest
  source of saw regression at these pitches; phase deformation alone is at
  least competitive with baseline.
- **Triangle:** phase-only and filter-only tie on aggregate score, while the
  winner changes from phase-only at 87 Hz to combined at 348 Hz. The baseline
  ranks third then fourth. This points to pitch-dependent phase/filter
  interaction rather than one block being uniformly wrong.
- **Pulse:** filter-only and combined tie for best aggregate rank; baseline is
  farthest in both cases. The fitted filter appears useful at these two
  validation pitches, while phase-only ranks third.

### Relationship to the first listening set

The earlier two-choice target match favored baseline over combined in 8 of 9
low/middle/high cases. In this four-way set, combined beats baseline in 3 of 6
interior-pitch cases: upper triangle and both pulse cases. The sets differ in
pitch and task format, so their counts must not be pooled. The reversal is
evidence that the current error is conditional on waveform and pitch, and that
a single global phase/filter interpretation would be misleading.

## Next decision

The full objective curves are complete and recorded in
`korg-monologue-phase-filter-ablation-curves-v1.md`. They show that fixed-phase
error over-rewards an arbitrary cycle origin and that harmonic-magnitude error
better follows the broad listening order, especially for saw. Keep the
candidate out of the live selector. Revise the v2 objective and refit from
training data; do not patch v1 coefficients from these six revealed rankings.
Any revision must pass a newly randomized blind test on the reserved test split.

## Reproduction

```text
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/generate_target_conditioned_ablation_listening_set.py
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_ablation_listening_set.py
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_ablations.py
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_ablations.py
```
