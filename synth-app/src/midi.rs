use midir::{Ignore, MidiInput, MidiInputConnection};
use wmidi::MidiMessage;

use crate::engine::SynthEngineControl;

pub fn start_midi(
    port_name: Option<&str>,
    control: SynthEngineControl,
) -> Option<MidiInputConnection<()>> {
    let Some(filter) = port_name else {
        eprintln!("No MIDI port selected; MIDI input disabled.");
        return None;
    };

    let mut midi_in = MidiInput::new("analog-synth").ok()?;
    midi_in.ignore(Ignore::None);
    let ports = midi_in.ports();

    let filter_lower = filter.to_lowercase();
    let port = ports.iter().find(|port| {
        midi_in
            .port_name(port)
            .map(|name| name.to_lowercase().contains(&filter_lower))
            .unwrap_or(false)
    });
    let Some(port) = port else {
        eprintln!("No MIDI port matching \"{filter}\"; MIDI input disabled.");
        return None;
    };
    eprintln!(
        "MIDI connected: {}",
        midi_in.port_name(port).unwrap_or_default()
    );

    let conn = midi_in
        .connect(
            port,
            "midi-in",
            move |_timestamp, message, _| {
                handle_midi(message, &control);
            },
            (),
        )
        .ok()?;

    Some(conn)
}

fn handle_midi(message: &[u8], control: &SynthEngineControl) {
    let msg = match MidiMessage::try_from(message) {
        Ok(msg) => msg,
        Err(err) => {
            eprintln!("Invalid MIDI message: {err}");
            return;
        }
    };

    match msg {
        MidiMessage::NoteOn(_, note, velocity) => {
            let note = u8::from(note);
            let velocity = u8::from(velocity);
            if velocity > 0 {
                control.note_on(note, velocity as f32 / 127.0);
            } else {
                control.note_off(note);
            }
        }
        MidiMessage::NoteOff(_, note, _) => {
            control.note_off(u8::from(note));
        }
        MidiMessage::PitchBendChange(_, bend) => {
            let value = u16::from(bend) as f32 / 16383.0 * 2.0 - 1.0;
            control.pitch_bend(value);
        }
        MidiMessage::PolyphonicKeyPressure(_, _, pressure) => {
            control.pressure(u8::from(pressure) as f32 / 127.0);
        }
        MidiMessage::ChannelPressure(_, pressure) => {
            control.pressure(u8::from(pressure) as f32 / 127.0);
        }
        MidiMessage::ControlChange(_, controller, value) => {
            let controller = u8::from(controller);
            let value = u8::from(value);
            match controller {
                1 => control.mod_wheel(value as f32 / 127.0),
                64 => control.sustain_pedal(value >= 64),
                120 | 123 => control.all_notes_off(),
                _ => control.control_change(controller, value as f32 / 127.0),
            }
        }
        _ => {}
    }
}

pub fn list_ports() -> Vec<String> {
    let midi_in = match MidiInput::new("analog-synth") {
        Ok(input) => input,
        Err(_) => return Vec::new(),
    };
    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}
