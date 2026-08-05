# Pro Analog (Far Roadmap)

A hybrid instrument: digital control and effects on a microcontroller, with the
core voice path in real analog hardware.

The MCU runs the familiar Noctum control plane — MIDI, envelopes, LFOs,
modulation routing, patch storage, and global effects — while pitch, filter,
and amplitude are produced by analog components driven over CV. Platform, voice
count, and physical I/O are not yet determined.

## Single voice

USB MIDI and audio talk to one MCU. Control voltages leave through DACs into
the analog voice chain; the analog audio returns through a codec so the MCU can
apply digital effects before the final audio output.

```text
                      +----------+                    +---------------+
                      |   MCU    |                    |    Analog     |
USB MIDI/Audio <----> |          |                    |               |
                      | Control  | ----- DAC -------> |               |
                      |          |                    |  DCO/VCF/VCA  |
                      | Effects  | <-- Codec In ----- |               |
                      |          |                    +---------------+
                      |          |
                      |          | --- Codec Out ----> Audio Out
                      +----------+
```

## Multiple voices

For polyphony, a main MCU owns the USB host interface, voice allocation, the
final mix, and global effects. Each voice is its own board: a local MCU that
runs that voice's control (envelopes, LFOs, modulation) plus the analog
DCO/VCF/VCA. Voice audio returns to the main MCU, where the voices are mixed
before effects and output.

```text
                           +------------------+
 USB MIDI/Audio <--------> |    Main MCU      | --- Codec Out --> Audio Out
                           |                  |
                           |  Voice allocate  |
                           |  Mix + Effects   | <-- Codec In -------+
                           +--------+---------+                     |
                                    |                               |
                           control to each voice                    |
                                    |                               |
                 +------------------+------------------+            |
                 |                  |                  |            |
                 v                  v                  v            |
          +-------------+    +-------------+    +-------------+     |
          | Voice MCU   |    | Voice MCU   |    | Voice MCU   |     |
          |             |    |             |    |             |     |
          | Control     |    | Control     |    | Control     |     |
          |      |      |    |      |      |    |      |      |     |
          |     DAC     |    |     DAC     |    |     DAC     |     |
          |      v      |    |      v      |    |      v      |     |
          | Analog      |    | Analog      |    | Analog      |     |
          | DCO/VCF/VCA |    | DCO/VCF/VCA |    | DCO/VCF/VCA |     |
          +------+------+    +------+------+    +------+------+     |
                 |                  |                  |            |
                 +------------------+------------------+------------+
                                    |
                         voice audio mixed, then
                         returned to Main MCU
```

Control, timing, and modulation stay digital and precise. Tone generation and
filtering stay analog. Effects close the loop in the digital domain after the
analog path, so delay, reverb, and similar processing do not need an all-analog
implementation.

## Analog voice

The analog section is the subtractive chain — oscillators into a resonant
filter into a VCA. Pitch comes from digitally controlled oscillators (DCOs):
the voice MCU sets frequency with timers or other digital clocks so tuning
stays stable without expo-CV VCO calibration. Waveshape is still analog.
Filter cutoff, resonance, pulse width, and amplitude are driven by DAC CVs.
Chip choices are not locked; these are plausible starting points for a first
prototype:

| Role | Candidates | Notes |
| --- | --- | --- |
| Oscillator | MCU-timed DCO + analog waveshaper | Digital pitch (timer/clock); analog saw/pulse/triangle shaping |
| Filter | CEM3320 / Alfa AS3320 | Same CEM3320-class multimode filter family as the Rev2 target character; AS3320 is the current drop-in |
| VCA | SSI2164 / SSI2162, Alfa AS3360 | Quad/dual VCAs for amplitude and any extra CV-controlled levels |

A single voice board would typically carry two DCOs, one filter IC, and one
VCA channel (or a shared multi-VCA package), plus the usual op-amps and a
codec or ADC path back toward the main mix.

## Open hardware references

None of these are the same architecture as Pro Analog, but they are useful
prior art for MCU-timed DCOs, voice cards, and CEM3320-class filters:

| Project | Relevance |
| --- | --- |
| [polykit/pico-dco](https://github.com/polykit/pico-dco) | RP2040 PIO clocks a Juno-style analog DCO (saw/pulse); open schematics and PCB |
| [Polykit-6](https://polykit.rocks/open-source-analog-polyphonic-synthesizer/) | Open poly synth: main MCU + DAC CV into analog voice cards |
| [craigyjp/PolyKit-DUO](https://github.com/craigyjp/PolyKit-DUO-polyphonic-synthesizer) | Programmable 6-voice dual-DCO synth built on the Polykit DCO |
| [felipegaspari/DCO4_DCO](https://github.com/felipegaspari/DCO4_DCO) | 4 voices × 2 DCOs on RP2040, driven by a separate main controller |
| [KOSMO-POLY6](https://github.com/twinturbo/KOSMO-POLY6) | Modular Kosmo build path around the Pico DCO |
| [hermflink/3320VCF](https://github.com/hermflink/3320VCF) | Open CEM3320 / AS3320 filter schematic (Eurorack/DIY) |
