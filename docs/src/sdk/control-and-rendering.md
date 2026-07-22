# SDK: Control and Rendering

This page defines the host contract. Keep ownership of `SynthEngine` and all
calls that mutate or render it on one audio-processing context. Other threads
should pass control events to that context through the host's queue or lock-free
transport. `synth-app` demonstrates that pattern with `rtrb`.

## Control messages

`ControlMessage` is the complete event protocol:

| Message | Host responsibility |
| --- | --- |
| `SetParam(ParamId, f32)` | Update one parameter. Boolean controls use `0.0`/`1.0`; enum-like controls use their documented index. |
| `SetModulation { ... }` | Configure a free or dedicated modulation route. |
| `SetModulationParam { ... }` | Update one source, destination, or amount field of a modulation route. This supports incremental control protocols such as NRPN. |
| `SetFilterOversampling` | Change nonlinear-filter oversampling without rebuilding the stream. |
| `NoteOn` / `NoteOff` / `AllNotesOff` | Send MIDI-style note lifecycle events. Velocity zero is treated as note-off. |
| `PitchBend`, `ModWheel`, `Pressure` | Send normalized performance-source values. |
| `SustainPedal` | Hold released notes while pressed. |
| `ControlChange` | Forward a normalized MIDI controller value. |
| `SetMidiClockMode` | Select the device-global Rev2 clock policy. The core receives Slave/Slave No S/S clock; a host such as `synth-app` may implement Master output. Slave Thru remains inactive. |
| `MidiRealtime` | Deliver timestamped Timing Clock, Start, or Stop input. |

Convenience methods on `SynthEngine` exist for applying a complete `Patch`,
parameter updates, notes, all notes off, pitch bend, mod wheel, pressure,
sustain pedal, and generic control changes. They forward to the same message
handling path. The engine consumes only the generic patch representation and
has no dependency on MIDI or device-specific protocols.

`MidiClockStatus` reports configured and effective modes, live state, learned
and effective BPM, transport state, and pulse position. Hosts must timestamp
Timing Clock at receipt using a monotonic microsecond clock. The core derives
tempo at 24 PPQN and advances its clock-loss timer from rendered output frames,
so it remains `no_std` and independent of a platform timer.

## Parameter values

`ParamId` is intentionally a compact host address space rather than a UI
description. Hosts should present user-friendly labels, units, ranges, and
enums themselves. The `Patch` conversion helpers are the authoritative mapping
from stored typed values to message values.

The shared `midi::rev2` module translates Sequential Rev2 CC and NRPN messages
to this host address space, encodes parameter changes back to NRPN sequences,
maps global clock mode NRPN 4099, and converts Program Edit Buffer SysEx
messages to and from `Patch`. SysEx
packing and device-specific Layer A/Layer B policy remain at this boundary.
The unpacked 2,046-byte Rev2 program image is not indexed by NRPN number: the
codec uses the program-image offset table and reconstructs bipolar or extended
values whose high bit is stored in another parameter byte.
Reusing the codec in each host keeps desktop and embedded behavior consistent.
Stored Program Data decoding additionally returns its bank and program metadata
with the patch so hosts can build libraries without coupling that metadata to
the synthesis engine.

For example, cutoff is expressed in hertz, while `EffectMix` and other blend
amounts are normalized. Boolean fields are interpreted using a `0.5` threshold
where applicable. Indexed fields such as effect and waveform selection use the
corresponding enum index. Avoid inventing a second parameter state model in the
host; retain a `Patch` or a comparable host state and apply it through the
engine's message API.

## Audio callback example

```rust,ignore
fn render_callback(
    engine: &mut synth_core::SynthEngine,
    pending: impl Iterator<Item = synth_core::ControlMessage>,
    output: &mut [f32],
    channels: usize,
) {
    for message in pending {
        engine.handle_control(message);
    }
    engine.process_interleaved(output, channels);
}
```

The engine renders samples synchronously; it does not create threads, access
audio devices, allocate host buffers, or poll MIDI. The host controls block
size, device format, scheduling, and the boundary between UI/MIDI and audio.

`active_notes` and `active_voice_count` expose lightweight state suitable for
meters or diagnostic views. They should be read from the same ownership context
as the engine unless the host provides its own safe snapshot mechanism.
