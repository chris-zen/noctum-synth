# Characterise a synth (oscillator-static)

**Start here** for the end-to-end oscillator characterisation loop:

virtual routing → MIDI Learn → capture → extract → wavetable bank.

This covers the **`oscillator-static-v1`** protocol only. Broader full-voice
protocols (filter, envelopes, control laws) are catalogued in
[`plans/analog-osc/16-full-voice-characterisation.md`](../../plans/analog-osc/16-full-voice-characterisation.md)
and are not required for a static wavetable bank.

Worked example: **Prophet-5 V** (`prophet5-v1`).

```text
devices → new → doctor → run → verify → extract → wavetable_bank
```

## Identity and attribution

- Prophet-5 V is **software**, not Sequential/Prophet hardware. Do not
  label banks or papers as hardware Prophet references.
- Static chromatic capture / cycle extraction follows the public Korg Monologue
  dataset by Simionato & Fasciani
  ([DOI 10.5281/zenodo.15196138](https://doi.org/10.5281/zenodo.15196138),
  CC-BY-4.0;
  [DAFx paper](https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf)).
  This tool does not redistribute their pickle files.

## 1. Prerequisites

| Need | Prophet5 example |
| --- | --- |
| Virtual MIDI port (exact name) | IAC Driver Bus 2 |
| Virtual audio loopback | BlackHole 2ch |
| Sample format | native **float32 @ 96 kHz**, input channel 0 |
| Absolute MIDI Learn | import [`Noctum-Characterisation.promidi`](Noctum-Characterisation.promidi) |
| OS permission | mic access for the loopback device (incl. IDE sandboxes) |

Mapping table:
[`prophet5-v1-mapping.md`](prophet5-v1-mapping.md).  
Target identity JSON:
[`prophet5-v1-target.json`](prophet5-v1-target.json).

Durable project location (kept under this checkout's `plans/` tree):

```text
plans/analog-osc/research/captures/arturia-prophet5-v1-r7
```

## 2. Operator setup (once per session)

When `doctor` / `run` prompts, set manually (not MIDI-mapped):

- Load the factory Init preset and import the current `.promidi`
- After reset, verify all three FX slots are bypassed with Dry/Wet at zero
- Osc 2 Fine Tune `0.000`
- Osc 2 Pulse Width `50%`
- Filter Env Amount `5.0`

## 3. Capture

From the repo root (use `--locked`):

```bash
cargo run --release -p synth-capture --locked -- devices

cargo run --release -p synth-capture --locked -- new \
  --project plans/analog-osc/research/captures/arturia-prophet5-v1-r7 \
  --target prophet5-v1 \
  --protocol oscillator-static-v1 \
  --midi-port "IAC Driver Bus 2" \
  --audio-device "BlackHole 2ch" \
  --input-channel 0 \
  --sample-rate 96000 \
  --plugin-version "YOUR_PLUGIN_VERSION"

cargo run --release -p synth-capture --locked -- doctor \
  --project plans/analog-osc/research/captures/arturia-prophet5-v1-r7

cargo run --release -p synth-capture --locked -- run \
  --project plans/analog-osc/research/captures/arturia-prophet5-v1-r7
```

Doctor now probes every waveform at MIDI 48, 64, and 80. It rejects
pitch-dependent spectral corruption before the 226-case capture begins.

Interrupt with Ctrl-C, then `run` again to resume. Completed WAVs are never
overwritten. Prefer `retry --failed` for bad takes.

```bash
cargo run --release -p synth-capture --locked -- verify \
  --project plans/analog-osc/research/captures/arturia-prophet5-v1-r7
```

Expect **226** complete cases (1 silence + 75 notes × 3 waves).

## 4. Extract cycles

```bash
cargo run --release -p synth-capture --locked -- extract \
  --project plans/analog-osc/research/captures/arturia-prophet5-v1-r7
```

Writes under `…/derived/`:

- `saw-cycles-v1.npz`, `triangle-cycles-v1.npz`, `pulse50-cycles-v1.npz`
- matching `*-summary-v1.json`

Fitting inputs: `median_cycles` (normalized) and `measured_frequency_hz`.

## 5. Build wavetable bank

```bash
cargo run --release -p synth-tools --locked --bin wavetable_bank -- \
  --derived-root plans/analog-osc/research/captures/arturia-prophet5-v1-r7/derived \
  --output-dir plans/analog-osc/research/banks
```

Defaults (if args are omitted) resolve from the current repository checkout,
never from a hard-coded `~/dev/analog-synth` path.

Outputs:

- `arturia-prophet5-measured-bank-v1.f32le` — little-endian f32 tables  
  layout: waveform × training pitch × 2048 samples (`saw`, `triangle`, `pulse50`)
- `arturia-prophet5-measured-bank-v1.json` — manifest (freqs, Nyquist limits,
  checksums, identity warning, prior-work DOI)
- `synth-core/src/voice/osc_engine/wavetable_banks/prophet5_profile.rs` —
  regenerated compile-time metadata beside the compiled bank binary

Policy notes:

- Source capture rate **96 kHz**; generated playback-bank reference **48 kHz**
  with Nyquist guard **0.45**
- Training rows: NPZ `role == 0` (scientific Training)
- Phase: extraction landmark (no Monologue “align to production source”)
- Adjacent training cycles must pass the phase-invariant spectral-coherence gate
  (`cosine >= 0.90`), otherwise generation stops with the failing pitches.

Harness install (desktop app + research binaries):

```bash
mkdir -p target/analog-osc/banks
cp plans/analog-osc/research/banks/arturia-prophet5-measured-bank-v1.f32le \
  target/analog-osc/banks/
```

Keep the existing Monologue bank beside it
(`korg-monologue-measured-bank-v1.f32le`). `synth-app` loads both and exposes
two combo entries: **Wavetable (Monologue)** and
**Wavetable (Prophet-5 V)**. Compile-time profile metadata for each bank lives
beside the compiled `.f32le` assets in
`synth-core/src/voice/osc_engine/wavetable_banks/`.

## 6. What “done” means

For **oscillator-static** characterisation you have:

1. Verified immutable WAV project  
2. Derived cycle NPZs + summaries  
3. Measured bank binary + manifest  

Next research steps (out of this runbook): held-out residual metrics, listening
gates, optional live adapter — see research plans 07 / 16. New synths need a
`SynthTarget` adapter only; protocols and this bank tool stay shared.
