# Arturia Prophet-5 V capture audit v1

Date: 2026-08-02  
Disposition: **revision-5 capture and derived bank invalid; recapture required**

## What failed

The malformed wavetable is not primarily a display problem. The earliest
corruption is already present in the recorded WAVs and repeats at similar MIDI
notes across saw, triangle, and pulse. The old extraction and bank stages then
failed to stop that data from reaching the runtime.

1. Adapter revision 5 reset generic Detune under the misleading name
   `voice_dispersion`, but did not own Prophet-5 V's actual pitch, pulse-width,
   filter, envelope, modulation, or level dispersion controls. It also left
   advanced modulation, arp/chord, and all three FX slots preset-dependent.
2. Extraction revision 1 chose the steepest raw upward crossing. Harmonic-rich
   waves may have several such crossings per period, so the landmark could jump
   between different features.
3. The Arturia bank was limited for a 96 kHz playback rate but exposed to a
   48 kHz engine. Harmonics retained up to 43.2 kHz can alias at 48 kHz.
4. The bank tool defaulted to a hard-coded main-checkout path, so another
   worktree could silently consume stale derived data.
5. Acceptance examined isolated notes and pitch accuracy, not waveform
   coherence over the complete chromatic grid.

## Evidence

Phase-aligned adjacent-cycle correlations in the revision-5 derived set were
approximately:

| Wave | Median | Minimum |
| --- | ---: | ---: |
| Saw | 0.794 | 0.335 |
| Triangle | 0.988 | 0.890 |
| Pulse | 0.819 | 0.507 |

The Monologue reference is roughly 0.998 or better on the same check. A
phase-invariant spectral comparison also showed severe Arturia outliers. With
the new automatic gate, the old set is rejected immediately at saw rows 10/12
(36.677/41.168 Hz): spectral cosine 0.8884, below the required 0.90.

The shared pitch-dependent RMS/shape pattern across independently captured
waveforms is consistent with uncontrolled common plugin state, such as effects
or dispersion, rather than legitimate waveform-table evolution.

## Repairs

- Adapter revision 7 and the supplied `.promidi` now reset the actual
  dispersion controls, advanced modulation sections, arp/chord, and FX
  Dry/Wet/bypass state. Oscillator 2 is captured at its maximum level (CC 127)
  and master level is 110. A fresh project is mandatory.
- Extraction revision 2 detects phase on a half-period moving-average proxy,
  which suppresses upper partials, then resamples the original audio.
- Bank generation uses a 48 kHz playback reference by default and records that
  policy in the manifest and regenerated Rust profile metadata.
- Runtime profiles carry their build reference rate. Incompatible playback
  rates fall back instead of using unsafe tables.
- The bank builder rejects adjacent training cycles with phase-invariant
  spectral cosine below 0.90.
- Default paths are repository-relative.

## Required recapture gate

Create `plans/analog-osc/research/captures/arturia-prophet5-v1-r7` from Init
after importing the current `.promidi`. Confirm the operator prompts, verify FX
bypass/Dry-Wet visually after `doctor`, capture all 226 cases, extract with
revision 2, and build the bank at the default 48 kHz reference. Do not copy or
rename revision-5 WAVs, NPZs, manifests, or banks into the new project.
