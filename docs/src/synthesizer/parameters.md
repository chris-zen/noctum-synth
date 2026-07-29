# Parameter Guide

This page describes controls by their musical purpose. The SDK section covers
the Rust parameter identifiers used by hosts.

## Oscillators and mixer

| Control | Use it for |
| --- | --- |
| Waveform | Choose saw, saw/triangle, triangle, or pulse as the starting harmonic content. |
| Frequency and fine tune | Set interval and detuning between the two main oscillators. Frequency is a note index (0–120). |
| Shape Mod | Vary pulse width / waveshape depth. |
| Level and oscillator mix | Balance the two main oscillators. |
| Sub oscillator and noise | Add bass weight or noisy texture. |
| Hard sync | Reset oscillator 2 from oscillator 1 for harmonically intense sweep sounds. |
| Slop | Introduce analog-style pitch variation. |
| Glide | Slide from the previously played pitch. Rate sets both oscillators; the individual oscillator controls can then diverge. Fixed Rate scales duration with interval, Fixed Time does not, and the two Auto modes glide only while playing legato. |
| Note reset and keyboard tracking | Choose repeatable phase starts and whether oscillator pitch follows the keyboard. |

## Filter

| Control | Use it for |
| --- | --- |
| Cutoff | Set the brightness by choosing how much high-frequency content passes. |
| Resonance | Emphasize the cutoff point; high settings can self-oscillate. |
| Poles | Select a gentler 2-pole or steeper 4-pole roll-off. |
| Key tracking | Open the filter progressively on higher keys. |
| Envelope amount | Apply the filter envelope to cutoff. Positive amounts open with the envelope. |
| Velocity | Make harder notes create more filter movement. |
| Audio modulation | Add oscillator-1-rate modulation for aggressive, inharmonic tones. |

The filter oversampling setting is an engine-quality choice exposed by the
development application. It is not a normal patch-performance control.

## Envelopes and stereo

All three envelopes are DADSR envelopes: delay, attack, decay, sustain, and
release. The filter envelope controls cutoff movement, the amplifier envelope
shapes loudness, and the auxiliary envelope is a freely assignable modulation
source.

The auxiliary envelope has a destination and amount. Its velocity setting
makes that amount respond to key velocity, and Repeat loops its attack/decay
portion for cycling modulation.

Amplifier envelope amount controls how strongly the volume contour gates a
voice. VCA Level sets a static amplifier bias before the envelope is applied; at
full level the envelope amount has no effect, which is useful for drone or
bypass-style patches. Amplifier velocity changes loudness response. Filter envelope
amount, filter velocity, and auxiliary envelope amount and velocity use the same
short (~5 ms) de-zippering on live knob or MIDI changes; patch recall snaps those
values instantly. Pan spread increases the
left/right placement applied to successive voices. Program volume sets each
layer's stored output level after effects. Master volume is a device-global
listening level applied after the output limiter and is not stored in the patch.
Live program-volume and master-volume changes use the same short (~5 ms)
de-zippering as amplifier and filter amount knobs; patch recall snaps program
volume instantly.

## Unison and key mode

Unison changes the engine from polyphonic allocation to one selected note
played by 1–16 voices. Detune spreads those voices symmetrically around the
played pitch; stacked voices are intentionally not gain-normalized. Low, High,
and Last key modes choose which physically held note controls the stack. Their
Retrigger variants restart the envelopes on every note-on, while the other
modes retune legato without restarting the sound.

Chord mode stores the intervals of a held chord from its lowest note. A single
key then transposes that voicing. Press Unison while Chord mode is selected and
notes are held to replace the stored voicing. Native patches preserve the
chord; the supplied Rev2 MIDI documentation does not identify the external
program bytes that contain it.

## Master clock

BPM sets the internal clock from 30–250 beats per minute. Clock Divide selects
the duration of one master-clock step, from a half note through a sixty-fourth
note triplet; swing divisions use the same nominal continuous rate as their
straight equivalents. These values drive clock-synchronized LFOs and the BPM
also drives synchronized effects.

## LFOs

The four LFOs share the same control model:

| Control | Use it for |
| --- | --- |
| Waveform | Choose triangle, saw, reverse saw, square, or sample-and-hold motion. |
| Rate | Set free-running frequency in hertz, or select a step ratio while synchronized. |
| Depth | Set its intensity. |
| Destination | Select the parameter the LFO controls. |
| Clock sync | Lock modulation frequency to BPM, Clock Divide, and the selected step ratio. |
| Key sync | Restart the LFO with a new first-held note. |

When key sync is on, the LFO restarts when playing begins from silence; adding a
note while another is held does not restart the shared phase.

The synchronized ratios are 32, 16, 8, 6, 4, 3, 2, 1.5, 1, 2/3, 1/2, 1/3,
1/4, 1/6, 1/8, and 1/16 step. Each LFO remembers its free-running hertz value
and synchronized ratio independently. Rate modulation is ignored while clock
sync is active so the LFO remains locked to the master clock.
