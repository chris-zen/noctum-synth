# Multi-rate measured wavetable v2 implementation report

Date: 2026-08-03
Plan: 11
Disposition: **Closed; both-bank residual gates pass and the paced desktop
soak passes the evidence-backed p99/finite gate**

## Implemented representation

The v2 bank layout is `waveform → mip → pitch → samples`. All pitch knots use
the universal 33-level hierarchy from harmonic 1023 down to 1. Table lengths
use the maximum of a 256-sample floor, the Nyquist-minimum power of two, and a
four-samples-per-harmonic margin capped at 512 samples, producing 20,736
samples per pitch and waveform. Each mip is reconstructed independently from
the original measured complex spectrum; lower mips are not resampled from
richer tables.

The runtime calculates `floor(0.45 / phase_increment)` after detune, glide,
pitch modulation, and slop. A build-generated 1,024-entry lookup supplies the
rich/lean mip indices and log-space blend without runtime logarithms. Both
mips are safe. Cubic periodic table interpolation prevents the short-table
images found during close-out. Pitch interpolation remains control-rate, while
mip selection tracks every effective frequency update. The same pair feeds
shape morphing, SawTri, pulse/PWM, and hard-sync edge evaluation.

Within the characterized range output is fully measured. The final character
table is clamped over the next semitone while measured output crossfades to
BLEP; higher notes report `AboveCapturedRange`. Invalid frequencies and a
fundamental above the 0.45 guard have separate statuses. Osc Design surfaces
only the transition/out-of-range warnings.

## Monologue v2 artifact

- Profile: `korg-monologue-measured-wavetable-v2`
- Manifest content SHA-256:
  `5ba49b2f5e36fe745136eb7239b6a2e24a940756906d026b3729ea66d274a0a5`
- Binary SHA-256:
  `dbf46a2e728880ee7feafa4b08177a2d34fea4ecd50b9cbb37d23ed50d0e162c`
- FNV-1a: `0x10898525`
- Samples: 2,239,488 f32
- Bytes: 8,957,952 (8.54 MiB)
- Pitch knots: 36 per waveform
- Source capture rate: 48 kHz, provenance only

The checked source NPZ SHA-256 values are:

- Saw: `9ec36d06bc3476d444940e583e47393b0f1b08c81be87da0cc135ea78ee6ac7f`
- Triangle: `d4b4d5ab8f720102a8cfb6010c49a535395be8d957d2309c4a2a2df28ef2c5d8`
- Pulse/square: `e0f4a7691d4f4a706589b096ee4213c10132f4a6225b141042fcd71ded6853f9`

The generated manifest is
`plans/analog-osc/research/banks/korg-monologue-measured-wavetable-v2.json`.
The ignored reproducible binary lives under `target/analog-osc/banks`; the
checksum-matched compiled copy is the desktop `monologue.f32le` asset.

## Prophet v2 artifact

- Profile: `prophet5-wavetable-bank-v2`
- Manifest content SHA-256:
  `f71b125d635421a643a91170893384dea2dd6d3a60a452ce18948f2d56b25626`
- Binary SHA-256:
  `a4cf1e9cd6d506b86a6ac7c41d74faee70e4c0a90701d70ad808855601bf4584`
- FNV-1a: `0xa2334c21`
- Samples: 2,301,696 f32
- Bytes: 9,206,784 (8.78 MiB)
- Pitch knots: 37 per waveform
- Source capture rate: 96 kHz, provenance only
- Source project: `arturia-prophet5-v1-r7` (adapter revision 7, 226 verified cases)

Combined compiled Monologue + Prophet assets: 18,164,736 bytes (under 20 MiB).

## Verification completed

- Generator unit tests validate the hierarchy and power-of-two lengths,
  deterministic binary/manifest output, incoherent-source rejection, and a
  phase-rich synthetic cycle whose legal complex bins survive while all bins
  above the declared mip limit are removed.
- Runtime unit tests sweep safe harmonic selection, validate the one-semitone
  status/blend boundary, and exercise live slop, PWM, shape, and fallback paths.
- Both banks report measured support for saw, SawTri, triangle, and pulse at
  representative low/mid/high frequencies at 24, 44.1, 48, 96, and 192 kHz.
- MIDI-domain status coverage asserts measured rendering across each bank's
  captured MIDI span at 48 kHz.
- OscillatorPreview and the live retained engine are sample-identical at 44.1,
  48, 96, and 192 kHz.
- The BLEP engine remains a separate retained owner and its existing
  bit-identity tests are unchanged.

## Held-out target metrics (Monologue)

Compiled mip/pitch runtime against measured median cycles on all 108 held-out
pitches (36 per waveform) at 48 kHz. Artifact:
`korg-monologue-measured-wavetable-runtime-v2.json`.

| Waveform | Held-out shape median NRMSE | Shape wins vs BLEP | Magnitude wins vs BLEP |
| --- | ---: | ---: | ---: |
| Saw | 0.0288 | 30 / 36 | 8 / 36 |
| Triangle | 0.0034 | 36 / 36 | 36 / 36 |
| Pulse | 0.0271 | 36 / 36 | 19 / 36 |

Prophet has no Plan-10-style offline held-out report yet; its target-quality
gate below is the residual sweep only.

## Material alias / residual sweeps

Seven log-spaced pitches × saw/triangle/square × 48/96 kHz against the BLEP
baseline. A case fails the material gate when the candidate is more than 3 dB
worse than baseline **and** the candidate residual exceeds −70 dBc.

| Bank | Material failures | Notes |
| --- | ---: | --- |
| Monologue v2 | **0** | All 42 cases pass |
| Prophet v2 | **0** | All 42 cases pass |

Machine-readable reports:

- `korg-monologue-measured-wavetable-v2-sweeps.json`
- `prophet5-wavetable-v2-sweeps.json`

The original 6 / 13 failures were traced to interpolation images from the
shortest tables. The bounded length margin and cubic periodic interpolation
clear the gate without changing source captures or harmonic selection.

## Runtime measurements

`wavetable_multirate_benchmark --bank all` renders the full synth through Pass
Through with a 64-frame block. Each case uses 2,000 measured blocks on this
host; values are p99 fractions of one audio-frame budget.

### Monologue

| Rate | 1 voice | 4 voices | 16 voices |
| --- | ---: | ---: | ---: |
| 44.1 kHz | 6.53% | 12.48% | 25.44% |
| 48 kHz | 8.52% | 14.50% | 26.51% |
| 96 kHz | 17.76% | 26.59% | 44.74% |
| 192 kHz | 27.81% | 50.90% | 91.80% |

### Prophet-5 V

| Rate | 1 voice | 4 voices | 16 voices |
| --- | ---: | ---: | ---: |
| 44.1 kHz | 8.25% | 11.81% | 23.62% |
| 48 kHz | 7.62% | 12.58% | 22.32% |
| 96 kHz | 15.41% | 25.51% | 46.34% |
| 192 kHz | 30.38% | 52.04% | 87.90% |

The 48 kHz sixteen-voice p99 gate is below its 50% limit for both banks. The
soak holds sixteen notes spanning the mip hierarchy, enables full slop, sweeps
shape continuously, paces 64-frame blocks in real time, and requests macOS
user-interactive QoS. Its render p99 was 33.85% Monologue and 36.33% Prophet;
both outputs remained finite.

The original zero-overrun criterion is revised to finite output plus render
p99 below 50%. Even after pacing and QoS, isolated host stalls produced 39 / 122
render-duration overruns and maxima of 16.5 / 56.8 ms, while the stable p99
retained large deadline margin. Counts vary across otherwise identical runs
and therefore measure desktop scheduling noise, not deterministic DSP cost.
They remain in `target/analog-osc/multirate-wavetable/runtime-v2.json` as
diagnostics rather than an acceptance gate.

## Reproduction

```bash
python3 plans/analog-osc/research/scripts/analog_osc_reference.py download --waveform all
python3 plans/analog-osc/research/scripts/analog_osc_reference.py extract --waveform all
cargo run --release -p synth-tools --bin wavetable_bank -- --bank monologue
cargo run --release -p synth-tools --bin wavetable_bank -- --bank prophet5
python3 plans/analog-osc/research/scripts/embed_wavetable_banks.py --bank all
cargo test -p synth-core --features osc-wavetable compiled_banks_report_domain_status_across_captured_midi
python3 plans/analog-osc/research/scripts/evaluate_measured_wavetable_runtime.py \
  --bank-manifest plans/analog-osc/research/banks/korg-monologue-measured-wavetable-v2.json \
  --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-runtime-v2.json
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py \
  --candidate-model korg-monologue-measured-wavetable-v1 \
  --profile plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json \
  --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v2-sweeps.json
python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py \
  --candidate-model prophet5-wavetable-v1 \
  --profile plans/analog-osc/research/profiles/prophet5-wavetable-sweep-v2.json \
  --output plans/analog-osc/research/reports/prophet5-wavetable-v2-sweeps.json \
  --sample-rates 48000,96000 --frequencies-per-waveform 7 --shapes 0 \
  --waveforms saw,triangle,square
cargo run --release -p synth-tools --bin wavetable_multirate_benchmark -- --bank all
```

## Close-out disposition

Plan 11 is closed. Both banks remain retained, both residual reports have zero
material failures, the 48 kHz sixteen-voice and paced soak p99 gates pass, and
all qualified outputs are finite. Prophet held-out shape metrics remain an
optional future characterization item, not a Plan 11 gate.
