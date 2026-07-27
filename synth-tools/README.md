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
| [`generate_wavetable_bank`](src/bin/generate_wavetable_bank.rs) | Write retained f32 (and Q15 comparison) wavetable bank files |
| [`wavetable_listening_samples`](src/bin/wavetable_listening_samples.rs) | Short WAV listening samples for the wavetable prototype report |

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

### `wavetable_listening_samples`

```bash
cargo run --release -p synth-tools --bin wavetable_listening_samples
```

Writes WAVs under `plans/wavetable-listening/`.
