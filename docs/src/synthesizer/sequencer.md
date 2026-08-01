# Sequencers

Each layer owns two Prophet Rev2-compatible sequencers. The **Sequencer Type**
switch selects which one is active; sequence contents for both types remain in
the patch when the switch changes.

## Polyphonic sequencer

The polyphonic sequencer has 64 steps and six note/velocity lanes per step.
Live notes play over a running sequence without changing its pitch. To
transpose the sequence, enable **Record** while it is playing and press a key;
middle C is the reference pitch. Starting playback again restores the recorded
pitch. Live-play notes remain separately owned, so stopping or clearing the
sequence cannot release a live note at the same pitch.

- Ordinary notes gate for half a step.
- A tie holds the preceding lane note through the entire tie step; the following
  note, rest, reset, or tie determines what happens next. A leading tie is silent.
- A rest silences its lane.
- A step with Reset/End in all six lanes loops playback to step 1. A Reset in
  only some lanes leaves the other notes on that step playable.
- In Normal mode, UI Play and Stop address the edited layer. In Stack and Split
  modes they address both audible layers so dual-layer factory sequences play as
  a complete preset. MIDI transport remains layer-addressed. Stop releases only
  sequencer-owned voices and retains the playback position; Continue resumes
  from that position, while Start restarts at step 1.
- The arpeggiator is suspended while poly playback is active and resumes without
  orphaning notes when playback stops.
- MIDI Start/Stop controls poly playback in Slave mode. Slave No S/S ignores
  transport while still following Timing Clock.

### Step recording

Press **Record**, then play a chord. A chord is committed when the last physical
key is released. Sustain does not delay this commit. Up to six unique notes are
stored with their original velocities; further held notes are ignored and the
editor shows an overflow warning. Repeated Note On for an already-held pitch
does not create a duplicate lane.

Stopping recording or replacing the program cancels a partially held chord.
The Rest, Tie, End/Reset, and Clear Cursor Step commands edit the current cursor;
committing/inserting advances with wrap from step 64 to step 1. Clear Cursor Step
writes a rest without advancing.

## Gated sequencer

The gated sequencer has four independent 16-step modulation tracks. Values
0–125 are normalized modulation values, 126 resets that track to step 1, and 127
is a rest. Track 1 controls the shared envelope gate. Tracks 2 and 4 can select
**Slew previous track**, applying the Prophet-style modulation-time curve to
tracks 1 and 3 respectively.

| Mode | Key reset | Envelope gate | Advance |
|---|---|---|---|
| Normal | Yes | Half step | Clock while a key is held |
| No Reset | No | Half step | Clock while a key is held |
| No Gate | Yes | Continuous held envelope | Clock while a key is held |
| No Gate / No Reset | No | Continuous held envelope | Clock while a key is held |
| Key Step | Yes | Per note | Each Note On |

The four track outputs are also available as modulation sources Seq1–Seq4.
Direct oscillator-frequency destinations use exactly half a semitone per raw
step. MIDI Start/Stop never controls the gated sequencer; external Timing Clock
still advances it.

## Timing

The arpeggiator and both sequencers share the same allocation-free step-clock
implementation. It supports the patch BPM, all Clock Divide settings, external
24-PPQN MIDI clock, phase-preserving tempo changes, and straight/half/full swing.
Internal clocks continue across patch-content replacement. Starting playback
with Start resets the poly playback position; changing a running program swaps
sequence content without resetting transport phase.

## Desktop editor

Open the **Sequencer** tab. Layer A/B selection follows the main edit layer. The
gated grid scrolls horizontally at narrow window widths. The compact poly grid
mounts all 64 positions at once and scrolls horizontally without pagination.
Its dedicated lane-number column spans the normal-weight Note and Velocity rows.
The selected edit position header is written as `[ 02 ]`. During playback, the
active header uses the theme selection color and the grid scrolls to keep that
position visible.

The Parameters and Sequencer views render the same full-width **Layers** and
**Sequencer** bars. The Layers bar is the only layer selector. The Sequencer bar
contains a segmented Gated/Polyphonic Type selector and a one-based
`Position [<] [value] [>]` edit/record cursor. Polyphonic type shows equal-width
Play/Stop and Record controls; Gated type replaces them with the Mode selector.
Position state is independent for each layer and sequencer type.

The editor presents a semantic Event cell over the lossless Rev2 note/velocity
pair. Event accepts a pitched note, Tie (`=`), Rest (`-`), or Reset (`<`), while
Velocity is editable only for pitched notes. Choosing a note or Tie from a
Rest/Reset cell supplies velocity 127 automatically; an existing numeric
velocity is preserved. Inactive raw note values under Rest/Reset remain intact,
so loading and saving an untouched program is lossless. All visible labels and
markers use ASCII characters supported by the egui font. Event accepts MIDI
note numbers or ASCII note names such as `C4`, `C#4`, and `Db4`; Velocity accepts
values `1`-`127` without a `V` prefix.

Bulk clears require a second confirmation. They enter the bounded audio queue as
one command; MIDI output resynchronizes the complete edit buffer. All ordinary
cell edits use typed sequence updates, participate in native patch dirty/save
state, and work without a Daisy connected.

## Current exclusions

Sequencer pedal/audio-input triggering and transmission of generated sequencer
notes as MIDI output are not implemented. Swing ratios and slew timing use the
documented project defaults pending optional side-by-side hardware calibration.
