# Rev2 Control Calibrations

Noctum uses these Prophet Rev2–compatible control laws for pitch modulation,
filter tracking, and envelope timing so patches and MIDI decode match hardware
where those laws are established.

## Oscillator frequency modulation

Two amount scales apply when the destination is oscillator frequency.

### LFO Amount → Osc Freq

Use when an LFO’s **own destination** is Osc 1 / Osc 2 / Osc All Freq.

| LFO Amount | Musical span |
| ---: | --- |
| 8 | 1 semitone |
| 96 | 1 octave |

At full Amount (127), peak swing is ≈ 15.875 semitones.

### Mod matrix / Env 3 → Osc Freq

Use for free or dedicated matrix slots and for the auxiliary (Env 3)
envelope destination.

| Matrix / Env amount | Musical span |
| ---: | --- |
| 2 | 1 semitone |
| 24 | 1 octave |

At full amount (±127), span is ±63.5 semitones.

**Cross-check:** LFO Amount **4** matches matrix Amount **1**.

An LFO used as a **matrix** source (source = LFO, dest = Osc Freq) uses the
matrix table. LFO Amount still scales the LFO waveform; matrix Amount sets the
pitch range of that route.

## Filter cutoff and keyboard tracking

| Spec | Value |
| --- | --- |
| Cutoff step | 1 semitone per raw tick |
| A4 reference | raw **105** = 440 Hz |
| Cutoff max | raw **164** |
| Key Amount unity | raw **64** = 1:1 keyboard tracking |
| Key Amount 64 + Cutoff 0 at C4 | filter near C2 (−2 octaves) |
| Key Amount 64 + Cutoff 24 | self-osc / cutoff tracks the played note |
| Audio Mod full | ±1 octave cutoff sweep from Osc 1 |

## Envelope timing

| Segment | Raw 0 | Raw 127 |
| --- | --- | --- |
| Attack / decay | ≈ 3 ms | ≈ 24.66 s |
| Release | ≈ 3 ms | ≈ 40 s |

Intermediate values interpolate between measured anchors. Delay stays 0–5 s in
Rev2 MIDI mapping. Development harness knobs use the same attack/decay/release
maxima.

## Note Number as a mod source

Note Number starts at MIDI note 0.

| Goal | Rough Note Num amount |
| --- | ---: |
| Match Osc Key Tracking (on) | ~256 to Osc Freq (often split across slots) |
| Match Key Amount 64 to cutoff | ~128 to Filter Cutoff |

A negative DC offset may be needed so the first physical key starts at a useful
baseline.

## Open calibrations

- Osc Slop depth and free-running animation are not yet at Rev2 scale.
- Hardware velocity and pressure curves reshape the keybed before the engine;
  they do not reshape received MIDI. Host MIDI into Noctum stays linear unless
  an input-curve setting is added.
