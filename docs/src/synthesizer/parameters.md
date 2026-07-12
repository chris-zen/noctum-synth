# Parameter Guide

This page describes controls by their musical purpose. The SDK section covers
the Rust parameter identifiers used by hosts.

## Oscillators and mixer

| Control | Use it for |
| --- | --- |
| Waveform | Choose saw, saw/triangle, triangle, or pulse as the starting harmonic content. |
| Frequency and fine tune | Set interval and detuning between the two main oscillators. |
| Shape | Vary the selected waveform's timbre. |
| Level and oscillator mix | Balance the two main oscillators. |
| Sub oscillator and noise | Add bass weight or noisy texture. |
| Hard sync | Reset oscillator 2 from oscillator 1 for harmonically intense sweep sounds. |
| Slop | Introduce analog-style pitch variation. |
| Glide | Slide pitched notes into their next pitch. |
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
voice. Amplifier velocity changes loudness response. Pan spread increases the
left/right placement applied to successive voices, while master volume sets the
final output level.

## LFOs

The four LFOs share the same control model:

| Control | Use it for |
| --- | --- |
| Waveform | Choose triangle, saw, reverse saw, square, or sample-and-hold motion. |
| Rate | Set the speed of the cyclic modulation. |
| Depth | Set its intensity. |
| Destination | Select the parameter the LFO controls. |
| Clock sync | Quantize modulation rate to musical divisions for sync-capable use. |
| Key sync | Restart the LFO with a new first-held note. |

When key sync is on, the LFO restarts when playing begins from silence; adding a
note while another is held does not restart the shared phase.
