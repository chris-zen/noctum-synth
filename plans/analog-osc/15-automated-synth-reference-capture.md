# Automated Synth Reference Capture and Extraction

## Decision

Build a host-only, resumable capture tool that controls an external synth over
MIDI, records its audio input, validates every take, and extracts the
pitch-conditioned cycles and harmonics consumed by the oscillator research
pipeline. Target-specific behavior is isolated behind a typed Rust trait;
capture scheduling, audio, persistence, validation, and extraction remain
generic.

The first target is Arturia Prophet-5 V running as a standalone application
through virtual MIDI and audio ports. The first protocol intentionally matches
the useful part of the public Korg Monologue dataset: chromatic static captures
of saw, triangle, and 50-percent pulse. Prophet-5 V **oscillator 2** is the
source because it is the oscillator that provides all three waveforms.
Oscillator 1 is disabled throughout capture.

Rust is authoritative for both acquisition and extraction. Python remains only
for the published Korg pickle importer, experimental fitting/plotting, and
temporary numerical-parity checks. Runtime bank fitting and installation are
separate research decisions and are not part of this workstream.

## Scope and boundaries

Included:

- External MIDI output through `midir`.
- Input-only audio capture through `cpal`.
- A reusable target adapter trait with mandatory reset, semantic parameter,
  note, panic, optional operator-setup steps, and session-prepare operations.
- A reusable protocol trait that produces complete capture cases.
- Exact float WAV recording, validation, checksums, transactional state, and
  interruption/resume.
- A Rust oscillator-static extractor producing the normalized cycles, raw
  measurements, measured frequencies, complex harmonics, and fixed data split
  required by existing fitting scripts.
- Arturia Prophet-5 V target adapter and Korg-equivalent static protocol.
- Simulation, parity, resume, and manual Arturia acceptance tests.

Excluded from v1:

- Hosting AU or VST3 plugins.
- Variable pulse-width grids, continuous PWM, hard sync, output-level sweeps,
  note-reset/free-running studies, and stochastic drift fitting.
- Filter captures. The architecture must support a future filter protocol
  without replacing MIDI, audio, project, or runner code.
- Automatic target-specific bank fitting, harmonic-guard selection, Rust
  profile generation, or installation into `synth-core`.
- A GUI or changes to the synth application's live audio path.

## Workspace layout

Add a dedicated host crate rather than extending `synth-app` or placing the
system in a single `synth-tools` binary:

```text
synth-capture/
  Cargo.toml
  README.md
  src/
    lib.rs
    main.rs
    domain.rs
    project.rs
    runner.rs
    validation.rs
    audio/
      mod.rs
      cpal_input.rs
      wav.rs
    extraction/
      mod.rs
      oscillator_static_v1.rs
      wav_reader.rs
    midi/
      mod.rs
      midir_output.rs
    protocols/
      mod.rs
      oscillator_static_v1.rs
    targets/
      mod.rs
      prophet5_v1.rs
  tests/
    extraction_parity.rs
    project_resume.rs
    simulated_capture.rs
```

Add `synth-capture` to the workspace. The dependency policy and exact crate
selection are fixed in the next section; the implementer must not substitute a
different terminal, hashing, WAV, signal, array, or interruption crate without
review.

Do not reuse `synth-app/src/audio.rs`: it manages a continuously rendered
output/monitoring session, while capture needs an input-only stream, exact
frame transactions, and strict overflow failure.

## Crates and supply-chain policy

No third-party crate can truthfully be described as immune to a compromised
publisher, malicious maintainer, dependency confusion, or undiscovered
vulnerability. The goal is therefore a small, explicit dependency set with
locked sources, restricted features, reviewable lockfile changes, published
security audits where available, and no opportunistic replacements.

Reuse these versions already present in the workspace lockfile:

| Purpose | Crate/version | Policy |
| --- | --- | --- |
| Audio input | `cpal 0.18.1` | Reuse the same backend as `synth-app`; enable only host-required platform features. |
| MIDI output | `midir 0.11.0` | Reuse the existing MIDI backend. |
| Audio ring | `rtrb 0.3.4` | Reuse the existing lock-free SPSC ring. |
| FFT | `rustfft 6.4.1` | Reuse the established offline spectral implementation. |
| Complex values | `num-complex 0.4.6` | Reuse the version selected by `rustfft`. |
| Serialization | `serde 1.0.228`, `serde_json 1.0.150` | Use derived schemas and deterministic pretty JSON. |
| Errors | `thiserror 2.0.18` | Use the already locked current major only. |
| Test directories | `tempfile 3.27.0` | Development dependency only. |

Add only these direct dependencies, at the versions verified when this plan
was written:

| Purpose | Crate/version | Required feature policy |
| --- | --- | --- |
| CLI parsing | `clap 4.6.4` | `default-features = false`; enable only `std`, `derive`, `help`, `usage`, and `error-context`. Do not enable Clap color because terminal rendering is centralized. |
| Color and progress | `indicatif 0.18.6` | `default-features = false`; use ASCII bars and its existing `console` integration. Do not add a second color crate. |
| SHA-256 | `sha2 0.11.0` | Pure-Rust SHA-256 only; do not add OpenSSL for hashing. |
| WAV I/O | `hound 3.5.1` | Float WAV read/write only. |
| Ctrl-C | `ctrlc 3.5.2` | Install one process-wide handler that only flips an atomic flag. |
| Arrays | `ndarray 0.17.2` | Disable optional parallel/BLAS integrations. |
| NPZ output | `ndarray-npy 0.10.0` | `default-features = false`; enable only `npz` and `num-complex-0_4`, leaving compression off to avoid the deflate dependency and unnecessary capture-time CPU. |

The versions above are inputs to the first implementation, not a claim that
newer releases are unsafe. Cargo.lock is the authoritative pin. Apply these
controls:

1. Commit `Cargo.lock`; all build/test/documented commands use `--locked`.
2. Accept crates only from crates.io and workspace paths. Deny unreviewed Git
   dependencies, alternate registries, and local path dependencies outside the
   workspace.
3. Add `deny.toml` and run `cargo deny check advisories bans licenses sources`
   in CI. Deny known vulnerabilities, yanked crates, unknown registries, and
   unapproved licenses.
4. Initialize `cargo-vet`. Import a reputable audit set, audit the direct new
   crates and enabled feature paths, and require a documented exemption with
   version and criteria when no audit exists. An exemption is visible debt,
   never an implicit pass.
5. Run `cargo audit` against the committed lockfile in CI and before release.
6. Review every `Cargo.lock` diff and `cargo tree -p synth-capture -e features`
   change. Reject unrelated packages or unexpectedly enabled networking,
   process execution, TLS, async runtimes, compression, or native libraries.
7. Use `cargo fetch --locked` followed by offline `cargo build --locked` for
   release artifacts. Cargo checksum verification must remain enabled.
8. Do not add runtime networking, automatic update checks, plugin loading,
   shell execution, or deserialization of untrusted pickle data to the Rust
   tool.
9. Keep unsafe code out of project-owned modules. Native unsafe code inside
   CPAL/MIDI platform dependencies is accepted only through the pinned,
   reviewed versions above.
10. Re-run advisories and vet review deliberately when updating a dependency;
    never use an automated major-version merge for the capture tool.

## Semantic domain

Protocols never emit raw MIDI. Define validated, serializable domain values
for MIDI channel, MIDI note, velocity, unit interval, bipolar unit interval,
duration, and sample rate. Constructors reject NaN, infinity, and values
outside their legal ranges while creating or loading a project.

Define semantic parameter operations whose variants own correctly typed
values:

```rust
pub enum ParameterSetting {
    OscillatorWaveform {
        oscillator: OscillatorId,
        waveform: OscillatorWaveform,
    },
    OscillatorPulseWidth {
        oscillator: OscillatorId,
        normalized: UnitInterval,
    },
    OscillatorLevel {
        oscillator: OscillatorId,
        normalized: UnitInterval,
    },
    OscillatorTuneSemitones {
        oscillator: OscillatorId,
        semitones: i16,
    },
    OscillatorKeyboardTracking {
        oscillator: OscillatorId,
        enabled: bool,
    },
    OscillatorLowFrequencyMode {
        oscillator: OscillatorId,
        enabled: bool,
    },
    NoiseLevel(UnitInterval),
    FilterCutoffNormalized(UnitInterval),
    FilterResonance(UnitInterval),
    FilterEnvelopeAmount(BipolarUnit),
    AmplifierEnvelope(EnvelopeSetting),
    FilterEnvelope(EnvelopeSetting),
    UnisonEnabled(bool),
    OscillatorSyncEnabled(bool),
    VoiceDispersion(UnitInterval),
    MasterLevel(UnitInterval),
}
```

The filter-related variants establish the future common vocabulary; the v1
protocol does not generate them beyond the neutral state applied by target
reset.

`CaptureCase` is target-neutral and contains:

- A stable deterministic case ID.
- `Silence` or `Stimulated` kind.
- The complete semantic settings applied after reset.
- An optional MIDI-note stimulus.
- Parameter-settle, attack-discard, stored-capture, and post-note durations.
- Expected fundamental and permitted pitch error.
- Scientific role: `Training`, `Validation`, `Test`, `GuardValidation`,
  `GuardTraining`, or `NoiseFloor` (silence / oscillator-off reference only).
- Typed tags for waveform, note, pulse width, oscillator, protocol revision,
  and target revision.

## MIDI and target interfaces

The transport owns the physical connection and exposes only synchronous byte
submission:

```rust
pub trait MidiTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
    fn flush(&mut self) -> Result<(), MidiError>;
}
```

Wrap every transport in a transcript decorator that records relative time and
the exact bytes sent for case metadata. Port selection is an exact enumerated
name; never silently fall back or use a substring match.

The synth-specific contract is:

```rust
pub trait SynthTarget {
    fn descriptor(&self) -> TargetDescriptor;
    fn capabilities(&self) -> TargetCapabilities;
    fn audio_requirements(&self) -> AudioRequirements;
    fn operator_setup_steps(&self) -> Vec<OperatorSetupStep> {
        Vec::new()
    }
    fn reset(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError>;
    fn set_parameter(
        &mut self,
        midi: &mut dyn MidiTransport,
        setting: &ParameterSetting,
    ) -> Result<(), TargetError>;
    fn note_on(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
        velocity: MidiVelocity,
    ) -> Result<(), TargetError>;
    fn note_off(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
    ) -> Result<(), TargetError>;
    fn panic(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError>;
    fn prepare_session(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        self.panic(midi)
    }
    fn settle_policy(&self) -> SettlePolicy;
}
```

Rules:

- `reset()` is mandatory and has no default implementation.
- It must be idempotent and establish the full known characterization state;
  the runner never relies on the preceding case.
- Target methods return only after all required messages are submitted.
- Unsupported semantic settings return a typed error before recording starts.
- Note operations live on the target so a future implementation may use MIDI
  1.0, MPE, SysEx, or another target-specific trigger.
- Adding a synth requires one adapter module, adapter tests, and one target
  registry entry. Protocols and runner code do not change.
- `operator_setup_steps()` declares one-time manual operator actions that MIDI
  cannot establish. The shared `confirm_target_setup` helper asks once per
  `doctor`/`run` session through an `OperatorConfirmer` (stdin in the CLI;
  skip in tests). Targets must not re-implement that prompt.
- `prepare_session()` runs once at session start before operator setup. Default
  is `panic()`. Targets may strengthen it (for example Arturia sends all-notes
  off for every MIDI note) without flooding that sequence on every case.
- Per-case and error paths continue to call the lighter `panic()` (channel-mode
  all-notes-off / all-sound-off / sustain-off as appropriate).

## Capture protocol interface

```rust
pub trait CaptureProtocol {
    fn descriptor(&self) -> ProtocolDescriptor;
    fn validate_target(
        &self,
        capabilities: &TargetCapabilities,
    ) -> Result<(), ProtocolError>;
    fn build_cases(
        &self,
        config: &ProtocolConfig,
    ) -> Result<Vec<CaptureCase>, ProtocolError>;
}
```

The runner consumes only `CaptureCase`. A later filter protocol will define an
oscillator stimulus plus filter settings and will reuse the same runner,
target, audio, validation, and state machinery.

## Arturia Prophet-5 V adapter

### Routing and mapping

Run Prophet-5 V externally:

```text
synth-capture MIDI output
  -> virtual MIDI port
  -> Arturia Prophet-5 V standalone
  -> virtual audio cable
  -> synth-capture audio input
```

Arturia's MIDI Learn setup is a one-time transport mapping, not synth-state
initialization. Commit a versioned absolute-controller mapping contract in the
adapter documentation. Allocate undefined CC numbers from 102-119 first and
14-31 next, skipping any controller with standard channel-mode behavior.
Record the complete semantic-to-CC table and its SHA-256 in the adapter
descriptor. Prophet-5 V must be configured for absolute rather than relative
or toggle response.

No capture project may silently adopt a revised controller table. Changing a
CC assignment or neutral value increments the adapter revision and changes the
mapping fingerprint.

### Reset contract

`Prophet5V1::reset()` must send an absolute value for every relevant
switch and knob in this order:

1. All-notes-off, sustain-off, and modulation-wheel zero.
2. Oscillator 1 waveform switches off and oscillator 1 mixer level zero.
3. Oscillator 2 mixer level to the documented nominal capture level.
4. Oscillator 2 keyboard tracking on.
5. Oscillator 2 low-frequency mode off.
6. Oscillator 2 triangle, saw, and pulse switches all off.
7. Oscillator 2 pulse width to exactly 50 percent.
8. Noise level zero.
9. Oscillator sync off.
10. Poly-mod oscillator-2/noise source amounts zero and all destinations off.
11. LFO/modulation amounts and destinations off.
12. Filter cutoff fully open, resonance zero, and filter-envelope amount
    neutral.
13. Amplifier attack/decay/release minimum and sustain maximum.
14. Filter envelope set to its neutral capture values.
15. Unison, voice dispersion, effects, and other voice variation off.
16. Master output to the documented non-clipping nominal value.
17. Flush MIDI and wait the adapter reset-settle duration.

Oscillator 2 Fine Tune, Pulse Width, and Filter Envelope Amount are **not**
MIDI-mapped and are **not** part of `reset()`. 7-bit CC cannot center them.
The adapter declares `operator_setup_steps` that ask once per session to set
VCO 2 Fine Tune to exactly `0.000`, Pulse Width to exactly `50%`, and Filter
Env Amount to exactly `5.0` (bipolar center), then leave them untouched.
Protocol pulse cases still request 50% width semantically; the adapter treats
that request as a no-op so MIDI cannot overwrite the manual setting.

For each waveform, `set_parameter(OscillatorWaveform { oscillator: Two, ... })`
must explicitly send all three oscillator-2 switches: one selected switch is
127 and the other two are zero. Requests for oscillator 1 waveforms in this
protocol fail as unsupported. This makes retries deterministic even when the
previous capture was interrupted.

The adapter owns all MIDI-reachable neutral values. An operator is never asked
to restore an init preset manually between runs. The only allowed manual state
is declared via `operator_setup_steps` (Fine Tune, 50% Pulse Width, and centered
Filter Env Amount for Arturia v1). If a required state is neither MIDI-owned nor
declared as operator setup, `doctor` must fail rather than accepting an unknown
state.

### Automated doctor

Before `run`, `doctor` must:

- Open the exact MIDI output and audio input.
- Require native float32 input at exactly 96 kHz for Arturia v1.
- Call `prepare_session()` once, then confirm any `operator_setup_steps`.
- Run `reset()` for each probe.
- Capture short oscillator-off silence.
- Capture short A4 oscillator-2 saw, triangle, and pulse probes.
- Confirm all probes are non-silent and spectrally distinct.
- Confirm measured pitch is within 50 cents of A4.
- Report RMS, peak, DC, estimated frequency, clipping, and callback overflow.
- Fail on excessive silence, missing signal, clipping, octave error, identical
  probes, lost ports, or buffer overflow.

Store the successful doctor result, target/mapping revision, and measurements
in the project. `run` refuses to start without a compatible success record.

## Oscillator Static v1 matrix

The characterization range is MIDI 16 through 88 inclusive, E0 through E6.
Capture MIDI 89 and 90 as upper guards. Capture all 75 notes for oscillator-2
saw, triangle, and 50-percent pulse:

- 75 saw recordings.
- 75 triangle recordings.
- 75 pulse recordings.
- One ten-second oscillator-off silence recording.
- 226 cases total.

Roles for MIDI 16-88:

- `(note - 16) % 2 == 0`: training/table-knot candidate.
- `(note - 16) % 4 == 1`: validation.
- `(note - 16) % 4 == 3`: test.
- MIDI 89: guard validation.
- MIDI 90: guard training knot.

The later bank may use even notes through MIDI 90 to keep the last supported
interval guarded, but its declared playable maximum remains MIDI 88 unless a
separate evaluation promotes the guard range. Persist roles explicitly; no
downstream tool recalculates or changes them.

Default capture settings:

- Exact sample rate: 96,000 Hz.
- Stored file: mono IEEE-754 float32 WAV.
- Input channel index: 0.
- MIDI channel: 1.
- Velocity: 100.
- Settle after reset/settings: 250 ms.
- Discard after note-on: 500 ms.
- Stored stimulated duration: exactly 8.0 seconds or 768,000 frames.
- Post-note drain: 100 ms.
- Silence duration: exactly 10.0 seconds.
- No capture-time normalization, DC removal, resampling, trimming, or phase
  alignment.

Keep the input stream open across cases and delimit files using audio frames,
not wall-clock duration. Generate the execution order once and persist it by
sorting cases by SHA-256 of a length-prefixed encoding of
`capture_order_seed` and `case_id` (8-byte little-endian length of the seed,
seed bytes, 8-byte little-endian length of the case id, case id bytes). The
fixed seed is stored in `project.json`, distributing notes, waves, and data
roles throughout the session instead of aligning them with slow temporal drift.

## Project schema and storage

Default root:

```text
target/analog-osc/captures/<project-id>/
  project.json
  state.json
  sessions/<session-id>.json
  audio/<case-id>.wav
  cases/<case-id>.json
  incomplete/
  superseded/
  logs/events.jsonl
  derived/
    saw-cycles-v1.npz
    saw-summary-v1.json
    triangle-cycles-v1.npz
    triangle-summary-v1.json
    pulse-cycles-v1.npz
    pulse-summary-v1.json
```

Large project data remains ignored by Git.

`project.json` is immutable after creation and contains schema/project IDs,
target and adapter revisions, mapping fingerprint, Arturia/plugin/OS metadata,
protocol revision, explicit cases/order/roles, pitch and guard policy, exact
audio and MIDI settings, timings, validation thresholds, device names,
creation time, and a SHA-256 fingerprint of all scientific configuration.
The fingerprint is computed from a canonical integer/string material (frame
counts, milli-scaled widths/cents, case ids/roles/tags) so JSON float
round-trips cannot change it. Changing scientific settings creates a new
project.

`state.json` is mutable and atomically replaced after every transition. Status
is one of `Pending`, `Recording`, `Validating`, `Complete`, `Failed`, or
`Interrupted`. Each entry stores attempts, session ID, timestamps, reason,
paths, exact frames, WAV SHA-256, signal metrics, transcript fingerprint, and
case fingerprint. Write a temporary state file, flush it, and atomically rename
it; never edit live JSON in place.

## Recording transaction and resume

For every case:

1. Mark `Recording`.
2. Call target `panic()` and `reset()`.
3. Apply every case setting.
4. Wait while draining input.
5. Send note-on for a stimulated case.
6. Discard the configured attack interval.
7. Write the exact frames to `<case-id>.partial.wav`.
8. Send note-off and `panic()`.
9. Close, flush, and sync the partial WAV.
10. Mark `Validating`.
11. Validate and checksum it.
12. Atomically write per-case metadata.
13. Rename the WAV to its final path.
14. Mark `Complete`.

Before the first case of a `run` session (and before doctor probes), call
`prepare_session()` once, then confirm any `operator_setup_steps` once. Do not
repeat those prompts between cases.

A case is complete only when final WAV, metadata, checksum, case fingerprint,
and state agree.

On Ctrl-C, set an atomic stop flag, send note-off/all-notes-off, stop the
current take, move its partial file under `incomplete/`, mark it interrupted,
flush state, and exit nonzero. Do not begin another case.

On resume, verify every completed hash, skip only valid completed cases,
convert stale `Recording`/`Validating` entries to `Interrupted`, archive stale
partials, and restart only non-complete cases. Never overwrite completed audio.
`retry` first archives prior WAV/metadata under
`superseded/<timestamp>/`.

MIDI/audio disconnection, callback overflow, wrong frame count, or a repeated
validation failure stops the session. One validation failure may be retried
after a fresh reset; a second failure is recorded and terminates the run.

## Audio real-time boundary and validation

The CPAL callback performs no allocation, locking, or file I/O. It selects the
configured channel, converts the native sample representation to `f32`, pushes
to a bounded `rtrb` ring, and increments atomic error/overflow counters. The
recorder thread writes files. Allocate at least two seconds of ring capacity;
one dropped sample invalidates the case.

Arturia v1 requires a native float32 input. The generic backend may later
support integer hardware inputs, but it must record native format and
conversion policy in metadata.

Reject a stimulated file when frame count is wrong, a sample is non-finite,
the callback reported an error/overflow, RMS is below -48 dBFS, any sample has
absolute magnitude at or above 0.999, or measured pitch differs by more than 50
cents. Reject silence above -72 dBFS. Record absolute DC above 0.1 as a warning
without altering raw samples.

## Rust extraction layer

Rust owns the production extraction API:

```rust
pub trait CaptureExtractor {
    fn descriptor(&self) -> ExtractorDescriptor;
    fn supports(&self, protocol: &ProtocolDescriptor) -> bool;
    fn extract(
        &self,
        project: &CaptureProject,
        output: &Path,
    ) -> Result<ExtractionSummary, ExtractionError>;
}
```

`OscillatorStaticExtractorV1` must:

1. Validate project/state fingerprints and require all protocol cases complete.
2. Recheck WAV hashes, float format, rate, channels, and frame counts.
3. Estimate fundamental using a Hann-windowed FFT, local expected-frequency
   search, and three-bin log-magnitude parabolic interpolation, matching the
   current Python algorithm.
4. Remove only a temporary mean for landmark detection; preserve raw audio.
5. Find interpolated upward midpoint crossings and select the strongest valid
   landmark near each predicted period.
6. Reject startup/tuning anomalies and periods outside 0.8-1.2 times the median.
7. Select at most 1,024 cycles distributed uniformly through the stable take.
8. Interpolate each accepted cycle to 2,048 phase bins.
9. Compute a per-bin median cycle with deterministic total ordering and defined
   NaN rejection.
10. Preserve the raw median cycle and its DC/RMS/peak/crest/duty metrics.
11. Create a reversible normalized cycle by subtracting stored DC and dividing
    by stored peak absolute magnitude.
12. Compute normalized complex harmonics with `rustfft`, including DC and up to
    256 non-DC harmonics.
13. Record measured frequency, pitch error, period jitter, cycle-amplitude
    variation, accepted/rejected cycle counts, role, source checksum, and
    normalization scale.
14. Write compatible NPZ arrays and JSON summaries under `derived/`.

NPZ arrays per waveform:

```text
median_cycles
median_cycles_raw
complex_harmonics
measured_frequency_hz
nominal_midi
raw_dc
raw_rms
raw_peak
normalization_scale
period_jitter_ppm
cycle_amplitude_cv
role
```

`median_cycles` and `complex_harmonics` are normalized inputs for existing
fitting scripts. Raw cycles and normalization metadata make the transform
reversible. Include project, target, adapter, protocol, extractor, WAV, and
mapping fingerprints in the summaries.

Do not choose target phase canonicalization, table knots beyond the persisted
roles, Nyquist guards, runtime table layout, or promotion thresholds here.
Those remain in target-specific model/bank evaluation.

The existing Python Korg pickle importer remains intact. Port its numerical
algorithm into Rust rather than making the Rust tool invoke Python. Keep small
synthetic and extracted-cycle parity fixtures so Rust output is compared with
the established Python results before Rust becomes authoritative. Python
continues to handle Korg pickle deserialization because pickle is unsafe and
source-specific, not a format for future capture projects.

## CLI

Provide one release binary named `synth-capture`:

```text
synth-capture devices
synth-capture new
synth-capture doctor
synth-capture run
synth-capture status
synth-capture verify
synth-capture retry
synth-capture extract
```

- `devices`: enumerate exact MIDI outputs and audio input formats/rates.
- `new`: require a new/empty project path and write immutable config, cases,
  order, initial state, and Arturia mapping instructions.
- `doctor`: perform and store automated routing/state probes; confirm any
  one-time operator setup steps first.
- `run`: start or resume; accept no scientific overrides; confirm any one-time
  operator setup steps once at session start.
- `status`: show counts, last/current case, failures, elapsed captured duration,
  and estimated remaining duration; support `--json`.
- `verify`: rehash completed audio, verify metadata/header/frame agreement, and
  report missing, corrupt, or orphaned artifacts.
- `retry --failed`, `--complete`, `--all`, `--session <id>`, or `--case <id>`:
  archive WAV/metadata under `superseded/<stamp>/` and reset those cases to
  pending (project config and doctor record unchanged).
- `extract`: run the registered Rust extractor and refuse incomplete projects.
- `--dry-run`: print cases, MIDI operations, and operator-setup steps without
  opening ports or writing audio for `doctor` and `run`.

### Terminal colors and progress

All human terminal rendering goes through one `TerminalReporter` backed by
`indicatif::MultiProgress`; domain, target, audio, project, and extraction code
must emit typed events rather than ANSI sequences. Render progress and
diagnostics to stderr so stdout remains available for machine-readable output.

Use this fixed color vocabulary:

- Green bold: completed/verified (`OK`, completed case, valid checksum).
- Cyan: informational state (`INFO`, connected device, current phase).
- Yellow bold: warning/retry/interrupted state (`WARN`).
- Red bold: failure (`ERROR`, corrupt file, lost device).
- Magenta: current target, waveform, note, or extraction item.
- Dim white: skipped/resumed cases already verified as complete.

Provide global `--color auto|always|never`, defaulting to `auto`. `auto` enables
color only on a terminal and honors the standard `NO_COLOR` environment
variable. `--json` and non-terminal output disable colors and animated bars.
Never place ANSI escapes in logs, JSON, case metadata, error values, or test
fixtures.

During capture show no more than two live rows:

```text
Capture  [███████████---------------------]  78/226  34%  elapsed 21:04  ETA 40:12
Current  pulse  MIDI 52  recording  [██████████████----]  6.2/8.0 s
```

The overall bar advances only after a case becomes `Complete` or a previously
complete case is checksum-verified and skipped. The current bar is driven by
frames consumed on the recorder thread, never from the real-time callback.
Its length is the exact required capture frames. The message changes through
`reset`, `settle`, `discard`, `record`, `validate`, and `commit` phases.

During extraction use an overall waveform/note bar and a spinner for the
current `frequency`, `cycles`, `median`, `FFT`, or `write` phase. Progress
updates must be rate-limited to at most 10 Hz so terminal output cannot affect
capture throughput.

On success, interruption, panic cleanup, or error, finish or clear every bar,
restore cursor state, print exactly one final colored summary, and then return
the documented exit code. In non-interactive mode replace animation with one
plain line on phase changes and one line per completed/failed case; do not emit
thousands of frame updates.

Exit summaries include completed/skipped/failed counts, elapsed duration,
project path, and the precise resume command when incomplete. `status --json`
prints one JSON value to stdout and no progress UI. `devices` uses colored
availability markers only in human mode.

Representative creation:

```text
cargo run --release -p synth-capture -- new \
  --project target/analog-osc/captures/prophet5-v1 \
  --target prophet5-v1 \
  --protocol oscillator-static-v1 \
  --midi-port "Noctum Capture" \
  --audio-device "BlackHole 2ch" \
  --input-channel 0 \
  --sample-rate 96000 \
  --plugin-version "<installed version>"
```

## Tests and acceptance

Unit tests must cover validated values, exact 226-case generation, 75 cases per
wave, split/guard counts, stable IDs/order, complete oscillator-2 reset bytes,
reset idempotence, Fine Tune excluded from reset/MIDI map, session
`prepare_session` note-off flood, explicit three-switch waveform selection,
oscillator-1 rejection, unsupported parameters, project fingerprints, atomic
state recovery, retry archival, WAV format/frame counts, overflow failure, and
Ctrl-C cleanup.

Use fake MIDI and audio backends for an end-to-end shortened project. The fake
target renders deterministic saw/triangle/pulse from received operations.
Verify that the generic runner never inspects target MIDI, interrupted/resumed
and uninterrupted runs produce the same completed artifacts, completed cases
are not repeated, and extraction recovers expected frequency/cycles/harmonics.

Create parity fixtures containing non-bin-centered synthetic oscillators, DC,
noise, small pitch drift, and phase offsets. Assert Rust/Python agreement for
frequency, accepted cycles, median cycle, complex harmonics, and scalar metrics
with documented tight tolerances. Refactoring must not change existing Korg
reports or banks.

Manual Arturia acceptance requires:

1. Install the documented absolute MIDI Learn table once (no Fine Tune, Pulse
   Width, or Filter Env Amount CC; those are set manually at session start).
2. Run `devices`, `new`, and a successful `doctor` (confirm Fine Tune `0.000`,
   Pulse Width `50%`, and Filter Env Amount `5.0` when prompted).
3. Confirm doctor is using oscillator 2 for all three waves.
4. Capture several cases, interrupt one, and resume without repeating completed
   files.
5. Complete all 226 cases and pass `verify` with no corrupt, clipped, missing,
   or mis-pitched takes.
6. Run `extract` and obtain three NPZ files and summaries with 75 notes each.
7. Inspect representative raw/normalized cycles and spectra.
8. Consume the derived outputs from the existing measured-model evaluation via
   a thin source adapter.

Add terminal tests with a project-owned in-memory `TermLike` implementation
passed to `indicatif::ProgressDrawTarget::term_like`; do not enable indicatif's
optional `in_memory`/VT100 feature. Verify that color `never` contains no escape
bytes, `NO_COLOR` overrides auto, JSON keeps stdout clean, non-TTY mode emits
bounded plain lines, resume starts the overall bar at the verified complete
count, frame progress reaches exactly the configured length, and Ctrl-C/error
cleanup leaves no active bar or hidden cursor.

Verification commands use `--locked` and include `cargo check -p
synth-capture`, `cargo test -p synth-capture`, `cargo deny check`, `cargo vet`,
`cargo audit`, existing `synth-core` research tests, existing Python Korg tests,
and the normal `synth-app` check.

## Documentation and tracked artifacts

The crate README documents virtual routing, Arturia's absolute MIDI Learn
table, exact oscillator-2 reset state, devices, project creation, doctor,
terminal colors/progress and non-interactive behavior, interruption/resume,
verification/extraction, the dependency audit policy, adding a target, and
adding a protocol. Document hardware provenance requirements for future
Novation Peak and Prophet Rev2 adapters.

Update the Arturia capture target JSON to reference the target adapter revision,
mapping fingerprint, protocol revision, pitch/guard matrix, project schema,
and the identity warning that Arturia output is not Prophet hardware.

Track source, schemas, target descriptors, mapping documentation, compact
fixtures, and summarized reports. Keep source WAVs, project state, derived NPZ
files, and generated banks under ignored `target/analog-osc/` paths.

## Completion criteria

- A new synth can be added without editing runner, audio, persistence,
  validation, or extraction infrastructure.
- A new filter protocol can reuse the same target/transport/recording project
  and add only protocol and extractor behavior.
- Arturia initialization is adapter-owned for every MIDI-reachable control and
  repeatable after process restart. Oscillator 2 Fine Tune (`0.000`), Pulse
  Width (`50%`), and Filter Env Amount (`5.0`) are the declared one-time manual
  exceptions.
- Prophet-5 V oscillator 2 supplies triangle, saw, and pulse while oscillator 1
  remains disabled.
- Every completed take is immutable, checksummed, attributable, and resumable.
- Rust extraction reproduces accepted parity fixtures and emits the established
  research representation without requiring Python at capture time.
- Arturia v1 produces a verified Korg-equivalent static dataset ready for a
  separate target-specific wavetable study.

## References

- Existing capture/extraction plan:
  `plans/analog-osc/03-reference-capture-and-identification.md`
- Existing Korg importer: `scripts/analog_osc_reference.py`
- Existing measured-bank generator: `scripts/generate_measured_wavetable_bank.py`
- Arturia MIDI Learn and Prophet controls:
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
- Public Monologue dataset: <https://zenodo.org/records/15196138>
- Simionato and Fasciani capture/modeling paper:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
