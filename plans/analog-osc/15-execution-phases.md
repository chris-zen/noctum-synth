# Synth-capture execution phases

Working copy of the phased plan (also tracked in Cursor plans). Source of truth for
product behavior remains `15-automated-synth-reference-capture.md`.

## Process

After each phase:

1. Run the phase review gate.
2. Do an adversarial, defect-first review of that phase’s diff.
3. Fix all P0–P2 findings (and cheap/safe P3s) before starting the next phase.
4. Re-run the review gate after fixes.

## Phases

1. Crate scaffold, domain, protocol matrix — **done**
2. Project persistence and read-only CLI — **done**
3. MIDI transport and Arturia Prophet-5 V adapter — **done**
4. Audio capture, validation, runner, simulated E2E — **done**
5. Doctor, full CLI, terminal UX — **done**
6. Rust extraction and numerical parity — **blocked on live Arturia acceptance**
7. Supply-chain finish, docs, Arturia acceptance

### Live Arturia acceptance (before Phase 6)

In progress against Prophet-5 V via IAC + BlackHole. Adapter revision **4**
drops Fine Tune, Pulse Width, and Filter Env Amount from the MIDI map
(`prepare_session` / operator setup instead). Fresh 226-case capture is the gate
before extraction work.

### After Arturia acceptance (characterisation programme)

Full-voice measurement protocols, hardware session runbooks, and ship gates
before booking a real Prophet (or any other synth with an adapter) are defined
in [`16-full-voice-characterisation.md`](16-full-voice-characterisation.md).
Do not start Tier A protocol implementation for hardware day until plan 15
Phases 6–7 are complete enough to prove the host.

See `/Users/chris/.cursor/plans/capture_tool_phases_0e8505b9.plan.md` for full
phase goals, work items, and review gates.
