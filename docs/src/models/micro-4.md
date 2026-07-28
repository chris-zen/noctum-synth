# Micro 4

The current shipping model. A four-voice polyphonic virtual-analog synthesizer
built on the Daisy Seed 1.1 (STM32H750 Cortex-M7 at 480 MHz).

## Audio

Output runs at **48 kHz**. The internal sound engine operates at **24 kHz** and
is reconstructed to the output rate through a half-band interpolator. This keeps
four voices within the Cortex-M7's DSP budget while retaining full MIDI and
patch compatibility.

A USB Audio Class 1 capture interface mirrors the post-effects stereo output as
packed 24-bit PCM at 48 kHz when a host opens the stream. The analog DAC is
always active.

## Voices and polyphony

Four independently articulated notes. Voice allocation follows a standard
last-note priority scheme: released voices are reused before held voices are
stolen. Playing the same note again retriggers its existing voice rather than
stacking a duplicate. Unison stacks all four voices onto a single pitch with
detune spread.

## Low-pass filter

**Gain-Limited TPT** — a four-stage trapezoidal integration cascade with
analytic, gain-limited feedback. Switchable between 2-pole (gentler) and 4-pole
(steeper) slopes. Self-oscillating at high resonance settings. Supports keyboard
tracking, envelope amount, velocity sensitivity, and audio-rate modulation from
oscillator 1.

This is the CPU-floor filter candidate selected for the STM32H750. The desktop
application offers four additional filter models for auditioning during
development; the Micro 4 firmware compiles only Gain-Limited TPT
(`filter-gain-limited`).

## Build

```sh
cd hardware/daisy
make run-micro-4
make bench-dsp-micro-4
make bench-factory-banks-micro-4
```

Feature set: `fast-math,wide-4,downsampling,filter-gain-limited`. See the
[Daisy Seed](../hardware/daisy.md) guide for the full flag list.

## MIDI and program storage

USB MIDI in both directions on all channels. Supports the full Prophet Rev2
CC, NRPN, and SysEx protocol described in the [MIDI Spec](../appendix/midi-spec.md).

Programs are stored in the Daisy's 8 MiB QSPI flash: **8 banks × 128 programs**
(1,024 total slots) with persistent read-modify-write storage. Bank Select
(CC0/CC32) followed by Program Change recalls a program. Rev2 and Prophet '08
Program Data SysEx messages save patches to their addressed slot. The last
loaded program is restored on power-up.

MIDI clock modes: Off (patch BPM), Slave (follow external Timing Clock), and
Slave No S/S (follow clock, ignore Start/Stop). Continue, Master, and Slave Thru
are not implemented on this model.

## Physical I/O

| Connection | Purpose |
| --- | --- |
| USB (micro-B) | MIDI, UAC1 audio capture, firmware updates (DFU) |
| 3.5 mm TRS | Stereo line output (WM8731 codec) |
| Debug header | probe-rs flashing and RTT logging |

An onboard user LED indicates MIDI activity (dim pulse) and audio overruns
(bright triple flash).

## Compatibility

Patches are interchangeable with the desktop application and all other Noctum
models. The Micro 4 uses the same `synth-core` engine, the same `Patch` format,
and the same Rev2 MIDI codec.
