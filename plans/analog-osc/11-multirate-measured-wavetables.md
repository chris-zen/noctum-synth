# Multi-Rate Measured Wavetable Bank v2

**Order:** 11 · **Depends on:** plans 03, 06, 07, and 10 · **State:** `[~]`
runtime, generator, UI, and Monologue + Prophet v2 banks implemented; combined
held-out metrics, alias sweeps, and zero-miss soak remain open.

## Execution status — both banks built; qualification gates open

The schema-v2 generator and runtime are implemented. `synth-tools
wavetable_bank` is authoritative for both targets; it reconstructs all 33 mips
from each original 2,048-bin complex spectrum, emits variable table lengths
and offsets, writes generated Rust profiles, and rejects incoherent or
non-deterministic inputs. The Python Monologue entry point now delegates to the
Rust tool. The embedding script reads schema, counts, paths, and checksums from
the manifests and enforces the 20 MiB combined cap.

The Monologue source dataset was downloaded and checksum-verified, extracted,
and regenerated as `korg-monologue-measured-wavetable-v2`. Its 1,804,032 f32
samples occupy 7,216,128 bytes (6.88 MiB). The Arturia Prophet-5 V r7 capture
was verified (226 complete cases, adapter revision 7), extracted to revision-2
NPZs, and regenerated as `prophet5-wavetable-bank-v2` (1,854,144 f32 samples,
7,416,576 bytes). Combined compiled assets are 14,632,704 bytes. Runtime
selection uses the generated 1,024-entry log-space lookup from the effective
per-lane phase increment; pitch-knot lookup remains at the existing 16-sample
control rate. Saw/triangle shape, SawTri, pulse residual/PWM, and hard-sync
edge sampling all share the same mip pair. Core tests cover 24, 44.1, 48, 96,
and 192 kHz, safe-harmonic selection, the one-semitone upper transition, and
preview/live parity.

`WavetableSupportStatus` is exposed by `OscillatorPreview`. Osc Design displays
a warning only during the one-semitone transition or above the captured range.
The stable bank IDs and session/configuration surface are unchanged, and Daisy
still builds without `osc-wavetable` assets.

48/96 kHz held-out target metrics, material alias sweeps, and the final
zero-miss performance soak remain open. See
`research/reports/multirate-measured-wavetable-v2.md`.

## DSP background

One bright wavetable cannot safely play every pitch and sample rate. At higher
notes, its upper harmonics cross Nyquist and alias. A mip bank stores versions
with different harmonic limits. The renderer chooses the richest safe version
from the actual phase increment, then blends at boundaries. Pitch interpolation
answers a different question—how the target's character changes between
measured notes—so this plan keeps pitch and mip dimensions separate.

## Summary

Replace the fixed-playback-rate banks with one sample-rate-independent,
pitch-conditioned mip bank per target. Do not generate separate 44.1, 48, 96,
or 192 kHz banks.

The existing plans already anticipated measured mipmaps, safe mip
interpolation, and testing at 44.1, 48, 96, and 192 kHz, but the first runtime
simplified this into one pitch-conditioned table per knot, filtered for a
single reference rate. This plan restores the missing mip dimension while
retaining the successful pitch interpolation.

Update plans 00 and 10 with this plan's execution status. Keep frozen v1
reports unchanged.

## Bank Generation and Format

- Introduce manifest/profile schema v2 with layout `waveform -> mip -> pitch ->
  samples`.
- Generate a universal quarter-octave harmonic hierarchy:
  - Start at harmonic 1023.
  - Repeatedly divide by `2^(1/4)`, floor and deduplicate, ending at harmonic 1.
  - Table length is
    `max(64, next_power_of_two(2 * (harmonic_limit + 1)))`.
  - Keep the existing `0.45 * sample_rate` Nyquist guard.
- Reconstruct every mip directly from the original measured complex spectrum.
  Never derive lower mips by resampling another time-domain table.
- Preserve each target's current phase policy, level, DC policy, pitch grid,
  source hashes, and maximum characterized frequency.
- Record capture/source sample rate as provenance only; remove it as a playback
  compatibility gate.
- Make the Rust `synth-tools wavetable_bank` implementation authoritative for
  both Monologue and Prophet banks. Retire duplicated fixed-rate filtering from
  the Python generator.
- Regenerate:
  - `korg-monologue-measured-wavetable-v2`
  - `prophet5-wavetable-bank-v2`
- Keep `BankId::Monologue` and `BankId::Prophet5` stable, so desktop
  configuration requires no migration.
- Make the embedding script read sample counts and checksums from the v2
  manifests instead of hard-coded values.
- Cap the combined compiled banks at 20 MiB. If generation exceeds that limit,
  fail rather than silently reducing mip density.
- Use only the completed Arturia r7 capture and extraction revision 2. The
  capture has all 226 cases but its `derived/` directory is currently empty,
  so extraction is a required prerequisite. Never derive v2 from the current
  invalid legacy Prophet asset.

## Runtime and Interfaces

The central invariant is simple enough to test directly:

```rust
let safe_harmonics = (0.45 / phase_increment).floor() as usize;
let (rich, lean, amount) = mip_lookup[safe_harmonics.min(1023)];
let sample = lerp(render(rich, phase), render(lean, phase), amount);
```

`phase_increment` is cycles advanced per output sample after every pitch
modulator. The generated lookup must guarantee that both selected mips are safe;
the sketch omits pitch-knot interpolation and bounds/error handling.

- Replace `reference_sample_rate_hz` and `supports_sample_rate()` with
  mip-layout metadata and runtime support status.
- Calculate the safe harmonic budget from the effective per-lane phase
  increment after detune, glide, pitch bend, and slop:

  `safe_harmonics = floor(0.45 / phase_increment)`

- Use a generated 1024-entry lookup table to select the richest safe mip and
  its adjacent leaner mip every sample. This avoids logarithms and guarantees
  that slop cannot leave an unsafe mip active for several samples.
- Blend adjacent safe mips using a precomputed log-space amount. At every
  boundary, the previous output becomes the lean endpoint of the newly
  available richer pair, keeping the selection continuous without ever mixing
  an unsafe table.
- Continue calculating pitch-knot interpolation at the existing 16-sample
  control rate. Each sample therefore interpolates:
  - lower/upper target-character pitch knots;
  - richer/leaner safe mips.
- Apply the same mip pair to saw/triangle shaping, SawTri, measured pulse
  residual, PWM shifted-saw construction, and hard-sync edge evaluation.
- Remove sample-rate-based BLEP fallback. Within the captured pitch domain,
  both banks must use measured tables at 24, 44.1, 48, 96, and 192 kHz.
- Preserve the target-domain boundary:
  - Up to the declared maximum: 100% measured.
  - Over the next semitone: clamp to the final measured character table and
    crossfade to BLEP.
  - Above that transition: BLEP fallback with status `AboveCapturedRange`.
  - Invalid frequency or a sample rate too low to retain even the fundamental
    receives an explicit status rather than masquerading as measured output.
- Add a lightweight `WavetableSupportStatus` exposed by `OscillatorPreview`;
  Osc Design shows a warning only when the current note is transitioning or
  outside the captured domain. Do not add controls or clutter to the Params
  header.
- Preserve allocation-free/no-std rendering and the existing engine/bank
  session APIs.

## Test and Evaluation Plan

- Generator tests:
  - Validate v2 layout, offsets, lengths, checksums, monotonic harmonic limits,
    and power-of-two table lengths.
  - Use synthetic phase-rich cycles to prove every mip removes all bins above
    its declared limit while preserving legal harmonic amplitude and phase.
  - Prove v2 generation is deterministic and rejects invalid or incoherent
    source data.
- Runtime tests:
  - For both banks, every waveform and representative low/mid/high supported
    note must report measured rendering at 24, 44.1, 48, 96, and 192 kHz.
  - Sweep every MIDI note in each captured domain and assert
    `selected_harmonic * frequency <= 0.45 * sample_rate`.
  - Exercise every mip boundary with static pitch, glide, slop, PWM, and hard
    sync; require finite output and no transition discontinuity beyond the
    existing Plan 03 click gate.
  - Verify Osc Design and live engine sample parity at all four desktop sample
    rates.
  - Verify the one-semitone upper-domain crossfade and explicit fallback
    status.
  - Confirm 48/96 kHz v2 target metrics pass the existing Plan 10 held-out
    gates and all existing material alias gates report zero failures.
- Performance/build gates:
  - Benchmark one, four, and sixteen voices through Pass Through at 44.1, 48,
    96, and 192 kHz.
  - At 48 kHz, sixteen-voice p99 must remain below 50% of one audio-frame budget
    and a 60-second mip/slop sweep must have no missed deadlines.
  - BLEP-only output remains bit-identical.
  - `osc-wavetable`, `osc-all`, app, and research suites pass.
  - Daisy remains `osc-blep` only and must not link the v2 assets.

## Documentation and Assumptions

- Update plan 10 to describe the pitch-by-mip representation and mark the
  fixed-rate v1 bank as superseded, not rewritten.
- Update the master plan and research README with v2 manifests, memory/runtime
  results, alias sweeps, and reproduction commands.
- Add a new multirate v2 research report; retain the Arturia capture audit and
  Monologue v1 reports as historical evidence.
- Validated desktop rates are 44.1, 48, 96, and 192 kHz; 24 kHz is validated in
  core for future hardware evaluation. Other finite rates use the same
  algorithm but are not formally qualified.
- Higher playback rates expose only harmonics present in the source capture:
  Monologue cannot gain content above its 48 kHz capture bandwidth, and Prophet
  cannot gain content above its 96 kHz capture bandwidth.
- This work fixes sample-rate compatibility and boundary behavior only; it does
  not expand the target-character pitch range or claim Prophet hardware
  accuracy.

## References

- Plan 10 measured-model evidence:
  [`10-measured-wavetable-residual.md`](10-measured-wavetable-residual.md)
- Existing runtime: `synth-core/src/dsp/wavetable.rs`
- Vesa Välimäki and Antti Huovilainen, *Antialiasing Oscillators in Subtractive
  Synthesis*: <https://doi.org/10.1109/MSP.2007.366145>
