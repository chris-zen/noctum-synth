# Micro 1 (Planned)

A monophonic variant of the Micro platform built on the same Daisy Seed 1.1.

With one voice, the internal engine runs at the full **48 kHz** rate without
the half-band interpolator used by the Micro 4. Oscillators and the filter
therefore operate at double the Micro 4 internal bandwidth, trading polyphony
for fidelity.

## Low-pass filter

**Huovilainen Ladder** — nonlinear Moog-style ladder reference. Self-oscillating
at high resonance. Same modulation controls as Micro 4 (keyboard tracking,
envelope amount, velocity, audio-rate mod from oscillator 1).

## Other characteristics

MIDI protocol, program storage, USB audio capture, and physical I/O are shared
with the [Micro 4](micro-4.md).

Like Micro 4, it preserves complete two-layer programs while compiling exactly
one physical voice and one effects region. Recall selects Layer A; NRPN 4190
selects B. Stored Stack/Split programs remain intact and report degraded status
because only the selected component is rendered.

## Build

```sh
cd hardware/daisy
make run-micro-1
make bench-dsp-micro-1
make bench-factory-banks-micro-1
```

Feature set: `fast-math,wide-1,filter-huovilainen`. See the
[Daisy Seed](../hardware/daisy.md) guide for the full flag list.
