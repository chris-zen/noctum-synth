# Multi-rate measured wavetable v2 implementation report

Date: 2026-08-02  
Plan: 11  
Disposition: **Monologue + Prophet v2 banks built; combined held-out / alias / soak gates still open**

## Implemented representation

The v2 bank layout is `waveform → mip → pitch → samples`. All pitch knots use
the universal 33-level hierarchy from harmonic 1023 down to 1. Table lengths
are `max(64, next_power_of_two(2 * (limit + 1)))`, producing 16,704 samples per
pitch and waveform. Each mip is reconstructed independently from the original
measured complex spectrum; lower mips are not resampled from richer tables.

The runtime calculates `floor(0.45 / phase_increment)` after detune, glide,
pitch modulation, and slop. A build-generated 1,024-entry lookup supplies the
rich/lean mip indices and log-space blend without runtime logarithms. Both
mips are safe. Pitch interpolation remains control-rate, while mip selection
tracks every effective frequency update. The same pair feeds shape morphing,
SawTri, pulse/PWM, and hard-sync edge evaluation.

Within the characterized range output is fully measured. The final character
table is clamped over the next semitone while measured output crossfades to
BLEP; higher notes report `AboveCapturedRange`. Invalid frequencies and a
fundamental above the 0.45 guard have separate statuses. Osc Design surfaces
only the transition/out-of-range warnings.

## Monologue v2 artifact

- Profile: `korg-monologue-measured-wavetable-v2`
- Manifest content SHA-256:
  `c4b0eb29857dea92d764a815fc994a6ecc7a2aacf272113dceadb7bdf3deffa0`
- Binary SHA-256:
  `dc1f6c1a342462b46fd055438953de3c3041291aa228206be567bf5f3aff6c0c`
- FNV-1a: `0x4b4bbd0b`
- Samples: 1,804,032 f32
- Bytes: 7,216,128 (6.88 MiB)
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

## Verification completed

- Generator unit tests validate the hierarchy and power-of-two lengths,
  deterministic binary/manifest output, incoherent-source rejection, and a
  phase-rich synthetic cycle whose legal complex bins survive while all bins
  above the declared mip limit are removed.
- Runtime unit tests sweep safe harmonic selection, validate the one-semitone
  status/blend boundary, and exercise live slop, PWM, shape, and fallback paths.
- The Monologue reports measured support for saw, SawTri, triangle, and pulse at
  representative low/mid/high frequencies at 24, 44.1, 48, 96, and 192 kHz.
- OscillatorPreview and the live retained engine are sample-identical at 44.1,
  48, 96, and 192 kHz.
- The BLEP engine remains a separate retained owner and its existing
  bit-identity tests are unchanged.

## Runtime measurements

The release `wavetable_multirate_benchmark` renders the full synth through Pass
Through with a 64-frame block. Each case below uses 2,000 measured blocks on
this host; values are p99 fractions of one audio-frame budget.

| Rate | 1 voice | 4 voices | 16 voices |
| --- | ---: | ---: | ---: |
| 44.1 kHz | 3.63% | 8.61% | 15.37% |
| 48 kHz | 2.92% | 6.17% | 10.60% |
| 96 kHz | 5.57% | 10.74% | 21.54% |
| 192 kHz | 11.54% | 22.75% | 43.07% |

The 48 kHz sixteen-voice p99 gate is below its 50% limit. All cases remained
finite. The unprioritized desktop 60-second-audio-time soak remained finite but
recorded three wall-clock deadline overruns.
Because the no-miss requirement was not met reproducibly, the soak gate remains
open rather than being waived as host scheduling noise. The complete JSON is
reproducible at `target/analog-osc/multirate-wavetable/runtime-v2.json`.

## Reproduction

```bash
python3 scripts/analog_osc_reference.py download --waveform all
python3 scripts/analog_osc_reference.py extract --waveform all
cargo run --release -p synth-tools --bin wavetable_bank -- --bank monologue
python3 scripts/embed_wavetable_banks.py --bank monologue
cargo run --release -p synth-tools --bin wavetable_multirate_benchmark
```

## Prophet v2 artifact

- Profile: `prophet5-wavetable-bank-v2`
- Manifest content SHA-256:
  `e47ab310386a9b922b1afd5f7d757930e92137f59b078485ce7689f07d42f235`
- Binary SHA-256:
  `27541543f303586646895a691fcc6075acf41129de93ae54d62e54a66b17a937`
- FNV-1a: `0xc1f3dd25`
- Samples: 1,854,144 f32
- Bytes: 7,416,576 (7.07 MiB)
- Pitch knots: 37 per waveform
- Source capture rate: 96 kHz, provenance only
- Source project: `arturia-prophet5-v1-r7` (adapter revision 7, 226 verified cases)

Combined compiled Monologue + Prophet assets: 14,632,704 bytes (under 20 MiB).

## Open gates

Prophet NPZs and the v2 bank are now present. Remaining Plan 11 qualification:

- both-bank rate/domain/safety matrix;
- 48/96 kHz held-out target metrics and material alias sweeps;
- a zero-miss 60-second mip/slop deadline soak on an audio-priority thread.

No final Prophet target-quality or soak claim is made until those gates are
complete.
