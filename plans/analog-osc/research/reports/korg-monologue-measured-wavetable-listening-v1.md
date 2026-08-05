# Korg Monologue Measured-Wavetable Listening Set v1

## Status

Generated, technically validated, and completed once by one anonymous listener
on 2026-07-27. The result supports continued desktop experimentation but does
not prove perceptual promotion: ABX was 6/9, while target matching selected the
measured-wavetable candidate in 6/9 cases. The three saw choices carried only
0.1 confidence and were explicitly reported as indistinguishable.

The ignored listening package is at
`target/analog-osc/listening/korg-monologue-measured-wavetable-v1/`. The tracked
generator and decoder are `plans/analog-osc/research/scripts/generate_measured_wavetable_listening_set.py`
and `plans/analog-osc/research/scripts/analyze_measured_wavetable_listening_set.py`.

## Acceptance question

Does the compiled measured-wavetable runtime sound closer than the unchanged
production oscillator to the measured deterministic Monologue cycle?

ABX first asks whether the two oscillators can be distinguished. Target matching
then asks which one is closer to the measured reference. This is neither a
general preference test nor a model of hardware drift and noise.

## Fresh case selection

Only test rows absent from both Plan 04 listening packages were eligible. The
bank contains even-indexed training rows; every case below is an odd-indexed
held-out row.

| Waveform | Low | Middle | High |
| --- | ---: | ---: | ---: |
| Saw | 61.776 Hz (7) | 390.244 Hz (39) | 1548.387 Hz (63) |
| Triangle | 30.908 Hz (7) | 195.918 Hz (39) | 786.885 Hz (63) |
| Pulse (50%) | 30.888 Hz (7) | 196.721 Hz (39) | 774.194 Hz (63) |

Numbers in parentheses are source pitch indices. Bank tables and harmonic guards
were frozen before revealing the answers.

## Technical validation

- Sample rate: 48 kHz.
- Duration: 4 seconds.
- Level: independently matched to -18 dBFS centered RMS.
- Fade: 20 ms raised cosine at both ends.
- Observed centered RMS after fades: 0.12547–0.12552.
- Maximum package peak: 0.27755; no clipping.
- Seed: `20260729`.
- Cases: 9, all from the held-out test split.
- WAV files: 81/81 present and SHA-256 verified.
- Compiled renderer SHA-256:
  `98fd49ef5a4d8b41e538ca9c3aba28931557cbd094ae48b3d17ff5a7d02a3a50`.

## Listening result

Listener and playback-chain fields were left blank. This single session must
not be generalized across listeners or reproduction systems.

### ABX discrimination

| Waveform | Correct | Trials | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 2 | 3 | 0.10 |
| Triangle | 3 | 3 | 0.96 |
| Pulse | 1 | 3 | 0.60 |
| **Overall** | **6** | **9** | **0.55** |

Under an independent 50/50 guessing model, the one-sided probability of at
least 6 correct answers out of 9 is `0.25391`. The session therefore does not
establish reliable overall discrimination. Triangle was consistently and
confidently distinguishable; saw was explicitly reported as identical; pulse
was inconsistent despite high confidence at its endpoints.

### Blind target matching

| Waveform | Baseline judged closer | Measured candidate judged closer | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 0 | 3 | 0.10 |
| Triangle | 1 | 2 | 0.75 |
| Pulse | 2 | 1 | 0.65 |
| **Overall** | **3** | **6** | **0.50** |

The 6/9 aggregate is not strong evidence by itself because all three saw choices
were declared indistinguishable and effectively arbitrary. Among the six
triangle and pulse trials, the models split 3/3. Triangle is the strongest
candidate subset; pulse remains unresolved or baseline-favoring.

The decoded responses and immutable input hashes are recorded in
`korg-monologue-measured-wavetable-listening-v1.json`.

## Decision

Advance the unchanged candidate to bounded runtime-cost and dense
pitch-transition stress testing. This is an exploratory continuation justified
by its strong objective held-out result and modest 6/9 target-match result, not
a production-promotion decision. Do not tune this bank against the revealed
answers.

The remaining budget was redirected from exhaustive characterization to the
higher-value musical test. A focused live-engine render/selection smoke test
passes, and the candidate is exposed through the existing desktop oscillator
dropdown. At the time of this blind gate, pulse width, morph, hard sync, and
embedded deployment remained outside the first live adapter. Later Plan 07
increments added shape/PWM and sub-sample hard-sync compatibility without
changing the bank or tuning against these revealed answers.

## Reproduction

```text
cargo build --release -p synth-tools --bin analog_osc_research
python3 plans/analog-osc/research/scripts/generate_measured_wavetable_listening_set.py
python3 -m unittest plans/analog-osc/research/scripts/test_measured_wavetable_listening_set.py
# Complete responses-template.json before decoding.
python3 plans/analog-osc/research/scripts/analyze_measured_wavetable_listening_set.py
```
