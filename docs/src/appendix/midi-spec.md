# Appendix: MIDI Spec

This appendix documents the MIDI protocol accepted by the synth. The **Prophet
Rev2** is the primary reference: live CC/NRPN control, patch editing, and MIDI
output all follow the Rev2 specification (Appendix E of the User's Guide). The
Rev2 SysEx program-image byte layout was reverse-engineered by Razmo and
published on the Sequential forum:

> **"Analyzing the REV2 SysEx structure"** (Nov 20, 2017)
> <https://forum.sequential.com/index.php?topic=2056.msg22693.html#msg22693>

The synth also accepts **Prophet '08** Program Data and Program Edit Buffer
SysEx messages. P08 programs are decoded into the same internal patch format as
Rev2 Layer A and are saved to persistent program memory on the hardware
(banks 4–5, mapped from P08 banks 0–1). See [Prophet '08
Compatibility](#prophet-08-compatibility) below.

See [Factory Presets](factory-presets.md) for instructions on loading the
official Sequential factory sound banks into hardware program memory.

## Protocol Overview

The synth accepts five categories of MIDI input. Channel messages are accepted
on any channel; System Real-Time messages have no MIDI channel.

| Protocol | Purpose |
|---|---|
| **CC** (Control Change) | Coarse 7-bit control of a subset of parameters |
| **NRPN** (Non-Registered Parameter Number) | High-resolution 14-bit access to every parameter |
| **Program Change** | Recall a stored program; CC0/CC32 select the global bank |
| **SysEx** (System Exclusive) | Bulk patch import/export and Program Data library imports |
| **System Real-Time** | External timing clock plus transport Start and Stop |

CC and NRPN assignments in this appendix describe the **Prophet Rev2** live
control protocol. Prophet '08 programs are imported via SysEx only; see
[Prophet '08 Compatibility](#prophet-08-compatibility).

## MIDI Clock

Clock mode is a device setting (not part of a patch). It is selected with
global NRPN 4099 using the Prophet Rev2 values 0–4:

| Mode | Behavior |
|---|---|
| Off | Ignore MIDI clock and use the patch BPM |
| Master | Generate outbound Timing Clock at the patch BPM |
| Slave | Follow Timing Clock (`F8`) to derive BPM and drive synchronized clocks; honor Start (`FA`) and Stop (`FC`) |
| Slave Thru | Same as Slave, and forward Timing Clock with low jitter |
| Slave No S/S | Follow Timing Clock (`F8`) to derive BPM and drive synchronized clocks; ignore Start and Stop |

Off and Master run from the patch BPM. Master transmits Timing Clock (`F8`) on
MIDI output at 24 PPQN. Slave, Slave Thru, and Slave No S/S measure incoming
Timing Clock at 24 PPQN to set the effective BPM and drive clock-synchronized
LFOs and effects. Until a tempo is acquired, the patch BPM remains effective.
If pulses stop for 500 ms, clock status changes to lost and transport stops,
while synchronized destinations retain the last learned tempo. Selecting Off
restores the latest patch BPM.

Slave Thru retransmits Timing Clock on MIDI output. MIDI Continue (`FB`) and
Song Position Pointer are ignored.

## NRPN Protocol

The synth tracks NRPN state independently for each of the 16 MIDI channels.
A complete parameter change requires four Control Change messages:

| CC | Role |
|---|---|
| 99 | NRPN parameter number MSB |
| 98 | NRPN parameter number LSB |
| 6  | Data Entry MSB |
| 38 | Data Entry LSB — triggers the parameter update |

The value is formed as `(MSB × 128) + LSB` and is clamped to the parameter's
maximum raw value before being converted to real-world units.

Data Increment (CC 96) and Data Decrement (CC 97) are also supported. They
adjust the current NRPN value by ±1, clamped to the parameter's valid range.

### NRPN / RPN Reset

The synth responds to the RPN Reset command: sending CC 100 value 127 followed
by CC 101 value 127 clears the current NRPN selection, returning it to a known
idle state.


## Value Conversion

Raw MIDI values are converted to real-world units using one of four formulas.
The same formulas are inverted when encoding output.

| Scale | Formula | Example parameters |
|---|---|---|
| **Linear 0–1** | `raw / max` | Volume, resonance, mix levels, pan spread |
| **Linear range** | `lo + (raw / max) × (hi − lo)` | Envelope times, semitone offsets, cents |
| **Logarithmic** | `lo × (hi / lo)^(raw / max)` | Filter cutoff (20 Hz – 20 kHz), LFO rate |
| **Bipolar** | `(raw / max) × 2 − 1` | Envelope amounts, modulation depth |

Booleans are decoded as `raw ≥ 64` (CC) or `raw ≠ 0` (NRPN).

#### Program Memory

The synth supports 8 persistent banks of 128 programs. CC0 and CC32 values
0–7 are both accepted as global Bank Select inputs; the selected bank is
consumed by the next Program Change. Sending a Program Data message saves
the decoded patch to its included bank/program address. Rev2 Program Data
saves to the bank encoded in the message (0–7). P08 Program Data saves to
hardware bank +4 (P08 bank 0 → hardware bank 4, P08 bank 1 → hardware bank 5).
Sending a Program Edit Buffer only loads the live patch and does not write
program memory.

## Performance Messages

Standard channel voice messages are supported:

| Message | Behaviour |
|---|---|
| Note On (velocity > 0) | Voice allocation |
| Note On (velocity = 0) | Treated as Note Off |
| Note Off | Voice release |
| Pitch Bend | 14-bit, normalised to −1.0 to +1.0 |
| Channel Pressure / Poly Key Pressure | 7-bit, normalised to 0.0 to 1.0 |

## Special CC Handlers

The following CC numbers are reserved for performance controls and are handled
before any parameter decoding:

| CC | Function |
|---|---|
| 1  | Mod Wheel (0–127 → 0%–100%) |
| 64 | Sustain Pedal (64–127 = on, 0–63 = off) |
| 120, 123 | All Notes Off |

## Prophet Rev2 Control Change Assignments

All CC values are 7-bit (0–127).

### Oscillators

| CC | Parameter | Range | Description |
|---|---|---|---|
| 8  | Sub Oscillator Level | 0–127 → 0%–100% | |
| 9  | Oscillator Slop | 0–127 → 0%–100% | Analog drift amount |
| 20 | Oscillator 1 Frequency | 0–127 → 0–120 note index | Coarse tuning |
| 21 | Oscillator 1 Fine Tune | 0–127 → −50 to +50 cents | |
| 22 | Oscillator 1 Shape | 0–127 → off / saw / saw-tri / tri / pulse | 5-position switch |
| 24 | Oscillator 2 Frequency | 0–127 → 0–120 note index | Coarse tuning |
| 25 | Oscillator 2 Fine Tune | 0–127 → −50 to +50 cents | |
| 26 | Oscillator 2 Shape | 0–127 → off / saw / saw-tri / tri / pulse | 5-position switch |
| 28 | Oscillator Mix | 0–127 → 0%–100% | Blend between osc 1 and osc 2 |
| 29 | Noise Level | 0–127 → 0%–100% | |
| 30 | Oscillator 1 Shape Mod | 0–127 → 0%–100% | Pulse width / waveshape depth |
| 31 | Oscillator 2 Shape Mod | 0–127 → 0%–100% | Pulse width / waveshape depth |

### Filter

| CC | Parameter | Range | Description |
|---|---|---|---|
| 74, 102 | Filter Cutoff | 0–127 → 20 Hz – 20 kHz | Logarithmic scale |
| 71, 103 | Filter Resonance | 0–127 → 0%–100% | |
| 104 | Filter Keyboard Track | 0–127 → 0%–100% | |
| 105 | Filter Audio Mod | 0–127 → 0%–100% | Oscillator 1 FM of filter cutoff |
| 106 | Filter Envelope Amount | 0–127 → −100% to +100% | Bipolar |
| 107 | Filter Envelope Velocity | 0–127 → 0%–100% | |
| 108 | Filter Envelope Delay | 0–127 → 0–5 seconds | |
| 109 | Filter Envelope Attack | 0–127 → 0.5 ms – 5 seconds | |
| 110 | Filter Envelope Decay | 0–127 → 0.5 ms – 5 seconds | |
| 111 | Filter Envelope Sustain | 0–127 → 0%–100% | |
| 112 | Filter Envelope Release | 0–127 → 0.5 ms – 10 seconds | |

### Amplifier

| CC | Parameter | Range | Description |
|---|---|---|---|
| 7, 37 | Master Volume | 0–127 → 0%–100% | |
| 10 | Pan Mod Mode | 0–63 = Alternate, 64–127 = Fixed | Boolean threshold at 64 |
| 75 | Amp Envelope Sustain | 0–127 → 0%–100% | |
| 76 | Amp Envelope Release | 0–127 → 0.5 ms – 10 seconds | |
| 113 | VCA Level | 0–127 → 0%–100% | Static VCA bias |
| 114 | Pan Spread | 0–127 → 0%–100% | Stereo voice width |
| 115 | Amp Envelope Amount | 0–127 → 0%–100% | |
| 116 | Amp Envelope Velocity | 0–127 → 0%–100% | |
| 117 | Amp Envelope Delay | 0–127 → 0–5 seconds | |
| 118 | Amp Envelope Attack | 0–127 → 0.5 ms – 5 seconds | |
| 119 | Amp Envelope Decay | 0–127 → 0.5 ms – 5 seconds | |

### Auxiliary Envelope

| CC | Parameter | Range | Description |
|---|---|---|---|
| 77 | Aux Envelope Sustain | 0–127 → 0%–100% | |
| 78 | Aux Envelope Release | 0–127 → 0.5 ms – 10 seconds | |
| 85 | Aux Envelope Destination | 0–127 → destination 0–52 | 53-way selector |
| 86 | Aux Envelope Amount | 0–127 → −100% to +100% | Bipolar |
| 87 | Aux Envelope Velocity | 0–127 → 0%–100% | |
| 88 | Aux Envelope Delay | 0–127 → 0–5 seconds | |
| 89 | Aux Envelope Attack | 0–127 → 0.5 ms – 5 seconds | |
| 90 | Aux Envelope Decay | 0–127 → 0.5 ms – 5 seconds | |

### Effects

| CC | Parameter | Range | Description |
|---|---|---|---|
| 3  | Effect Type | 0–127 → type 0–12 | 13-position selector |
| 12 | Effect Parameter 1 | 0–127 → 0%–100% | |
| 13 | Effect Parameter 2 | 0–127 → 0%–100% | |
| 16 | Effect Enabled | 0–63 = off, 64–127 = on | Boolean |
| 17 | Effect Mix | 0–127 → 0%–100% | |

## Prophet Rev2 NRPN Assignments

NRPN values are 14-bit (0–16383). Each parameter has a documented maximum raw
value shown in the Max column. Values beyond this maximum are clamped.

### Oscillators

| NRPN | Parameter | Raw Range | Max |
|---|---|---|---|
| 0  | Oscillator 1 Frequency | 0–120 note index | 120 |
| 1  | Oscillator 1 Fine Tune | −50 to +50 cents | 100 |
| 2  | Oscillator 1 Shape | 0=off, 1=saw, 2=saw-tri, 3=tri, 4=pulse | 4 |
| 5  | Oscillator 2 Frequency | 0–120 note index | 120 |
| 6  | Oscillator 2 Fine Tune | −50 to +50 cents | 100 |
| 7  | Oscillator 2 Shape | 0=off, 1=saw, 2=saw-tri, 3=tri, 4=pulse | 4 |
| 10 | Hard Sync | 0=off, 1=on | 1 |
| 12 | Oscillator Slop | 0–127 → 0%–100% | 127 |
| 13 | Oscillator Mix | 0–127 → 0%–100% | 127 |
| 14 | Noise Level | 0–127 → 0%–100% | 127 |
| 99 | Oscillator 1 Note Reset | 0=off, 1=on | 1 |
| 100 | Pitch Bend Range | 0–12 semitones | 12 |
| 102 | Oscillator 1 Shape Mod | 0–99 → 0%–100% | 99 |
| 103 | Oscillator 2 Shape Mod | 0–99 → 0%–100% | 99 |
| 104 | Oscillator 2 Note Reset | 0=off, 1=on | 1 |
| 110 | Sub Oscillator Level | 0–127 → 0%–100% | 127 |

Oscillator Frequency (`0–120`) is a panel note index (C0…C10), not a bipolar
offset from MIDI 60.

### Filter

| NRPN | Parameter | Raw Range | Max |
|---|---|---|---|
| 15 | Filter Cutoff | 0–164 → 20 Hz – 20 kHz (log) | 164 |
| 16 | Filter Resonance | 0–127 → 0%–100% | 127 |
| 17 | Filter Keyboard Track | 0–127 → 0%–100% | 127 |
| 18 | Filter Audio Mod | 0–127 → 0%–100% | 127 |
| 19 | Filter Poles | 0=2-pole, 1=4-pole | 1 |
| 20 | Filter Envelope Amount | 0–254 → −100% to +100% (bipolar) | 254 |
| 21 | Filter Envelope Velocity | 0–127 → 0%–100% | 127 |
| 22 | Filter Envelope Delay | 0–127 → 0–5 s | 127 |
| 23 | Filter Envelope Attack | 0–127 → 0.5 ms – 5 s | 127 |
| 24 | Filter Envelope Decay | 0–127 → 0.5 ms – 5 s | 127 |
| 25 | Filter Envelope Sustain | 0–127 → 0%–100% | 127 |
| 26 | Filter Envelope Release | 0–127 → 0.5 ms – 10 s | 127 |

### Amplifier

| NRPN | Parameter | Raw Range | Max |
|---|---|---|---|
| 28 | Pan Spread | 0–127 → 0%–100% | 127 |
| 29 | Master Volume | 0–127 → 0%–100% | 127 |
| 30 | Amp Envelope Amount | 0–127 → 0%–100% | 127 |
| 31 | Amp Envelope Velocity | 0–127 → 0%–100% | 127 |
| 32 | Amp Envelope Delay | 0–127 → 0–5 s | 127 |
| 33 | Amp Envelope Attack | 0–127 → 0.5 ms – 5 s | 127 |
| 34 | Amp Envelope Decay | 0–127 → 0.5 ms – 5 s | 127 |
| 35 | Amp Envelope Sustain | 0–127 → 0%–100% | 127 |
| 36 | Amp Envelope Release | 0–127 → 0.5 ms – 10 s | 127 |

### Auxiliary Envelope

| NRPN | Parameter | Raw Range | Max |
|---|---|---|---|
| 57 | Aux Envelope Destination | 0–52 (53 destinations) | 52 |
| 58 | Aux Envelope Amount | 0–254 → −100% to +100% (bipolar) | 254 |
| 59 | Aux Envelope Velocity | 0–127 → 0%–100% | 127 |
| 60 | Aux Envelope Delay | 0–127 → 0–5 s | 127 |
| 61 | Aux Envelope Attack | 0–127 → 0.5 ms – 5 s | 127 |
| 62 | Aux Envelope Decay | 0–127 → 0.5 ms – 5 s | 127 |
| 63 | Aux Envelope Sustain | 0–127 → 0%–100% | 127 |
| 64 | Aux Envelope Release | 0–127 → 0.5 ms – 10 s | 127 |
| 97 | Aux Envelope Loop | 0=off, 1=on | 1 |

### LFOs 1–4

Each LFO uses a group of 5 consecutive NRPNs. The table shows the offset within
each group. LFO 1 starts at NRPN 37, LFO 2 at 42, LFO 3 at 47, LFO 4 at 52.

| Offset | Parameter | Raw Range | Max |
|---|---|---|---|
| +0 | LFO Rate | Unsynced: 0–150 → 0.022–500 Hz (log). Synced: buckets of 8 select 32 steps through 1/16 step; 128–150 clamp to 1/16 step. | 150 |
| +1 | LFO Waveform | 0=triangle, 1=saw, 2=reverse saw, 3=square, 4=random | 4 |
| +2 | LFO Depth | 0–127 → 0%–100% | 127 |
| +3 | LFO Destination | 0–52 (53 destinations) | 52 |
| +4 | LFO Clock Sync | 0=off, 1=on | 1 |

LFO Key Sync uses separate NRPNs: 105 (LFO 1), 106 (LFO 2), 107 (LFO 3),
108 (LFO 4). All are 0=off, 1=on.

In clock-sync mode the bucket starts 0, 8, 16, …, 120 select: 32, 16, 8,
6, 4, 3, 2, 1.5, 1, 2/3, 1/2, 1/3, 1/4, 1/6, 1/8, and 1/16 step.
The effective LFO frequency is BPM / 60 × Clock Divide steps per quarter ×
cycles per selected step.

### Free Modulation Slots 1–8

Each slot uses 3 consecutive NRPNs. Slot 1 starts at NRPN 65, slot 2 at 68,
up to slot 8 at 86.

| Offset | Parameter | Raw Range | Max |
|---|---|---|---|
| +0 | Modulation Source | 0–22 (off / 22 sources) | 22 |
| +1 | Modulation Amount | 0–254 → −100% to +100% (bipolar) | 254 |
| +2 | Modulation Destination | 0–52 (53 destinations) | 52 |

### Dedicated Modulation Slots

Starting at NRPN 116, each slot uses 2 NRPNs. The slots are Mod Wheel (116),
Pressure (118), Breath (120), Velocity (122), and MIDI Foot (124).

| Offset | Parameter | Raw Range | Max |
|---|---|---|---|
| +0 | Modulation Amount | 0–254 → −100% to +100% (bipolar) | 254 |
| +1 | Modulation Destination | 0–52 (53 destinations) | 52 |

### Effects

| NRPN | Parameter | Raw Range | Max |
|---|---|---|---|
| 153 | Effect Enabled | 0=off, 1=on | 1 |
| 154 | Effect Type | 0–12 (13 effect types; enable/disable is NRPN 153) | 12 |
| 155 | Effect Mix | 0–127 → 0%–100% | 127 |
| 156 | Effect Parameter 1 | 0–255 → 0%–100% | 255 |
| 157 | Effect Parameter 2 | 0–127 → 0%–100% | 127 |
| 158 | Effect Clock Sync | 0=off, 1=on | 1 |

## SysEx Messages

Both Prophet Rev2 and Prophet '08 use Sequential's standard SysEx framing and
7-bit packing (see [Seven-Bit Unpacking](#seven-bit-unpacking)). The model ID
byte distinguishes the two formats. Program Data messages from either synth are
saved to persistent hardware program memory.

### Seven-Bit Unpacking

MIDI SysEx data bytes must have their high bit clear (values 0–127 only).
Sequential synth program data consists of 8-bit bytes, so it must be packed
before transmission.

**The scheme:** take 7 raw bytes, strip off the top bit of each, collect those
7 bits into one header byte, then send the header followed by the 7 stripped
data bytes. 7 raw bytes → 8 MIDI-safe bytes.

| Byte | Contents |
|---|---|
| 0 (header) | Bit 6 = MSB of raw byte 0, bit 5 = MSB of raw byte 1, ..., bit 0 = MSB of raw byte 6 |
| 1 | Raw byte 0, low 7 bits (bits 6–0) |
| 2 | Raw byte 1, low 7 bits |
| 3 | Raw byte 2, low 7 bits |
| 4 | Raw byte 3, low 7 bits |
| 5 | Raw byte 4, low 7 bits |
| 6 | Raw byte 5, low 7 bits |
| 7 | Raw byte 6, low 7 bits |

To reconstruct raw byte N: take packed byte N+1 and set its bit 7 to the
corresponding bit from the header.

This repeats for each group of 7 raw bytes. Last group is padded if needed.
For the Prophet Rev2: 2,339 packed bytes unpack to 2,046 raw bytes. For the
Prophet '08: 439 packed bytes unpack to 384 raw bytes.

### Prophet Rev2 SysEx

#### Supported Messages

| Command | Name | Size |
|---|---|---|
| `0x02` | Program Data (stored patch import) | 2,346 bytes |
| `0x03` | Program Edit Buffer (live patch load/send) | 2,344 bytes |

Framing: `F0 01 2F <cmd> ... F7`

Program Data messages include bank (0–7) and program (0–127) bytes after the
command byte. Program Edit Buffer messages omit bank and program.

#### Prophet Rev2 Program Image Layout

The unpacked Rev2 program image is 2,046 bytes (1,024 per layer). The synth
decodes Layer A only; Layer B is ignored. The byte offsets below were
reverse-engineered by Razmo and verified against the Rev2 v1.0 factory bank.

**Important:** NRPN parameter numbers do not match SysEx byte offsets. The
offsets below are the unpacked byte positions within Layer A. Where a parameter
shares semantics with the Prophet '08, the value descriptions follow the
Prophet '08 User's Guide; Rev2-specific differences (byte layout, slop range,
LFO waveform order, mod destination count, and so on) are called out inline.

##### Layer A (bytes 0–1023)

| Offset | Parameter | Range / Values |
|---|---|---|
| 0 | Oscillator 1 Frequency | 0–120 note index |
| 1 | Oscillator 2 Frequency | 0–120 note index |
| 2 | Oscillator 1 Fine Tune | 0–100 (−50 to +50 cents; 50 = centered) |
| 3 | Oscillator 2 Fine Tune | 0–100 (−50 to +50 cents; 50 = centered) |
| 4 | Oscillator 1 Shape | 0 = off, 1 = saw, 2 = saw/tri mix, 3 = triangle, 4 = pulse |
| 5 | Oscillator 2 Shape | 0 = off, 1 = saw, 2 = saw/tri mix, 3 = triangle, 4 = pulse |
| 6 | Oscillator 1 Shape Mod | 0–99 (pulse width / waveshape depth) |
| 7 | Oscillator 2 Shape Mod | 0–99 (pulse width / waveshape depth) |
| 8 | Oscillator 1 Glide | 0–127 |
| 9 | Oscillator 2 Glide | 0–127 |
| 10 | Oscillator 1 Keyboard | 0 = off, 1 = on |
| 11 | Oscillator 2 Keyboard | 0 = off, 1 = on |
| 12 | Oscillator 1 Note Reset | 0 = off, 1 = on |
| 13 | Oscillator 2 Note Reset | 0 = off, 1 = on |
| 14 | Oscillator Mix | 0–127 |
| 15 | Sub Oscillator Level | 0–127 |
| 16 | Noise Level | 0–127 |
| 17 | Hard Sync | 0 = off, 1 = on |
| 18 | Glide Mode | 0 = fixed rate, 1 = fixed rate auto, 2 = fixed time, 3 = fixed time auto |
| 19 | Glide On/Off | 0 = off, 1 = on |
| 20 | Pitch Bend Range | 0–12 semitones |
| 21 | Oscillator Slop | 0–127 |
| 22 | Filter Cutoff | 0–164 (logarithmic; 20 Hz – 20 kHz when decoded) |
| 23 | Filter Resonance | 0–127 |
| 24 | Filter Keyboard Track | 0–127 |
| 25 | Filter Audio Mod | 0–127 |
| 26 | Filter Poles | 0 = 2-pole, 1 = 4-pole |
| 27 | VCA Level | 0–127 |
| 28 | Program Volume (low 7 bits); bit 7 is MSB for Aux Envelope Amount | 0–127 |
| 29 | Pan Spread | 0–127 |
| 30 | Aux Envelope Destination (low 7 bits); bit 7 is MSB for Filter Envelope Amount | 0–52 destination index |
| 31 | Aux Envelope Loop | 0 = off, 1 = on |
| 32 | Filter Envelope Amount (low 7 bits; MSB in byte 30) | 0–254 (−127 to +127) |
| 33 | Amp Envelope Amount | 0–127 |
| 34 | Aux Envelope Amount (low 7 bits; MSB in byte 28) | 0–254 (−127 to +127) |
| 35 | Filter Envelope Velocity | 0–127 |
| 36 | Amp Envelope Velocity | 0–127 |
| 37 | Aux Envelope Velocity | 0–127 |
| 38 | Filter Envelope Delay | 0–127 (0–5 s when decoded) |
| 39 | Amp Envelope Delay | 0–127 (0–5 s when decoded) |
| 40 | Aux Envelope Delay | 0–127 (0–5 s when decoded) |
| 41 | Filter Envelope Attack | 0–127 (0.5 ms – 5 s when decoded) |
| 42 | Amp Envelope Attack | 0–127 (0.5 ms – 5 s when decoded) |
| 43 | Aux Envelope Attack | 0–127 (0.5 ms – 5 s when decoded) |
| 44 | Filter Envelope Decay | 0–127 (0.5 ms – 5 s when decoded) |
| 45 | Amp Envelope Decay | 0–127 (0.5 ms – 5 s when decoded) |
| 46 | Aux Envelope Decay | 0–127 (0.5 ms – 5 s when decoded) |
| 47 | Filter Envelope Sustain | 0–127 |
| 48 | Amp Envelope Sustain | 0–127 |
| 49 | Aux Envelope Sustain | 0–127 |
| 50 | Filter Envelope Release | 0–127 (0.5 ms – 10 s when decoded) |
| 51 | Amp Envelope Release | 0–127 (0.5 ms – 10 s when decoded) |
| 52 | Aux Envelope Release | 0–127 (0.5 ms – 10 s when decoded) |
| 53 | LFO 1 Rate | 0–150; interpreted as free rate or a synchronized bucket according to byte 69 |
| 54 | LFO 2 Rate | same as LFO 1 |
| 55 | LFO 3 Rate | same as LFO 1 |
| 56 | LFO 4 Rate | same as LFO 1 |
| 57 | LFO 1 Waveform | 0 = triangle, 1 = saw, 2 = reverse saw, 3 = square, 4 = random |
| 58 | LFO 2 Waveform | same as LFO 1 |
| 59 | LFO 3 Waveform | same as LFO 1 |
| 60 | LFO 4 Waveform | same as LFO 1 |
| 61 | LFO 1 Depth | 0–127 |
| 62 | LFO 2 Depth | 0–127 |
| 63 | LFO 3 Depth | 0–127 |
| 64 | LFO 4 Depth | 0–127 |
| 65 | LFO 1 Destination | 0–52 (modulation destination index) |
| 66 | LFO 2 Destination | 0–52 |
| 67 | LFO 3 Destination | 0–52 |
| 68 | LFO 4 Destination | 0–52 |
| 69 | LFO 1 Clock Sync | 0 = off, 1 = on |
| 70 | LFO 2 Clock Sync | 0 = off, 1 = on |
| 71 | LFO 3 Clock Sync | 0 = off, 1 = on |
| 72 | LFO 4 Clock Sync | 0 = off, 1 = on |
| 73 | LFO 1 Key Sync | 0 = off, 1 = on |
| 74 | LFO 2 Key Sync | 0 = off, 1 = on |
| 75 | LFO 3 Key Sync | 0 = off, 1 = on |
| 76 | LFO 4 Key Sync | 0 = off, 1 = on |
| 77–84 | Mod Slots 1–8 Source | 0–22 (modulation source index) |
| 85–92 | Mod Slots 1–8 Amount | 0–254 (−127 to +127) |
| 93–100 | Mod Slots 1–8 Destination | 0–52 |
| 101 | Mod Wheel Amount | 0–254 (−127 to +127) |
| 102 | Mod Wheel Destination | 0–52 |
| 103 | Pressure Amount | 0–254 (−127 to +127) |
| 104 | Pressure Destination | 0–52 |
| 105 | Breath Amount | 0–254 (−127 to +127) |
| 106 | Breath Destination | 0–52 |
| 107 | Velocity Amount | 0–254 (−127 to +127) |
| 108 | Velocity Destination | 0–52 |
| 109 | MIDI Foot Amount | 0–254 (−127 to +127) |
| 110 | MIDI Foot Destination | 0–52 |
| 111–114 | Gated Sequencer 1–4 Destination | 0–52 |
| 115 | Effect Type | 0 = delay mono, 1 = DDL stereo, 2 = BBD delay, 3 = chorus, 4 = phaser high, 5 = phaser low, 6 = phaser mst, 7 = flanger 1, 8 = flanger 2, 9 = reverb, 10 = ring mod, 11 = distortion, 12 = high-pass filter (use byte 116 to enable/disable) |
| 116 | Effect Enabled | 0 = off, 1 = on |
| 117 | Effect Mix | 0–127 |
| 118 | Effect Parameter 1 | 0–255 |
| 119 | Effect Parameter 2 | 0–127 |
| 120 | Effect Clock Sync | 0 = off, 1 = on |
| 121 | (unused) | — |
| 122 | Key Mode | 0 = low, 1 = high, 2 = last, 3 = low retrigger, 4 = high retrigger, 5 = last retrigger |
| 123 | Unison On/Off | 0 = off, 1 = on |
| 124 | Unison Mode | 0 = 1 voice through 15 = 16 voices, 16 = chord memory |
| 125–129 | (unused) | — |
| 130 | BPM | 30–250 |
| 131 | Clock Divide | 0 = half note, 1 = quarter, 2 = eighth, 3 = eighth half swing, 4 = eighth full swing, 5 = eighth triplets, 6 = sixteenth, 7 = sixteenth half swing, 8 = sixteenth full swing, 9 = sixteenth triplets, 10 = thirty-second, 11 = thirty-second triplets, 12 = sixty-fourth triplets |
| 132 | Arpeggiator Mode | 0 = up, 1 = down, 2 = up/down, 3 = assign, 4 = random |
| 133 | Arpeggiator Range | 0–2 |
| 134 | Arpeggiator Repeats | 0–3 |
| 135 | Arpeggiator Relatch | 0 = off, 1 = on |
| 136 | Arpeggiator On/Off | 0 = off, 1 = on |
| 137 | (unused) | — |
| 138 | Sequencer Mode | 0–4 |
| 139 | Sequencer Type | 0 = gated, 1 = poly |
| 140–155 | Gated Seq 1 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 156–171 | Gated Seq 2 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 172–187 | Gated Seq 3 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 188–203 | Gated Seq 4 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 204–207 | (unused) | — |
| 208 | Unison Detune | 0–16 |
| 209 | Pan Mod Mode | 0 = alternate, 1 = fixed |
| 210–230 | (unused) | — |
| 231 | Layer Mode | 0 = layer A, 1 = split AB, 2 = stack AB |
| 232 | Split Point | 0–120 (MIDI note number) |
| 233–234 | (unused) | — |
| 235–254 | Layer A Name | 20 ASCII characters |
| 255 | (unused) | — |
| 256–319 | Poly Seq Track 1 Notes (16 steps) | MIDI note per step |
| 320–383 | Poly Seq Track 1 Velocities (16 steps) | 0–127 per step |
| 384–447 | Poly Seq Track 2 | notes and velocities |
| 448–511 | Poly Seq Track 3 | notes and velocities |
| 512–575 | Poly Seq Track 4 | notes and velocities |
| 576–639 | Poly Seq Track 5 | notes and velocities |
| 640–703 | Poly Seq Track 6 | notes and velocities |
| 704–767 | Gated Seq details | 4 tracks × 16 steps |
| 768–1023 | Poly Seq Track details | 6 tracks × 4 voices |

**Split-MSB sidebands:** Bipolar values (max 254) and other values above 127
store their low 7 bits in one byte and bit 8 in bit 7 of a *different* byte
(that host byte's own value occupies bits 0–6). Some parameters use bit 7 of
their own byte instead. Byte 27 (VCA Level) is a plain 0–127 field with no
MSB sideband. Offsets above 255 are sequencer data and are not imported.

| Value byte | Parameter | MSB stored in byte | Host parameter at MSB byte |
|---|---|---|---|
| 32 | Filter Envelope Amount | 30 | Aux Envelope Destination |
| 34 | Aux Envelope Amount | 28 | Program Volume |
| 85 | Mod Slot 1 Amount | 89 | Mod Slot 5 Amount |
| 86 | Mod Slot 2 Amount | 88 | Mod Slot 4 Amount |
| 87 | Mod Slot 3 Amount | 87 | Mod Slot 3 Amount (same byte) |
| 88 | Mod Slot 4 Amount | 86 | Mod Slot 2 Amount |
| 89 | Mod Slot 5 Amount | 85 | Mod Slot 1 Amount |
| 90 | Mod Slot 6 Amount | 84 | Mod Slot 8 Source |
| 91 | Mod Slot 7 Amount | 97 | Mod Slot 5 Destination |
| 92 | Mod Slot 8 Amount | 96 | Mod Slot 4 Destination |
| 101 | Mod Wheel Amount | 101 | Mod Wheel Amount (same byte) |
| 103 | Pressure Amount | 99 | Mod Slot 7 Destination |
| 105 | Breath Amount | 111 | Gated Sequencer 1 Destination |
| 107 | Velocity Amount | 109 | MIDI Foot Amount |
| 109 | MIDI Foot Amount | 107 | Velocity Amount |

Mod slot amounts 1–5 and dedicated Velocity/Foot amounts form reciprocal pairs
(each byte's bit 7 holds the other's MSB). Mod slots 6–8 borrow bit 7 from a
source or destination byte instead.

##### Layer B

Layer B occupies bytes 1024–2043 and follows the same layout as Layer A. The
synth currently ignores Layer B.

## Prophet Rev2 Unsupported Features

The following Prophet Rev2 systems are not implemented by this synth:

- Layer B parameter control
- Sequencer and arpeggiator
- Rev2 SysEx import/export of the undocumented chord-memory voicing bytes
- Global settings (tuning, MIDI channel, pedal config, etc.)
- Program memory management (save, rename, bank copy)
- Alternate tunings

## Prophet '08 Compatibility

The synth accepts Prophet '08 Program Data and Program Edit Buffer SysEx for
importing factory banks and loading patches from a Prophet '08 editor. Imported
programs are converted into the internal Rev2-style patch format (Layer A
parameters only) and are saved to persistent program memory on the hardware
(P08 bank 0 → hardware bank 4, P08 bank 1 → hardware bank 5). Live MIDI
control uses the Prophet Rev2 protocol above; only SysEx import is supported
for Prophet '08 parameters.

Reference: [Prophet '08 User's Guide v1.0](http://www.dsisynth.com/misc/Prophet_08_Manual_v1.0.pdf),
SysEx Messages (page 51) and Program Parameter Data (page 44).

### Prophet '08 SysEx Messages

| Command | Name | Size |
|---|---|---|
| `0x02` | Program Data (stored patch save/import) | 446 bytes |
| `0x03` | Program Edit Buffer (live patch load) | 444 bytes |

Framing: `F0 01 23 <cmd> ... F7`

Program Data messages include bank (0–1) and program (0–127) bytes after the
command byte. Program Edit Buffer messages omit bank and program. A Program
Edit Buffer message can be formed from a Program Data message by changing the
command byte from `02` to `03` and removing the bank and program bytes.

439 packed bytes unpack to 384 raw bytes.

### Prophet '08 Program Image Layout

The unpacked Prophet '08 program image is 384 bytes: Layer A (bytes 0–199) and
Layer B (bytes 200–383). The synth decodes Layer A only. Layer B uses the same
field layout at offset +200 (for example, Output Spread is byte 28 in Layer A
and byte 228 in Layer B).

Bipolar values (max 254) and other values above 127 use the same split-MSB
sideband scheme as Rev2 (low 7 bits in one byte, bit 8 in bit 7 of another).
All documented MSB sidebands are listed below.

#### Layer A (bytes 0–199)

| Offset | Parameter | Range / Values |
|---|---|---|
| 0 | Oscillator 1 Frequency | 0–120 note index |
| 1 | Oscillator 1 Fine Tune | 0–100 (−50 to +50 cents; 50 = centered) |
| 2 | Oscillator 1 Shape | 0 = off, 1 = saw, 2 = triangle, 3 = saw/tri mix, 4–103 = pulse (width 0–99) |
| 3 | Oscillator 1 Glide | 0–127 |
| 4 | Oscillator 1 Keyboard | 0 = off, 1 = on |
| 5 | Oscillator 2 Frequency | 0–120 note index |
| 6 | Oscillator 2 Fine Tune | 0–100 (−50 to +50 cents; 50 = centered) |
| 7 | Oscillator 2 Shape | 0 = off, 1 = saw, 2 = triangle, 3 = saw/tri mix, 4–103 = pulse (width 0–99) |
| 8 | Oscillator 2 Glide | 0–127 |
| 9 | Oscillator 2 Keyboard | 0 = off, 1 = on |
| 10 | Hard Sync | 0 = off, 1 = on |
| 11 | Glide Mode | 0 = fixed rate, 1 = fixed rate auto, 2 = fixed time, 3 = fixed time auto |
| 12 | Oscillator Slop | 0–5 |
| 13 | Oscillator 1–2 Mix | 0–127 |
| 14 | Noise Level (low 7 bits); bit 7 is MSB for Filter Envelope Amount | 0–127 |
| 15 | Filter Cutoff (low 7 bits; MSB in byte 19) | 0–164 (logarithmic; 20 Hz – 20 kHz when decoded) |
| 16 | Filter Resonance | 0–127 |
| 17 | Filter Keyboard Amount | 0–127 |
| 18 | Filter Audio Modulation | 0–127 |
| 19 | Filter Poles (low 7 bits); bit 7 is MSB for Filter Cutoff and Mod Wheel Amount | 0 = 2-pole, 1 = 4-pole |
| 20 | Filter Envelope Amount (low 7 bits; MSB in byte 14) | 0–254 (−127 to +127) |
| 21 | Filter Envelope Velocity | 0–127 |
| 22 | Filter Envelope Delay | 0–127 (0–5 s when decoded) |
| 23 | Filter Envelope Attack | 0–127 (0.5 ms – 5 s when decoded) |
| 24 | Filter Envelope Decay | 0–127 (0.5 ms – 5 s when decoded) |
| 25 | Filter Envelope Sustain | 0–127 |
| 26 | Filter Envelope Release | 0–127 (0.5 ms – 10 s when decoded) |
| 27 | VCA Initial Level | 0–127 |
| 28 | Output Spread | 0–127 |
| 29 | Voice Volume | 0–127 |
| 30 | VCA Envelope Amount | 0–127 |
| 31 | VCA Envelope Velocity | 0–127 |
| 32 | VCA Envelope Delay | 0–127 (0–5 s when decoded) |
| 33 | VCA Envelope Attack | 0–127 (0.5 ms – 5 s when decoded) |
| 34 | VCA Envelope Decay | 0–127 (0.5 ms – 5 s when decoded) |
| 35 | VCA Envelope Sustain | 0–127 |
| 36 | VCA Envelope Release | 0–127 (0.5 ms – 10 s when decoded) |
| 37 | LFO 1 Frequency (low 7 bits; MSB in byte 39) | 0–150 = unsynced rate, 151–166 = clock-synced divisions |
| 38 | LFO 1 Shape | 0 = triangle, 1 = reverse saw, 2 = saw, 3 = square, 4 = random |
| 39 | LFO 1 Amount (low 7 bits); bit 7 is MSB for LFO 1 Frequency | 0–127 |
| 40 | LFO 1 Destination | 0–43 (modulation destination index) |
| 41 | LFO 1 Key Sync | 0 = off, 1 = on |
| 42 | LFO 2 Frequency (low 7 bits; MSB in byte 48) | same as LFO 1 |
| 43 | LFO 2 Shape (low 7 bits); bit 7 is MSB for LFO 3 Frequency | same as LFO 1 |
| 44 | LFO 2 Amount | 0–127 |
| 45 | LFO 2 Destination | 0–43 |
| 46 | LFO 2 Key Sync | 0 = off, 1 = on |
| 47 | LFO 3 Frequency (low 7 bits; MSB in byte 43) | same as LFO 1 |
| 48 | LFO 3 Shape (low 7 bits); bit 7 is MSB for LFO 2 Frequency | same as LFO 1 |
| 49 | LFO 3 Amount | 0–127 |
| 50 | LFO 3 Destination | 0–43 |
| 51 | LFO 3 Key Sync | 0 = off, 1 = on |
| 52 | LFO 4 Frequency (low 7 bits; MSB in same byte); bit 7 is also MSB for Pressure Amount | same as LFO 1 |
| 53 | LFO 4 Shape | same as LFO 1 |
| 54 | LFO 4 Amount | 0–127 |
| 55 | LFO 4 Destination | 0–43 |
| 56 | LFO 4 Key Sync | 0 = off, 1 = on |
| 57 | Envelope 3 Destination | 0–43 |
| 58 | Envelope 3 Amount (low 7 bits; MSB in byte 60) | 0–254 (−127 to +127) |
| 59 | Envelope 3 Velocity | 0–127 |
| 60 | Envelope 3 Delay (low 7 bits); bit 7 is MSB for Envelope 3 Amount | 0–127 |
| 61 | Envelope 3 Attack | 0–127 |
| 62 | Envelope 3 Decay | 0–127 |
| 63 | Envelope 3 Sustain (low 7 bits); bit 7 is MSB for Mod 2 Amount | 0–127 |
| 64 | Envelope 3 Release | 0–127 |
| 65 | Mod 1 Source | 0–20 (modulation source index) |
| 66 | Mod 1 Amount | 0–254 (−127 to +127) |
| 67 | Mod 1 Destination | 0–43 |
| 68 | Mod 2 Source | 0–20 |
| 69 | Mod 2 Amount (low 7 bits; MSB in byte 63) | 0–254 (−127 to +127) |
| 70 | Mod 2 Destination | 0–43 |
| 71 | Mod 3 Source (low 7 bits); bit 7 is MSB for Mod 4 Amount | 0–20 |
| 72 | Mod 3 Amount (low 7 bits; MSB in byte 74) | 0–254 (−127 to +127) |
| 73 | Mod 3 Destination | 0–43 |
| 74 | Mod 4 Source (low 7 bits); bit 7 is MSB for Mod 3 Amount | 0–20 |
| 75 | Mod 4 Amount (low 7 bits; MSB in byte 71) | 0–254 (−127 to +127) |
| 76 | Mod 4 Destination | 0–43 |
| 77 | Sequence 1 Destination | 0–43 |
| 78 | Sequence 2 Destination | 0–43 |
| 79 | Sequence 3 Destination | 0–43 |
| 80 | Sequence 4 Destination | 0–43 |
| 81 | Mod Wheel Amount (low 7 bits; MSB in byte 19) | 0–254 (−127 to +127) |
| 82 | Mod Wheel Destination | 0–43 |
| 83 | Pressure Amount (low 7 bits; MSB in byte 52) | 0–254 (−127 to +127) |
| 84 | Pressure Destination | 0–43 |
| 85 | Breath Amount (low 7 bits; MSB in byte 89) | 0–254 (−127 to +127) |
| 86 | Breath Destination | 0–43 |
| 87 | Velocity Amount (low 7 bits; MSB in same byte) | 0–254 (−127 to +127) |
| 88 | Velocity Destination | 0–43 |
| 89 | Foot Control Amount (low 7 bits; MSB in byte 85) | 0–254 (−127 to +127) |
| 90 | Foot Control Destination | 0–43 |
| 91 | BPM | 30–250 |
| 92 | Clock Divide | 0 = half note, 1 = quarter, 2 = eighth, 3 = eighth half swing, 4 = eighth full swing, 5 = eighth triplets, 6 = sixteenth, 7 = sixteenth half swing, 8 = sixteenth full swing, 9 = sixteenth triplets, 10 = thirty-second, 11 = thirty-second triplets, 12 = sixty-fourth triplets |
| 93 | Pitch Bend Range | 0–12 semitones |
| 94 | Sequencer Trigger | 0 = normal, 1 = normal no reset, 2 = no gate, 3 = no gate/no reset, 4 = key step |
| 95 | Key Mode | 0 = low priority, 1 = low retrigger, 2 = high priority, 3 = high retrigger, 4 = last note, 5 = last note retrigger |
| 96 | Unison Mode | 0 = 1 voice, 1 = all voices, 2 = all detune 1, 3 = all detune 2, 4 = all detune 3 |
| 97 | Arpeggiator Mode | 0 = up, 1 = down, 2 = up/down, 3 = assign |
| 98 | Envelope 3 Repeat | 0 = off, 1 = on |
| 99 | Unison | 0 = off, 1 = on |
| 100 | Arpeggiator | 0 = off, 1 = on |
| 101 | Gated Sequencer | 0 = off, 1 = on |
| 102–117 | (unused) | — |
| 118 | Split Point | 0–127 (60 = C3) |
| 119 | Keyboard Mode | 0 = normal 8-voice, 1 = stack, 2 = split |
| 120–135 | Sequence Track 1 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 136–151 | Sequence Track 2 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 152–167 | Sequence Track 3 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 168–183 | Sequence Track 4 Steps 1–16 | 0–125 = step value, 126 = reset, 127 = rest |
| 184–199 | Program Name | 16 ASCII characters (bytes 32–127) |

**Split-MSB sidebands (Prophet '08):**

| Value byte | Parameter | MSB stored in byte | Host parameter at MSB byte |
|---|---|---|---|
| 15 | Filter Cutoff | 19 | Filter Poles |
| 20 | Filter Envelope Amount | 14 | Noise Level |
| 37 | LFO 1 Frequency | 39 | LFO 1 Amount |
| 42 | LFO 2 Frequency | 48 | LFO 3 Shape |
| 47 | LFO 3 Frequency | 43 | LFO 2 Shape |
| 52 | LFO 4 Frequency | 52 | LFO 4 Frequency (same byte) |
| 58 | Envelope 3 Amount | 60 | Envelope 3 Delay |
| 69 | Mod 2 Amount | 63 | Envelope 3 Sustain |
| 72 | Mod 3 Amount | 74 | Mod 4 Source |
| 75 | Mod 4 Amount | 71 | Mod 3 Source |
| 81 | Mod Wheel Amount | 19 | Filter Poles |
| 83 | Pressure Amount | 52 | LFO 4 Frequency |
| 85 | Breath Amount | 89 | Foot Control Amount |
| 87 | Velocity Amount | 87 | Velocity Amount (same byte) |
| 89 | Foot Control Amount | 85 | Breath Amount |

Filter Poles (byte 19) and LFO 4 Frequency (byte 52) each host two MSB
sidebands because their own values never use bit 7. Mod 3/4 source and
Breath/Foot amount bytes form reciprocal pairs.

Imported voice parameters cover offsets 0–36, 37–56 (LFOs), 57–64
(auxiliary envelope), 65–76 (free modulation slots), and 81–90 (dedicated
modulation), plus the program name at bytes 184–199. Byte 27 (VCA Initial
Level) is a plain 0–127 field with no MSB sideband. Key mode and the
Prophet '08 unison fields are applied: the three fixed detune modes map to
eight Rev2-style voices at progressively larger detune values. The two
oscillator Glide rates and Glide mode are imported, with Glide enabled when
either rate is nonzero. Sequencer, arpeggiator, tempo, and split settings are
present in the image but not applied.

#### Layer B (bytes 200–383)

Layer B follows the same layout at offset +200. The synth currently ignores
Layer B.

### Prophet '08 Unsupported Features

The following Prophet '08 systems are not implemented:

- Layer B parameter control
- Live MIDI control of Prophet '08 parameters (SysEx only, no CC/NRPN)
- Sequencer and arpeggiator
- Split and stack program modes
- Global settings (tuning, MIDI channel, pedal config, etc.)
- Program memory management (save, rename, bank copy)
