# Noctum

Noctum is a virtual-analog polysynth inspired by the Sequential Prophet Rev2: a
subtractive instrument with two oscillators per voice, a resonant low-pass
filter, three envelopes, four LFOs, flexible modulation routing, stereo spread,
and global effects. It is an original implementation, not a clone.

Noctum is available in different hardware [models](models/overview.md), each
running the same sound engine and sharing the same patch format and MIDI
protocol. Currently shipping: **Micro 4**, a four-voice instrument on the Daisy
Seed 1.1.

## What it sounds like

Two analog-style oscillators per voice — saw, triangle, pulse, and blended
shapes — feed a resonant ladder low-pass filter with switchable 2-pole and
4-pole slopes. Three DADSR envelopes and four LFOs animate pitch, timbre,
levels, and effects. Eight freely assignable modulation routes plus five
dedicated performance routes let you build patches that respond to velocity,
aftertouch, mod wheel, and more.

Stereo output with per-voice pan spread places chords across the field. Global
effects — delay, chorus, flanger, phaser, reverb, ring modulation, distortion,
and high-pass filtering — process the mixed voice output.

16-voice polyphony on the desktop application. Hardware models balance voice
count against CPU budget to keep a consistent sound character.

## Choosing a model

Start with [Models](models/overview.md) to compare the hardware lineup. Every
model shares the same [sound architecture](synthesizer/overview.md),
[parameter set](synthesizer/parameters.md), and [MIDI
protocol](appendix/midi-spec.md).

| Section | Who it's for |
| --- | --- |
| [Models](models/overview.md) | Choosing hardware, understanding voice/rate trade-offs |
| [Synthesizer](synthesizer/overview.md) | Learning the instrument's capabilities and sound |
| [Hardware](hardware/overview.md) | Building, flashing, and debugging firmware |
| [Application](application.md) | Running the desktop development harness |
| [SDK](sdk/getting-started.md) | Embedding `synth-core` in a host or working on the DSP |
| [Appendix](appendix/midi-spec.md) | MIDI protocol, SysEx, and factory-presset import |
