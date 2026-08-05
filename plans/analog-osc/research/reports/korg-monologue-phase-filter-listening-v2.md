# Korg Monologue Phase + Filter Listening Set v2

## Status

Generated, technically validated, and completed once by one anonymous listener
on 2026-07-27. V2 does not pass the perceptual promotion gate: ABX performance
does not establish reliable discrimination, and the production baseline was
judged closer in 5 of 9 target matches versus 4 of 9 for v2.

The ignored listening package is at:

`target/analog-osc/listening/korg-monologue-phase-filter-v2/`

It contains 81 mono float32 WAV files, a manifest, an answer key, instructions,
and `responses-template.json`. The tracked generator is
`plans/analog-osc/research/scripts/generate_target_conditioned_listening_set_v2.py`.

## Acceptance question

Does the compiled phase-invariant v2 candidate sound closer than the unchanged
production oscillator to the measured deterministic Monologue cycle?

ABX first establishes whether the listener can hear baseline versus v2. The
target-match task then asks which source is closer to the measured reference.
This is not a general preference test and does not represent drift, noise, or
other cycle-to-cycle hardware behavior.

## Fresh case selection

Only the untouched test split is used. Pitch index 71 appeared in the earlier
v1 listening package and is explicitly excluded. The validation pitches used
for v1 diagnosis cannot enter this package.

| Waveform | Low | Middle | High |
| --- | ---: | ---: | ---: |
| Saw | 49.180 Hz (3) | 309.677 Hz (35) | 2000.000 Hz (67) |
| Triangle | 24.590 Hz (3) | 155.844 Hz (35) | 979.592 Hz (67) |
| Pulse (50%) | 24.590 Hz (3) | 155.844 Hz (35) | 979.592 Hz (67) |

Numbers in parentheses are source pitch indices. No coefficient may be tuned
against the revealed answers after this gate.

## Listening protocol

1. Keep `named/` and `answer-key.json` closed.
2. For each directory under `blind-abx/`, decide whether `X.wav` equals
   `A.wav` or `B.wav` and record the answer, confidence, and notes.
3. For each directory under `blind-target-match/`, compare `reference.wav`
   with `choice-A.wav` and `choice-B.wav`; record which choice is closer.
4. Save the completed values directly in `responses-template.json`.
5. Only then reveal the answer key and aggregate the result.

Use one playback chain and a stable listening level. Take breaks if the pure
tones become fatiguing.

## Preparation and technical validation

- Sample rate: 48 kHz.
- Duration: 4 seconds.
- Level: independently matched to -18 dBFS centered RMS.
- Fade: 20 ms raised cosine at both ends.
- Observed centered RMS after fades: 0.12539–0.12562.
- Maximum package peak: 0.29858; no clipping.
- Seed: `20260728`.
- Cases: 9, all from the test split.
- WAV files: 81/81 present, finite, mono float32, hash-verified.
- Blind X files and target-match choices match their answer-key sources by
  SHA-256 without exposing the mapping in this report.
- Compiled renderer SHA-256:
  `2a078ada3440a4eb4d89a1d2dae7f0578bf923d090dfd443ee9ad0508c7eea79`.

## Listening result

Listener and playback-chain fields were left blank, so this result must not be
generalized across listeners or reproduction systems.

### ABX discrimination

| Waveform | Correct | Trials | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 2 | 3 | 0.60 |
| Triangle | 2 | 3 | 0.95 |
| Pulse | 2 | 3 | 0.63 |
| **Overall** | **6** | **9** | **0.73** |

Under an independent 50/50 guessing model, the one-sided probability of at
least 6 correct answers out of 9 is `0.25391`. The session therefore does not
establish that baseline and v2 were reliably distinguishable. Each waveform's
2/3 result alone has probability `0.5` under the same model. The listener was
highly confident on triangle, but the high triangle ABX answer was incorrect;
confidence is not evidence of correctness by itself.

### Blind target matching

| Waveform | Baseline judged closer | V2 judged closer | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 2 | 1 | 0.05 |
| Triangle | 1 | 2 | 0.75 |
| Pulse | 2 | 1 | 0.58 |
| **Overall** | **5** | **4** | **0.46** |

Saw choices were explicitly reported as having no appreciable difference and
carry very low confidence. Triangle is the only promising subset: v2 was
selected for the middle and high cases, while the listener reported both
choices as very close—and clearly different from the reference—at the low
case. For pulse, v2 was selected only at the low pitch; the listener separately
preferred the other sound, correctly distinguishing preference from target
matching. The high pulse comparison was also reported as having no appreciable
difference.

The decoded per-case result and input hashes are recorded in
`korg-monologue-phase-filter-listening-v2.json`.

## Decision

Do not promote v2, add it to the live Params selector, or proceed to broad
dynamic sweeps. The objective shape improvements did not translate into a
clear overall perceptual target-match advantage. Retain the profile, runtime,
and report as a reproducible result and do not tune its coefficients against
these revealed answers.

Triangle remains a useful hypothesis for a later cross-candidate comparison,
but 2 of 3 target choices with no reliable ABX result is insufficient to spend
a separate optimization cycle on this model now. Close Plan 04 at v2 and move
to an independent oscillator topology.

## Reproduction

```text
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/generate_target_conditioned_listening_set_v2.py
python3 -m unittest plans/analog-osc/research/scripts/test_target_conditioned_listening_set_v2.py
python3 plans/analog-osc/research/scripts/analyze_target_conditioned_listening_set_v2.py
python3 -m unittest plans/analog-osc/research/scripts/test_analyze_target_conditioned_listening_set_v2.py
```
