# Multi-Rate Measured Wavetable Bank v2

## Summary

Replace the fixed-playback-rate banks with one sample-rate-independent,
pitch-conditioned mip bank per target. Do not generate separate 44.1, 48, 96,
or 192 kHz banks.

The existing plans already anticipated measured mipmaps, safe mip
interpolation, and testing at 44.1, 48, 96, and 192 kHz, but the first runtime
simplified this into one pitch-conditioned table per knot, filtered for a
single reference rate. This plan restores the missing mip dimension while
retaining the successful pitch interpolation.

Update plans 00 and 07 with this plan's execution status. Keep frozen v1
reports unchanged.

## Background: Why Mip Wavetables Are Needed

A wavetable oscillator stores one cycle of a waveform as an array of samples.
It plays different notes by moving through that array at different speeds. The
table itself does not inherently belong to 44.1, 48, or 96 kHz; the playback
speed is calculated from `frequency / sample_rate`.

The difficulty is aliasing. Sharp saw and pulse shapes contain many harmonics.
At a given audio sample rate, frequencies above half the sample rate (the
Nyquist frequency) cannot be represented correctly and fold back into the
audible spectrum as unrelated tones. Raising the played note also raises every
harmonic, so a table that is safe for a low note may alias when played higher.

A mipmapped wavetable solves this by storing several versions of the same
cycle:

- Rich mip levels retain many harmonics for low notes or high audio sample
  rates.
- Lean mip levels retain fewer harmonics for higher notes or lower audio sample
  rates.
- The oscillator selects the richest version whose highest harmonic remains
  safely below Nyquist.

This is similar to image mipmaps: a renderer uses a lower-detail image when an
object becomes small enough that full detail would produce artifacts. Here the
"detail" is the number of waveform harmonics rather than image pixels.

The measured banks have a second, independent dimension. Arturia Prophet-5 V
and Korg Monologue waveforms change character across pitch, so the bank stores
cycles captured at multiple pitch knots and interpolates between neighboring
knots. The two dimensions have different jobs:

- **Pitch knots** describe how the target oscillator's waveform character
  changes with pitch.
- **Mip levels** make that character safe at the current playback sample rate
  and actual oscillator frequency.

The first runtime implemented the pitch-knot dimension but stored only one
band-limited version of each knot. A Prophet bank filtered for 96 kHz could
retain harmonics up to approximately 43.2 kHz. Playing that same table at 48
kHz would place some retained harmonics above the 24 kHz Nyquist limit, so the
runtime rejected the entire bank and silently rendered BLEP instead. Filtering
one replacement bank for 48 kHz would be safe at higher rates but would
permanently discard high-frequency detail that 96 or 192 kHz could reproduce.

The v2 bank therefore stores multiple harmonic limits for every measured pitch
knot. At runtime, the oscillator derives a safe harmonic budget from the
effective frequency and current sample rate, selects two safe neighboring mip
levels, and blends them. The same bank can consequently run at multiple sample
rates without runtime FFT generation or one complete bank per sample rate.

The `0.45 * sample_rate` guard leaves ten percent of the spectrum below Nyquist
unused. This margin protects interpolation, frequency modulation, drift, and
numerical error from placing a nominally legal harmonic directly on the
Nyquist boundary.

Mipmaps cannot invent information missing from the original recording. A
96 kHz Prophet capture can contain source detail only below 48 kHz, while the
published 48 kHz Monologue capture can contain detail only below 24 kHz. A
higher playback rate can use all available captured detail more safely, but it
cannot reconstruct harmonics that the capture never contained.

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
    existing Plan 13 click gate.
  - Verify Osc Design and live engine sample parity at all four desktop sample
    rates.
  - Verify the one-semitone upper-domain crossfade and explicit fallback
    status.
  - Confirm 48/96 kHz v2 target metrics pass the existing Plan 07 held-out
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

- Update plan 07 to describe the pitch-by-mip representation and mark the
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
