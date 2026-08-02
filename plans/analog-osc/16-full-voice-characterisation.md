# Full-Voice Characterisation Programme

## Decision

Define the measurement programme that turns “characterise a synth for
emulation” into concrete `synth-capture` protocols, extractors, session
runbooks, and software ship gates before booking scarce hardware time.

The host remains [`15-automated-synth-reference-capture.md`](15-automated-synth-reference-capture.md).
This document owns **what** to measure for a full voice; it does not redefine
runner, audio, project persistence, or CLI shape.

## Reuse rule

`synth-capture` characterises **any** instrument for which a proper
`SynthTarget` adapter exists (capabilities, reset, parameters, notes, panic,
operator setup). Protocols, runner, audio, projects, doctor/run UX, and
extractors stay shared. Only the adapter—and any declared
`operator_setup_steps`—are synth-specific.

Each physical unit and each plugin revision is its **own named target
profile**. Never silently merge Arturia Prophet-5 V, Sequential Prophet Rev2,
Prophet-5 hardware, or Novation Peak into one dataset.

Protocols that a target cannot support fail `validate_target` or are skipped
for that project. Do not fork per-synth protocol crates.

## First examples

| Profile | Role |
| --- | --- |
| `prophet5-v1` | Software rehearsal target (virtual MIDI/audio). |
| Sequential Prophet Rev2 | First hardware example (NRPN/SysEx adapter). |
| Peak, Prophet-5 HW, … | Same protocol catalog once adapters exist. |

## Goals and non-goals

Goals:

- Enough measured evidence to build or fit oscillators, filter behaviour,
  envelopes, LFO/mod depths, and MIDI→physical control laws for a chosen
  target profile—without inventing curves in the studio.
- The same protocol suite runnable on every adapter that advertises the needed
  capabilities (Arturia rehearsal → Rev2 hardware → future Peak/etc.).

Non-goals:

- Effects / FX buses, arpeggiator, sequencer.
- Layer B as a v1 characterisation surface.
- Automatic bank installation into `synth-core`.
- Conflating software and hardware targets.
- Replacing research fitting plans (03, 04, 07, 09, …); those consume derived
  artifacts, they do not define capture protocols here.

## Architecture

```text
SynthTarget adapter  ─┐
CaptureProtocol      ─┼─→ runner / audio / project ─→ extractor (per protocol)
```

- One **project** = one `(target_id, protocol_id)` plus immutable scientific
  config and fingerprint.
- New synth ⇒ new adapter module + registry entry.
- MIDI→physical curves are **control-law protocols**, separate from static
  waveform capture.
- Rev2-class adapters prefer **NRPN** (14-bit where the instrument provides it)
  over 7-bit CC for continuous parameters, and may use SysEx for bulk state.
  Fall back to CC only when NRPN is unavailable. Arturia-style absolute MIDI
  Learn CC tables are for software rehearsal, not the Rev2 hardware path.

## Protocol catalog

Each protocol below is a planned `CaptureProtocol` (+ matching extractor).
Wall times assume 96 kHz float capture, ~250 ms settle, and typical Rev2-like
MIDI latency. Treat them as planning budgets, not SLAs.

Column meanings:

- **Takes**: approximate case count.
- **Wall**: net audio/MIDI time excluding cabling mistakes and re-runs.

### Tier A — required before first hardware day

Ship software, `--dry-run`, simulated tests, and at least one live smoke
(Arturia and/or Rev2) before booking hardware for Tier A.

| Protocol ID | Measures | Stimulus / MIDI | Takes (approx.) | Wall | Validation (sketch) | Extractor outputs | Depends on |
| --- | --- | --- | ---: | ---: | --- | --- | --- |
| `chain-loopback-v1` | Interface HPF/LPF/phase/latency | Silence + known sweep/impulse with synth bypassed or muted; same I/O chain as capture | ~5–15 | 5–10 min | Non-clip; measurable stimulus; stable RMS | Transfer summary (mag/phase vs Hz), latency | — |
| `oscillator-static-v1` | Saw / triangle / 50% pulse × pitch | Plan 15 matrix (MIDI 16–90, osc source per target) | 226 | 30–40 min | Pitch ±50¢; non-silent; spectral distinctness | Cycles, harmonics, roles (plan 15) | Host + osc doctor |
| `oscillator-pwm-static-v1` | Pulse width shape vs width × pitch | Pulse on; PW grid e.g. 10/25/50/75/90%; ≥1 note per octave (or denser) | ~40–80 | 15–25 min | Duty/width monotonic; pitch stable | Median cycles + duty metrics vs PW | Osc static smoke |
| `control-law-pitch-v1` | Note + fine/coarse → Hz / cents | Chromatic notes; fine/coarse sweeps at fixed notes | ~150–200 | 10–15 min | Monotonic pitch; known A4 reference | `raw → Hz/cents` tables + fit residuals | Loopback optional |
| `control-law-cutoff-v1` | Cutoff raw → Hz | Dense grid (e.g. 0–127); self-osc and/or noise/impulse FRF | ~64–128 | 15–25 min | Monotonic cutoff; no clip; method tagged | `raw → Hz` curve + method metadata | Pitch law helpful |
| `control-law-resonance-v1` | Resonance → Q / self-osc onset | Coarse res grid at fixed cutoffs | ~40–80 | 10–15 min | Onset detectable; Q finite | `raw → Q` / onset threshold | Cutoff law |
| `amp-env-shape-v1` | Amp ADSR times / shapes | Gated notes; A/D/S/R grids at fixed pitch/level | ~60–120 | 20–30 min | Envelope landmarks detectable | Time constants / shape summaries vs raw | Pitch law |
| `filter-env-shape-v1` | Filter env → cutoff motion | Same pattern; audible via cutoff / self-osc | ~60–120 | 20–30 min | Cutoff motion above noise | Env→cutoff timing/shape vs raw | Cutoff law |
| `lfo-rate-law-v1` | LFO rate raw → Hz | Rate sweep; audible destination (pitch or cutoff) | ~32–64 | 10–15 min | Period estimate stable | `raw → Hz` table | Pitch or cutoff law |

**Tier A subtotal:** roughly **2–3.5 hours** net audio for a minimum viable day
(loopback + osc static + pitch/cutoff laws + one env + LFO), or **~3.5–5 hours**
if every Tier A protocol is completed in one sitting.

### Tier B — same booking if time remains; else second session

| Protocol ID | Measures | Notes | Wall (ballpark) |
| --- | --- | --- | ---: |
| `filter-frf-v1` | Magnitude/phase vs cutoff, resonance, poles | Noise or impulse; denser than control-law alone | 30–60 min |
| `filter-keyboard-tracking-v1` | Cutoff vs note at fixed raw cutoff | Sparse note grid | 10–20 min |
| `oscillator-dynamic-v1` | Pitch glide, PWM LFO, hard-sync ratios | From plan 03 dynamic matrix | 30–60 min |
| `drift-longform-v1` | Slow drift, jitter, amplitude variation | Multi-minute held notes (plan 09) | 20–60+ min |
| `modulation-depth-v1` | Poly-mod / LFO depth → cents or cutoff Hz | Depth grids at fixed rate | 15–30 min |
| `unison-slop-spot-v1` | Unison / slop musical spot checks | Small case set; not a full law | 10–20 min |

### Tier C — later / research

- Soft clipping / drive / mixer saturation.
- Audio-rate FM/PM or oscillator sync depth laws if present.
- Aftertouch / velocity response curves.
- Multi-voice correlation (requires multiple voices or units).

## Session runbook

### Ordered sequence (hardware day)

1. Power-on warm-up (document duration; typically ≥20–30 min for analog).
2. Record unit ID, firmware, tuning/calibration state, interface, sample rate,
   input gain, date, operator.
3. Cable check: MIDI out exact port name; audio in exact device; 96 kHz float32.
4. `devices` → `new` projects as needed (one project per protocol).
5. `doctor` per project (confirm any `operator_setup_steps` once).
6. Run protocols in this order when possible:
   1. `chain-loopback-v1`
   2. `oscillator-static-v1`
   3. `control-law-pitch-v1`
   4. `control-law-cutoff-v1` / `control-law-resonance-v1`
   5. `oscillator-pwm-static-v1`
   6. `amp-env-shape-v1` / `filter-env-shape-v1`
   7. `lfo-rate-law-v1`
   8. Tier B as time allows (`filter-frf-v1`, dynamics, drift, mod depth)
7. `status` / `verify` after each project; resume rather than restart.
8. Do not change cabling, gain, or sample rate mid-profile without a new
   loopback and a new project fingerprint.

### Minimum viable day (~2–3 h net audio)

Loopback, oscillator-static, pitch law, cutoff law, amp-env or filter-env,
LFO rate. Enough to start emulation curves and a pitch-conditioned osc bank.

### Full day (~6–8 h including warm-up and re-runs)

All of Tier A plus filter FRF, keyboard tracking, PWM grid, one dynamic or
drift protocol, and modulation-depth spots.

### Operator discipline

- Prefer absolute MIDI over relative/toggle.
- Parameter transport preference, in order:
  1. **NRPN** (or other ≥14-bit absolute control) when the instrument exposes it.
  2. **7-bit CC** only when NRPN is unavailable for that parameter.
  3. **`operator_setup_steps`** only when no automated absolute control can hit
     the required value (e.g. Fine Tune-class parameters that cannot be centered
     with 7-bit CC and have no usable NRPN). Never silently guess or leave that
     state undefined.
- Use `prepare_session` / `panic` so hung notes never contaminate takes.
- Keep Arturia rehearsal projects completely separate from hardware profiles.

## Software ship gates (before booking)

All of the following must pass simulated tests and `--dry-run`, plus at least
one live smoke on Arturia and/or Rev2:

1. **Plan 15 host proven** — Arturia oscillator-static acceptance and Phase 6
   extraction (Phases 6–7 of [`15-execution-phases.md`](15-execution-phases.md)).
2. **Hardware target adapter** for the booked instrument (Rev2 first): prefer
   NRPN for continuous controls, fall back to CC only when needed, full
   reset/panic/`prepare_session`, capability declaration, operator setup only
   for values automated MIDI cannot hit, exact port matching.
3. **Tier A protocols implemented** — case builders, doctor probes where
   needed, validation thresholds, project creation via CLI.
4. **Extractors for Tier A** — at minimum:
   - oscillator-static NPZ/JSON (plan 15),
   - control-law curves (`raw → physical` tables + residuals),
   - envelope timing/shape summaries,
   - LFO rate table.
5. **Hardware session checklist** rehearsed: cabling, 96 kHz, gain staging,
   warm-up timer, unit ID fields, resume commands, spare time for failed takes.

Do not book a scarce Prophet day until gates 1–5 are green for the protocols
you intend to run that day. Tier B software may land between sessions.

## Relation to existing plans

| Document | Relationship |
| --- | --- |
| `15-automated-synth-reference-capture.md` | Host + first protocol (`oscillator-static-v1`). |
| `15-execution-phases.md` | Implementation phases; characterisation work follows Arturia acceptance. |
| `03-reference-capture-and-identification.md` (WIP tree) | Oscillator capture matrix inspiration; fitting/identification stay there. |
| `09-drift-variation-and-calibration.md` (WIP tree) | Consumes `drift-longform-v1` recordings; fitting not redefined here. |
| Oscillator candidate plans 04–08, 10–11 | Consume derived osc/filter features; not capture definitions. |

## Completion criteria

- Tier A protocol IDs, matrices, and extractor contracts are documented and
  implementable without inventing new host architecture.
- A Rev2 (or other) adapter can run the same Tier A suite as Arturia where
  capabilities overlap.
- A hardware session runbook and ship-gate checklist exist and are used before
  booking.
- Derived control-law and osc artifacts are attributable (target revision,
  mapping fingerprint, protocol revision, WAV checksums).
- No dataset claims to be “the Prophet” without a named unit profile.

## References

- Host plan: `plans/analog-osc/15-automated-synth-reference-capture.md`
- Execution phases: `plans/analog-osc/15-execution-phases.md`
- Oscillator research programme (WIP): `plans/analog-osc/00-master-plan.md`
- Capture/identification (WIP): `plans/analog-osc/03-reference-capture-and-identification.md`
- Drift/calibration (WIP): `plans/analog-osc/09-drift-variation-and-calibration.md`
- Sequential Prophet Rev2 User's Guide (MIDI/NRPN):
  <https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf>
- Arturia Prophet V manual (software rehearsal only):
  <https://downloads.arturia.net/products/prophet-v/manual/Prophet_V_Manual_3_0_0_EN.pdf>
