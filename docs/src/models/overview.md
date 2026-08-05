# Models

Noctum is available in different hardware models. Every model runs the same sound
engine and shares the same patch format, MIDI protocol, and factory-presset
compatibility. They differ in platform, voice count, internal sample rate, and
physical I/O.

The naming convention combines the platform with the maximum voice count:
**Micro 4** is a Daisy Seed model with four voices, **Micro 1** will be the
same platform configured for a single voice.

| Model | Platform | Voices | Internal Rate | Output Rate | Status |
| --- | --- | --- | --- | --- | --- |
| [Micro 4](micro-4.md) | Daisy Seed 1.1 | 4 | 24 kHz | 48 kHz | Supported |
| [Micro 1](micro-1.md) | Daisy Seed 1.1 | 1 | 48 kHz | 48 kHz | Supported |
| [Mini](mini.md) | Raspberry Pi Zero 2 | TBD | TBD | TBD | Planned — awaiting stock to evaluate |
| [Pro Digital](pro-digital.md) | Raspberry Pi 4 | TBD | TBD | TBD | Planned |
| [Pro Analog](pro-analog.md) | TBD | TBD | TBD | TBD | Far roadmap |

Models with fewer voices can run the engine at a higher internal rate on the
same hardware because fewer voice lanes consume fewer CPU cycles.
