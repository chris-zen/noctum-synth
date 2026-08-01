# synth-tools

Host-only binaries for performance checks, DSP measurements, and wavetable
prototyping against [`synth-core`](../synth-core). Not published; keeps
`synth-core` free of Cargo examples and host CLIs.

```bash
cargo run --release -p synth-tools --bin <name> [-- args...]
```

## Tools

| Binary | Purpose |
| --- | --- |
| [`filter_perf`](src/bin/filter_perf.rs) | Filter model timing (ns/sample-block, buffer estimates) |
| [`voice_block_perf`](src/bin/voice_block_perf.rs) | `VoiceBlock` render timing for neutral / active / modulation / self-osc cases |
| [`filter_measurements`](src/bin/filter_measurements.rs) | Deterministic per-model gain, slope, and self-oscillation metrics |
| [`sample_rate_quality`](src/bin/sample_rate_quality.rs) | Offline spectral CSV for candidate sample rates (oscillator / filter / effects) |
| [`generate_wavetable_bank`](src/bin/generate_wavetable_bank.rs) | Write retained **ideal** f32 (and Q15 comparison) wavetable bank files |
| [`measured_wavetable_bank`](src/bin/measured_wavetable_bank.rs) | Build **measured** pitch-conditioned bank from `synth-capture` NPZs |
| [`wavetable_listening_samples`](src/bin/wavetable_listening_samples.rs) | Short WAV listening samples for the wavetable prototype report |
| [`factory_corpus_acceptance`](src/bin/factory_corpus_acceptance.rs) | Full 512-program, two-layer topology/filter acceptance with per-program CSV |
| [`export_rev2_patches`](src/bin/export_rev2_patches.rs) | Decode Rev2 Program Data SysEx into schema v1 two-layer patch JSON |

### `filter_perf`

```bash
cargo run --release -p synth-tools --bin filter_perf
cargo run --release -p synth-tools --bin filter_perf -- <filter-model-name>
```

Optional first argument limits the run to one `FilterType` name.

### `voice_block_perf`

```bash
cargo run --release -p synth-tools --bin voice_block_perf
```

### `filter_measurements`

```bash
cargo run --release -p synth-tools --bin filter_measurements
```

### `sample_rate_quality`

```bash
cargo run --release -p synth-tools --bin sample_rate_quality
```

Prints CSV to stdout (alias / fundamental / image metrics across rates).

### `generate_wavetable_bank`

```bash
cargo run --release -p synth-tools --bin generate_wavetable_bank
cargo run --release -p synth-tools --bin generate_wavetable_bank -- <output-directory>
```

Default output directory: `target/wavetable-prototype`.

### `measured_wavetable_bank`

Builds an Arturia-first measured bank from extract NPZs (`median_cycles`,
training `role == 0`, 96 kHz Nyquist guard). End-to-end capture→bank runbook:
[`../synth-capture/docs/characterise-a-synth.md`](../synth-capture/docs/characterise-a-synth.md).

```bash
cargo run --release -p synth-tools --locked --bin measured_wavetable_bank -- \
  --derived-root ~/dev/analog-synth/plans/analog-osc/research/captures/arturia-prophet5-v1/derived \
  --output-dir ~/dev/analog-synth/plans/analog-osc/research/banks
```

Writes `{profile-id}.f32le` and `{profile-id}.json` (default profile
`arturia-prophet5-measured-bank-v1`). Distinct from `generate_wavetable_bank`,
which synthesizes ideal (non-measured) tables.

### `wavetable_listening_samples`

```bash
cargo run --release -p synth-tools --bin wavetable_listening_samples
```

Writes WAVs under `plans/wavetable-listening/`.

### `factory_corpus_acceptance`

```bash
cargo run --release -p synth-tools --bin factory_corpus_acceptance -- \
  Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx \
  target/factory-corpus.csv
```

Runs constrained Layer A/B and stored Normal/Stack/Split scenarios through the
Gain-Limited TPT and Huovilainen filters. It validates the official mode
distribution and four documented factory regressions, then writes peak/RMS,
active-layer voices, non-finite count, limiter engagement, names, and host
callback timing for every program.

### `export_rev2_patches`

```bash
cargo run --release -p synth-tools --bin export_rev2_patches -- <input.syx> [output-dir]
```

Reads a Rev2 Program Data `.syx` byte stream (`F0`…`F7` framing), decodes each
Program Data message, and writes schema version 1 two-layer patch JSON
(`mode`, `split_point`, `layers.a` / `layers.b`). Filenames follow the desktop
MIDI import convention (`F1-001-Name.json`, `U1`–`U4` for user banks).

Default `output-dir` is the Noctum patches directory
(`~/Library/Application Support/Noctum/patches` on macOS).
