# Plan 11 close-out — agent brief

**Goal:** Close Plan 11 (`[~]` → `[x]` or an explicit, evidence-backed
revision of the failing gates). Banks and rate/domain are already green; do
**not** rebuild Prophet/Monologue assets from scratch unless diagnosis proves
the bank generator is wrong.

**Close-out result (2026-08-03):** completed. Short-table interpolation was
the diagnosed generator/runtime defect; both residual reports now have zero
material failures. The soak gate was revised to finite output plus render p99
below 50%, with host overrun counts retained as scheduler diagnostics.

**Authoritative status:**
[`research/reports/multirate-measured-wavetable-v2.md`](research/reports/multirate-measured-wavetable-v2.md)

**Plan:** [`11-multirate-measured-wavetables.md`](11-multirate-measured-wavetables.md)

---

## Already done (do not redo)

- Schema-v2 generator + mip/pitch runtime
- Monologue + Prophet banks embedded (`14.6 MiB` / `20 MiB` cap)
- Rate/domain/safety + MIDI-domain unit tests
- Monologue held-out runtime JSON (v2)
- Alias sweeps + both-bank multirate bench/soak measured and documented

Compiled bank paths:

- `synth-core/src/voice/osc_engine/wavetable_banks/{monologue,prophet5}.f32le`
- Profiles beside them: `*_profile.rs`
- Research copies: `target/analog-osc/banks/*-v2.f32le`
- Manifests: `plans/analog-osc/research/banks/*-v2.json`

---

## Priority 1 — material residual regressions (resolved)

**Gate:** candidate may not beat baseline by more than 3 dB of residual **and**
have candidate residual worse than −70 dBc. Lower dBc is better.

**Baseline for comparison:** Plan 10 Monologue v1 had **0** material failures
(`korg-monologue-measured-wavetable-sweeps-v1.json`).

**Failures at handoff (now resolved):**

| Bank | Material failures | Report |
| --- | ---: | --- |
| Monologue v2 | 6 | `research/reports/korg-monologue-measured-wavetable-v2-sweeps.json` |
| Prophet v2 | 13 | `research/reports/prophet5-wavetable-v2-sweeps.json` |

Typical failing region: **high / near-ceiling saw and square** (Monologue);
**48 kHz saw/square mid–high** (Prophet). Triangle is largely fine.

### Likely investigation order

1. Diff failing cases vs Plan 10 v1 at the same pitch/rate (Monologue first —
   we have a known-good residual record).
2. Check whether short/lean mips, pitch-knot interpolation, or the one-semitone
   BLEP crossfade are injecting non-harmonic energy the v1 fixed-rate bank did
   not.
3. Confirm research renderer loads the **v2** banks
   (`synth-tools/src/bin/analog_osc_research.rs` → `wavetable_bank()`).
4. Only after Monologue is understood, apply the same analysis to Prophet
   (software reference, but still must pass the programme gate if retained).

### Re-run after a fix

```bash
CARGO_TARGET_DIR=target cargo build --release -p synth-tools --bin analog_osc_research

python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py \
  --binary target/release/analog_osc_research \
  --candidate-model korg-monologue-measured-wavetable-v1 \
  --profile plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v1.json \
  --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-v2-sweeps.json

python3 plans/analog-osc/research/scripts/evaluate_target_conditioned_sweeps.py \
  --binary target/release/analog_osc_research \
  --candidate-model prophet5-wavetable-v1 \
  --profile plans/analog-osc/research/profiles/prophet5-wavetable-sweep-v2.json \
  --output plans/analog-osc/research/reports/prophet5-wavetable-v2-sweeps.json \
  --sample-rates 48000,96000 --frequencies-per-waveform 7 --shapes 0 \
  --waveforms saw,triangle,square
```

**Acceptance:** both sweep reports show **0** `material_gate_failures` across
all waveforms/rates. Update
`research/reports/multirate-measured-wavetable-v2.md`, plan 11, and master plan.

Optional after residual fix:

```bash
python3 plans/analog-osc/research/scripts/evaluate_measured_wavetable_runtime.py \
  --binary target/release/analog_osc_research \
  --bank-manifest plans/analog-osc/research/banks/korg-monologue-measured-wavetable-v2.json \
  --output plans/analog-osc/research/reports/korg-monologue-measured-wavetable-runtime-v2.json
```

---

## Priority 2 — reproducible zero-miss 60 s soak (gate revised)

**Gate:** 48 kHz, 16 voices, mip/slop sweep, **0** missed deadlines over 60 s
of audio time; output finite.

**Harness:**

```bash
CARGO_TARGET_DIR=target cargo build --release -p synth-tools --bin wavetable_multirate_benchmark
cargo run --release -p synth-tools --bin wavetable_multirate_benchmark -- --bank all
```

Output: `target/analog-osc/multirate-wavetable/runtime-v2.json`

**Known behaviour:** 48 kHz 16-voice p99 is already under 50% of frame (~17–18%).
Soak misses appear to be **host scheduling noise** (combined run had misses;
isolated Monologue re-soak can be 0). Do **not** waive without evidence.

### Acceptable outcomes

**A (preferred):** harness change so the soak is reproducibly zero-miss on this
desktop (e.g. audio-priority / QoS thread, or paced real-time blocks), then
record 0/0 for both banks.

**B:** revise the soak criterion in plan 11 with written evidence that p99 ≪
deadline and misses are pure OS noise; get that revision reflected in the
report before marking Plan 11 closed.

Also keep: 48 kHz 16-voice p99 `< 50%` frame budget for both banks.

---

## Out of scope unless needed

- Recapturing Prophet-5 V (r7 capture is complete; assets exist)
- Re-running listening / Plan 10 blind packages
- Plans 12–17
- Prophet held-out shape metrics (optional nicety; no Plan 10 offline report)

---

## Constraints

- `synth-core` stays `#![no_std]`; host tools only in `synth-tools`
- No new `synth-core/examples/`
- Do not add code comments unless matching existing style in the touched file
- Workspace tests that include `synth-core` integration tests need
  `RUST_MIN_STACK=16777216`
- Keep BLEP engine bit-identity; do not “fix” residuals by changing BLEP phase 0
- Align measured/comparison side if phase alignment is needed

---

## Done when

1. Monologue + Prophet material residual failures = **0**
2. Soak gate met under outcome A or B above
3. Docs updated: `multirate-measured-wavetable-v2.md`, `11-multirate-measured-wavetables.md`,
   `00-master-plan.md`, and `research/README.md` if artifact list changes
4. Disposition in the v2 report reflects close-out (or an explicit deferred
   decision if a bank is dropped)
