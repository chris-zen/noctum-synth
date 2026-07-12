# Synthesizer Capabilities

Analog Synth is a 16-voice subtractive polysynth. Each played note has its own
oscillator phase, filter and envelope movement, while the instrument shares a
single set of patch controls and global effects.

It is designed for the familiar range of subtractive synthesis: solid basses,
sync leads, resonant sweeps, animated pads, and stereo textures. The sound
starts with oscillator tone, gains character from the filter and envelopes, and
becomes dynamic through LFO, envelope, and performance-control modulation.

## At a glance

| Section | What it provides |
| --- | --- |
| Voices | 16 notes of polyphony with predictable voice stealing and stereo spread. |
| Oscillators | Two analog-style oscillators, sub oscillator, white noise, hard sync, drift, and glide. |
| Filter | Resonant 2-pole or 4-pole ladder low-pass filtering with key, velocity, envelope, and audio modulation. |
| Envelopes | Filter, amplifier, and auxiliary DADSR envelopes. The auxiliary envelope can repeat. |
| LFOs | Four LFOs with five waveforms, rate, depth, destination, clock sync, and key sync. |
| Modulation | Eight freely assigned routes plus five dedicated performance routes. |
| Effects | Delay, modulation, reverb, ring modulation, distortion, and high-pass processing. |

## Playing behavior

The synth is polyphonic. A new key uses an available voice; released voices are
reused before a held voice is stolen. If every voice is still held, the oldest
held voice is replaced. Playing the same note again retriggers its existing
voice rather than stacking a duplicate note.

Velocity shapes the amplifier and filter response when those controls are set
above zero. Pitch bend, mod wheel, pressure, breath, foot controller, and
expression can each become modulation sources. A sustain pedal retains released
notes until it is lifted.
