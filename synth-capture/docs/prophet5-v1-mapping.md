# Prophet-5 V capture adapter (v1)

**Identity warning:** Prophet-5 V is a **software** instrument. Captures
are not Sequential/Prophet hardware measurements and must not be labeled as
hardware Prophet-5 or Rev2 references. Machine-readable identity:
[`prophet5-v1-target.json`](prophet5-v1-target.json).

Target id: `prophet5-v1`  
Adapter revision: matches `ADAPTER_REVISION` in `prophet5_v1.rs` (currently **7**).

## Routing

```text
synth-capture MIDI output
  -> virtual MIDI port (exact name)
  -> Prophet-5 V standalone
  -> virtual audio cable
  -> synth-capture audio input (float32 @ 96 kHz)
```

## MIDI Learn

Configure Prophet-5 V for **absolute** CC response (not relative/toggle). Prefer
importing [`Noctum-Characterisation.promidi`](Noctum-Characterisation.promidi)
(MIDI config menu → Import) instead of learning each control by hand.

`paramid` values are **not** in the user manual. They are the 0-based index of
parameters in the installed
`/Library/Arturia/Prophet-5 V/resources/Reference_ParamNames.xml` (`<vst>` list).
`channel="0"` means MIDI channel 1.

Prophet-5 V has no Osc 1 triangle; CC 103 (`osc1_triangle`) is sent by the
adapter during reset but has no `.promidi` assignment (ignored by the plugin).

Changing any CC or neutral value requires bumping `ADAPTER_REVISION` and creating
a new project.

| CC | Semantic | Neutral |
| --- | --- | ---: |
| 102 | osc1_saw | 0 |
| 103 | osc1_triangle | 0 |
| 104 | osc1_pulse | 0 |
| 105 | osc1_level | 0 |
| 106 | osc2_saw | 0 |
| 107 | osc2_triangle | 0 |
| 108 | osc2_pulse | 0 |
| 109 | osc2_level | 127 |
| 111 | osc2_keyboard_tracking | 127 |
| 112 | osc2_lo_freq | 0 |
| 13 | filter_keyboard_tracking | 0 |
| 114 | noise_level | 0 |
| 115 | osc_sync | 0 |
| 116 | filter_cutoff | 127 |
| 117 | filter_resonance | 0 |
| 119 | amp_attack | 0 |
| 14 | amp_decay | 0 |
| 15 | amp_sustain | 127 |
| 16 | amp_release | 0 |
| 17 | filter_attack | 0 |
| 18 | filter_decay | 0 |
| 19 | filter_sustain | 0 |
| 20 | filter_release | 0 |
| 21 | unison | 0 |
| 22 | oscillator_detune | 0 |
| 23 | master_level | 110 |
| 24 | polymod_osc2_amount | 0 |
| 25 | polymod_noise_amount | 0 |
| 26 | lfo_amount | 0 |
| 27 | polymod_dest_freq1 | 0 |
| 28 | polymod_dest_pw1 | 0 |
| 29 | polymod_dest_filter | 0 |
| 30 | lfo_dest_freq | 0 |
| 31 | lfo_dest_pw | 0 |
| 80 | modulations_enable | 0 |
| 81 | keyboard_modulations_enable | 0 |
| 82 | pitch_dispersion | 0 |
| 83 | pulse_width_dispersion | 0 |
| 84 | filter_cutoff_dispersion | 0 |
| 85 | filter_resonance_dispersion | 0 |
| 86 | envelope_time_dispersion | 0 |
| 87 | modulation_dispersion | 0 |
| 88 | level_dispersion | 0 |
| 89 | fx1_dry_wet | 0 |
| 90 | fx2_dry_wet | 0 |
| 91 | fx3_dry_wet | 0 |
| 92 | arpeggiator_enable | 0 |
| 93 | chord_enable | 0 |
| 94 | fx1_bypass | 127 |
| 95 | fx2_bypass | 127 |
| 96 | fx3_bypass | 127 |

The mapping fingerprint is the SHA-256 of the sorted `cc\\tsemantic\\tneutral` lines.

## Operator setup (manual)

Start from the factory Init preset, import the revision-7 `.promidi`, and use a
**new capture project**. After `doctor` resets the target, visually verify that
all three FX slots are bypassed and have Dry/Wet at zero. Revision 5 did not own
these parameters and its captures are invalid for wavetable generation.

These continuous controls cannot be centered with 7-bit CC and are **not** in
the MIDI map. At the start of each `doctor` / `run` session the tool asks once
for the operator to set:

- Oscillator 2 Fine Tune to exactly `0.000`
- Oscillator 2 Pulse Width to exactly `50%`
- Filter Envelope Amount to exactly `5.0` (bipolar center / no env→cutoff)

Automated `reset()` does not touch those controls. Protocol pulse cases still
request 50% width semantically; the adapter treats that as a no-op.

Remaining MIDI neutrals are endpoints or intentional non-center levels (e.g.
cutoff fully open, amp sustain max, osc/master levels). Filter keyboard
tracking is forced off so the cutoff knob alone defines the filter.

## Capture source

Oscillator 2 supplies saw, triangle, and 50% pulse. Oscillator 1 stays off.
`reset()` restores the full neutral table; waveform changes always send all three
oscillator-2 switches.
