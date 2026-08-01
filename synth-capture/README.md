# synth-capture

Host-only tool that controls an external synth over MIDI, records audio input,
validates each take, and extracts pitch-conditioned cycles for oscillator
research. It does not host plugins and does not depend on Python at runtime.

**Characterise a synth (end-to-end):**
[`docs/characterise-a-synth.md`](docs/characterise-a-synth.md)

Build and test with a locked lockfile:

```bash
cargo check -p synth-capture --locked
cargo test -p synth-capture --locked
```

## Virtual routing

```text
synth-capture MIDI out  ->  virtual MIDI port (exact name match)
                         ->  synth / plugin standalone
                         ->  virtual audio cable (e.g. BlackHole)
synth-capture audio in  <-  float32 @ protocol rate (Arturia: 96 kHz)
```

Use a dedicated virtual MIDI bus and a loopback audio device so the capture
machine does not hear the take on speakers. Cursor / OS mic permission may be
required for the loopback input.

## CLI

```bash
cargo run --release -p synth-capture --locked -- devices
cargo run --release -p synth-capture --locked -- new \
  --project target/analog-osc/captures/arturia-prophet5-v1 \
  --target arturia-prophet5-v1 \
  --protocol oscillator-static-v1 \
  --midi-port "IAC Driver Bus 2" \
  --audio-device "BlackHole 2ch" \
  --input-channel 0 \
  --sample-rate 96000 \
  --plugin-version "3.0.0"

cargo run --release -p synth-capture --locked -- doctor --project target/analog-osc/captures/arturia-prophet5-v1
cargo run --release -p synth-capture --locked -- run --project target/analog-osc/captures/arturia-prophet5-v1
cargo run --release -p synth-capture --locked -- status --project target/analog-osc/captures/arturia-prophet5-v1
cargo run --release -p synth-capture --locked -- verify --project target/analog-osc/captures/arturia-prophet5-v1
cargo run --release -p synth-capture --locked -- retry --project target/analog-osc/captures/arturia-prophet5-v1 --failed
cargo run --release -p synth-capture --locked -- retry --project target/analog-osc/captures/arturia-prophet5-v1 --all
cargo run --release -p synth-capture --locked -- retry --project target/analog-osc/captures/arturia-prophet5-v1 --complete
cargo run --release -p synth-capture --locked -- retry --project target/analog-osc/captures/arturia-prophet5-v1 --session session-…
cargo run --release -p synth-capture --locked -- extract --project target/analog-osc/captures/arturia-prophet5-v1
```

`--dry-run` works on `doctor` and `run` (no ports opened, no audio written).

Global `--color auto|always|never` controls terminal styling. `--json` keeps
stdout machine-readable and disables animated bars. `NO_COLOR` overrides
`auto`. Non-TTY sessions emit bounded plain progress lines.

### Interruption and resume

Ctrl-C stops after the current take: note-off / panic, partial audio under
`incomplete/`, case marked interrupted, nonzero exit. On the next `run`,
completed hashes are verified, only non-complete cases restart, and completed
WAVs are never overwritten.

`retry` archives WAV/metadata under `superseded/<stamp>/` and returns cases to
`Pending` without recreating the project. Prefer `--failed` / `--case` /
`--session`; use `--all` or `--complete` only when intentionally redoing work.
Doctor record and `project.json` stay intact.

### Extract

`extract` requires every case `Complete` with matching WAV hashes (`verify`
equivalent). It writes under `<project>/derived/`:

- `{saw,triangle,pulse50}-cycles-v1.npz` — median cycles (normalized + raw),
  harmonics, pitch/role/normalization arrays
- `{saw,triangle,pulse50}-summary-v1.json` — per-note scalars and fingerprints

Parity fixtures live under `tests/fixtures/extraction/`.

## Arturia Prophet-5 V (v1)

**Identity:** this target is Arturia’s software Prophet-5 V, not Sequential
hardware. Do not treat derived cycles as Prophet-5 / Rev2 hardware references.
See [`docs/arturia-prophet5-v1-target.json`](docs/arturia-prophet5-v1-target.json).

### Absolute MIDI Learn

Import [`docs/Noctum-Characterisation.promidi`](docs/Noctum-Characterisation.promidi)
(absolute CC). Full CC / neutral table:
[`docs/arturia-prophet5-v1-mapping.md`](docs/arturia-prophet5-v1-mapping.md).

Fine Tune, Pulse Width, and Filter Env Amount are **not** MIDI-mapped (7-bit
cannot center). Confirm once per `doctor` / `run` session when prompted:

- Osc 2 Fine Tune `0.000`
- Osc 2 Pulse Width `50%`
- Filter Env Amount `5.0`

### Oscillator-2 reset

`reset()` drives every MIDI-reachable neutral in the mapping (osc 1 off, osc 2
level up, cutoff open, envelopes neutral, filter key follow off, etc.). Waveform
changes always send all three osc-2 switches. Changing any CC or neutral bumps
`ADAPTER_REVISION` and requires a new project.

Doctor probes must show three spectrally distinct osc-2 waves.

### Manual acceptance checklist

1. Import `.promidi` once; set manual center controls when prompted.
2. `devices` → `new` → successful `doctor`.
3. Capture a few cases, Ctrl-C, resume without rewriting completed files.
4. Complete all 226 cases; `verify` clean.
5. `extract` → three NPZs × 75 notes; spot-check cycles/spectra.
6. Record pass/fail + plugin/OS versions (see acceptance notes below).

## Dependency policy

- Commit `Cargo.lock`; use `--locked` for build/test/docs commands.
- Crates.io + workspace paths only (`deny.toml` sources).
- `cargo deny check advisories bans licenses sources`
- `cargo vet check` (Mozilla + Google imports; exemptions are visible debt in
  `supply-chain/config.toml`, with notes on synth-capture direct pins)
- `cargo audit` (ignores aligned with `deny.toml` via `.cargo/audit.toml`)
- Review `Cargo.lock` and `cargo tree -p synth-capture -e features` on dependency
  changes; reject surprise networking, TLS, async runtimes, compression, or
  native libs beyond CPAL/MIDI platform backends.

```bash
cargo deny check advisories bans licenses sources
cargo vet check
cargo audit
cargo tree -p synth-capture -e features --locked
```

## Adding a target

1. Implement `SynthTarget` under `src/targets/` (descriptor, capabilities,
   reset, parameters, notes, panic / `prepare_session`, optional
   `operator_setup_steps`).
2. Register in `targets::resolve_target` / descriptor lookup.
3. Add mapping doc + optional Learn artifact; bump adapter revision on map
   changes.
4. Do not edit runner, audio ring, project persistence, or validation to teach
   a new synth MIDI dialect.

## Adding a protocol

1. Implement `CaptureProtocol` under `src/protocols/` (case matrix, roles).
2. Add a matching `CaptureExtractor` under `src/extraction/` if the protocol
   needs derived products beyond raw WAVs.
3. Wire CLI `new` / extract dispatch. Reuse the same project/transport/recorder.

## Future hardware provenance (Peak / Rev2)

Novation Peak and Prophet Rev2 adapters must document:

- Exact hardware revision / firmware / OS host used for the capture set
- MIDI path (DIN vs USB class-compliant) and audio interface identity
- Whether the take is single-voice / unison / stacked, and any factory init
- Mapping fingerprint + adapter revision in project metadata

Software-model captures (Arturia, etc.) must keep an explicit identity warning
so research consumers never confuse them with hardware measurements.

## Attribution (prior work)

The static chromatic capture grid and cycle-extraction numerics follow the
public **Korg Monologue** analog-VCO dataset and accompanying work by
Riccardo Simionato and Stefano Fasciani:

- Dataset (CC-BY-4.0): [DOI 10.5281/zenodo.15196138](https://doi.org/10.5281/zenodo.15196138)
- Record: <https://zenodo.org/records/15196138>
- Paper: <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Companion code: <https://github.com/RiccardoVib/NeuralOSC>

`synth-capture` does **not** redistribute those pickle files. Arturia (and
future) takes are separate capture projects. The Monologue set remains the
hardware reference that this protocol is designed to be comparable with; use
their DOI/authors when discussing that dataset or methods derived from it.

## Tracked vs ignored artifacts

Tracked: crate source, schemas, target JSON, mapping docs, compact fixtures,
acceptance stubs. Ignored under `target/`: source WAVs, project state, derived
NPZ/banks (`target/analog-osc/…`).
