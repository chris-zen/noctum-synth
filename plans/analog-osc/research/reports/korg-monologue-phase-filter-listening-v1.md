# Korg Monologue Phase + Filter Listening Set v1

## Status

Generated, technically validated, and completed once by one anonymous listener
on 2026-07-27. The candidate was reliably distinguishable in this set, but it
failed the perceptual target-match gate: the production baseline was judged
closer to the measured deterministic reference in 8 of 9 cases.

The generated package is intentionally stored under ignored build artifacts:

`target/analog-osc/listening/korg-monologue-phase-filter-v1/`

It contains 81 mono float32 WAV files, a public manifest, a response template,
and a separate answer key. The package is reproducible with
`scripts/generate_target_conditioned_listening_set.py` and is not committed
because the WAV files occupy about 62 MB.

## Listening cases

All cases use held-out reference pitches that were not used as fit knots.

| Waveform | Low | Middle | High |
| --- | ---: | ---: | ---: |
| Saw | 43.597 Hz | 347.826 Hz | 2400.000 Hz |
| Triangle | 21.583 Hz | 174.545 Hz | 1263.158 Hz |
| Pulse (50%) | 21.858 Hz | 174.545 Hz | 1263.158 Hz |

The high saw case deliberately retains the known static-residual warning. It
must be heard rather than omitted from the comparison.

## Sources

Each case provides:

- `baseline`: the unchanged production table-BLEP oscillator adapter.
- `candidate`: the target-conditioned phase/filter oscillator.
- `measured_target`: periodic interpolation of the independently normalized
  measured Monologue median cycle at that pitch.

`measured_target` is useful for deterministic waveform matching, but it is not
a continuous raw hardware recording. It does not reproduce analog drift,
capture noise, or cycle-to-cycle variation and must not be presented as such.

## Level and file preparation

- Sample rate: 48 kHz.
- Duration: 4 seconds per file.
- Level: independently matched to -18 dBFS centered RMS. DC is excluded from
  the level measurement but otherwise preserved in the waveform.
- Fade: 20 ms raised-cosine fade at both ends.
- Observed post-fade centered RMS: 0.12544 to 0.12559 linear, close to the
  pre-fade target of 0.12589.
- Maximum peak across the package: 0.34461; no clipping.
- Every file's SHA-256, RMS, peak, source case, profile checksum, binary hash,
  and Git state are recorded in `manifest.json`.

Matching RMS prevents a louder oscillator from appearing automatically better.
It does not force equal brightness, fundamental amplitude, or peak level; those
differences are part of the timbre under test.

## How to listen

Use the `blind-abx` directory first. For each case:

1. Listen to `A.wav` and `B.wav` until their difference is understood.
2. Listen to `X.wav` and record whether it equals A or B.
3. Record confidence and brief notes in `responses-template.json`.

Then use `blind-target-match`:

1. Listen to `reference.wav`.
2. Compare `choice-A.wav` and `choice-B.wav` at the same playback volume.
3. Record which choice sounds closer to the reference and why.

Do not open `answer-key.json` until the answers have been recorded. The `named`
directory is available afterward for unblinded inspection. Use the same
headphones/speakers and listening level for the complete session. Short breaks
are preferable to continuing after fatigue.

ABX answers whether baseline and candidate are audibly distinguishable.
Target-match answers which is closer to this measured deterministic reference.
Neither question is the same as personal preference, which should be recorded
separately if desired.

## Technical validation

- Nine low/middle/high held-out cases were generated deterministically with
  seed `20260727`.
- All 81 referenced WAVs exist, are finite mono float32 at 48 kHz, and match
  their recorded hashes.
- Blind X files are bit-identical to the answer-key A or B source.
- Randomized choice mappings are reproducible from the fixed seed.
- Analytic tests cover RMS matching, fades, periodic target frequency, and
  held-out case selection.

## First listening result

Listener and playback-chain fields were left blank, so this result must not be
generalized across listeners or reproduction systems.

### ABX discrimination

| Waveform | Correct | Trials | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 3 | 3 | 0.17 |
| Triangle | 3 | 3 | 0.78 |
| Pulse | 3 | 3 | 0.88 |
| **Overall** | **9** | **9** | **0.61** |

All nine forced choices were correct. Under an independent 50/50 guessing
model, the one-sided probability of 9 correct guesses out of 9 is
`1 / 512 = 0.00195`. This is strong evidence that this listener could hear a
difference across the complete set. It is not nine independent replications of
each waveform: each waveform has only three cases, and all cases came from one
listener in one session. In particular, the three correct saw answers alone
are not strong evidence (`p = 0.125`), and their mean confidence was low. The
listener reported no appreciable difference for the low and middle saw and
hardly any difference for the high saw.

### Blind target matching

| Waveform | Baseline judged closer | Candidate judged closer | Mean confidence |
| --- | ---: | ---: | ---: |
| Saw | 2 | 1 | 0.60 |
| Triangle | 3 | 0 | 0.63 |
| Pulse | 3 | 0 | 0.67 |
| **Overall** | **8** | **1** | **0.63** |

Target matching is a closer-sound judgement, not a right/wrong test. The result
is nevertheless directionally strong: the candidate was chosen only for the
43.597 Hz saw, while the unchanged production baseline was chosen for every
triangle and pulse case and for the other two saw cases. With one listener and
one judgement per case this is descriptive evidence, not a population-level
preference statistic.

### Interpretation

The objective median-cycle metrics and the first perceptual result disagree.
The fitted phase/filter transformation moves the waveform enough to be heard,
but those changes do not sound closer to the periodic measured reference in
this test. The candidate therefore does not satisfy Plan 04's perceptual
acceptance rule and must not be promoted to the live Params selector.

The response file remains unchanged in the ignored listening package. This
report records the aggregate result without inventing listener or playback
metadata.

## Next decision

Do not add this candidate to the live Params selector. Generate a smaller blind
diagnostic set containing baseline, phase-only, filter-only, and phase-plus-
filter variants. That test should identify which fitted component causes the
perceptual regression before more dynamic-sweep or live-adapter work is spent
on this exact model. Keep the current profile as a reproducible negative result;
do not tune against these nine held-out listening choices.

## Reproduction

```text
cargo build --release -p synth-tools --bin analog_osc_research
python3 scripts/generate_target_conditioned_listening_set.py
python3 -m unittest scripts/test_target_conditioned_listening_set.py
```
