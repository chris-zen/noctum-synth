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
| [`wavetable_bank`](src/bin/wavetable_bank.rs) | Build **measured** pitch-conditioned bank from `synth-capture` NPZs |
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

### `wavetable_bank`

Builds schema-v2 Monologue or Prophet measured banks from extract NPZs. Every
pitch knot receives the universal 33-level harmonic mip hierarchy; capture
sample rate is provenance, while runtime safety uses the 0.45 guard and actual
phase increment. The tool rejects incoherent adjacent training cycles and
reconstructs every mip directly from the original complex spectrum. Tables use
a 256-sample interpolation floor and bounded mid-mip margin so cubic runtime
resampling does not reintroduce material images. End-to-end
capture→bank runbook:
[`../synth-capture/docs/characterise-a-synth.md`](../synth-capture/docs/characterise-a-synth.md).

```bash
cargo run --release -p synth-tools --locked --bin wavetable_bank -- \
  --bank prophet5

cargo run --release -p synth-tools --locked --bin wavetable_bank -- \
  --bank monologue
```

Writes the ignored reproducible `{profile-id}.f32le`, committed v2 manifest,
and generated Rust profile metadata (disable the latter with
`--no-rust-output`). Defaults select `prophet5-wavetable-bank-v2`; use
`--bank monologue` for `korg-monologue-measured-wavetable-v2`. This is distinct from
`generate_wavetable_bank`, which synthesizes ideal (non-measured) tables.

### `wavetable_listening_samples`

```bash
cargo run --release -p synth-tools --bin wavetable_listening_samples
```

Writes WAVs under `plans/wavetable-listening/`.

### `wavetable_multirate_benchmark`

Benchmarks compiled measured wavetable banks through the full Pass Through synth
engine with 1, 4, and 16 active voices at 44.1, 48, 96, and 192 kHz, then runs
the 48 kHz sixteen-voice mip/slop soak for each selected bank.

```bash
cargo run --release -p synth-tools --bin wavetable_multirate_benchmark -- --bank all
```

`--bank` accepts `all` (default), `monologue`, or `prophet5`. Use `--output
<file>` to change the JSON path or `--soak-seconds <seconds>` for a shorter
diagnostic run. The qualification default is 60 seconds. Schema version 2
reports one entry per bank under `banks`. The soak uses paced real-time blocks
and macOS user-interactive QoS when available. Qualification requires finite
output and p99 below 50% of the block deadline; maximum render time and overrun
counts remain scheduler-noise diagnostics.

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
