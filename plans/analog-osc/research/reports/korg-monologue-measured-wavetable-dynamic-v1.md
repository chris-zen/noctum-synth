# Measured wavetable dynamic characterization v1

## Outcome

Retain and freeze the current measured-wavetable adapter as a **desktop
real-time experimental candidate**. It is deterministic and bounded in the
compact dynamic matrix, supports the requested shape/PWM, sync, slop, detune,
and glide paths, and has ample real-time margin on this desktop. This is not a
production or embedded promotion.

The first run caught a real PWM defect: the shifted-saw DC compensation used
the wrong sign, so narrow pulse widths could reach approximately 2.9 instead
of remaining near bipolar unity. The correction was fixed in both runtime and
sync-edge evaluation. Shape zero (the measured 50-percent pulse used by the
earlier blind set) was unaffected. The 120-case static shape sweep was rerun
after the fix and still has zero material residual failures.

## Dynamic source matrix

The source-only renderer executes each case three times at 48 and 192 kHz:

- logarithmic triangle pitch plus shape sweep;
- 110 Hz audio-rate pulse-width modulation;
- three fractional hard-sync ratios;
- combined pitch, PWM, and changing-master sync.

The 192 kHz render is low-pass filtered and resampled to 48 kHz. Its difference
from the native 48 kHz render is an alias/implementation-disagreement proxy,
not a pure alias measurement: intentional PWM and sync sidebands are part of
the wanted signal.

| Scenario | Baseline NRMSE | Measured NRMSE | Measured correlation |
| --- | ---: | ---: | ---: |
| Pitch + shape sweep | 0.00708 | 0.00602 | 0.999982 |
| Audio-rate PWM | 0.04215 | 0.01489 | 0.999855 |
| Hard-sync ratios | 0.22225 | 0.18150 | 0.983492 |
| Combined pitch/PWM/sync | 0.14393 | 0.12232 | 0.991037 |

The measured candidate has lower sample-rate disagreement than baseline in all
four cases. Hard sync remains the largest disagreement and should be treated as
the main dynamic limitation. Every source render is repeatable bit for bit.
At 48 kHz, the measured maximum raw peak is 1.443 and the maximum adjacent
sample step is 1.920; neither case is a runaway or non-finite result.

## Full synth desktop cost

The full synth runs through Pass Through in 64-frame blocks. Reported p99 is a
percentage of the 20.833 microsecond budget for one 48 kHz audio frame on this
host. It includes the voice, envelope, mixer, and output path, so it is not an
oscillator-only benchmark.

| Profile | Measured/baseline median | Measured p99 | 48 kHz frame budget |
| --- | ---: | ---: | ---: |
| Steady, one voice | 2.06x | 384 ns/frame | 1.84% |
| Steady, four voices | 2.33x | 851 ns/frame | 4.08% |
| One voice: PWM + sync + slop + detune + glide | 3.62x | 1,555 ns/frame | 7.46% |

All full-engine samples are finite. These results establish comfortable
desktop margin for the tested profiles, but not maximum-polyphony, OS audio
deadline, cache-soak, Daisy, or another embedded platform qualification.

## Decision and limits

- Keep the candidate playable and selectable for musical comparison.
- Freeze the bank and adapter behavior before comparing another oscillator
  family; do not tune it against these dynamic results.
- Do not claim target-accurate PWM or sync. The Monologue dataset contains only
  fixed-width, unsynchronized oscillator captures.
- Preserve hard sync as the first case to revisit if a real hardware reference
  or a stronger oversampled/minBLEP implementation becomes available.
- Defer maximum-polyphony soak and hardware placement until this candidate
  survives a cross-candidate listening decision.

## Reproduction

```text
cargo run --release -p synth-tools --bin analog_osc_dynamic
python3 scripts/evaluate_measured_wavetable_dynamic.py
python3 -m unittest scripts/test_measured_wavetable_dynamic.py
```

Machine-readable results are in
`korg-monologue-measured-wavetable-dynamic-v1.json`. Raw WAVs and timing input
remain under `target/analog-osc/dynamic-characterization-v1/`.
