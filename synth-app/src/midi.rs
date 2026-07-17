use std::sync::Arc;

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use synth_core::{
    ModDestination, ModRoute, ModSource, ParamId, Patch, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN,
    Rev2MidiDecoder, Rev2MidiEncoder, Rev2MidiUpdate,
};
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

    let mut decoder = Rev2MidiDecoder::default();
    midi_in
        .connect(
            port,
            "midi-in",
            move |_timestamp, message, _| {
                handle_midi(message, &control, &mut decoder);
            },
            (),
        )
        .ok()
}

fn handle_midi(message: &[u8], control: &SynthEngineControl, decoder: &mut Rev2MidiDecoder) {
    if message.first() == Some(&0xf0) {
        let mut remaining = message;
        while let Some(start) = remaining.iter().position(|byte| *byte == 0xf0) {
            remaining = &remaining[start..];
            let Some(end) = remaining.iter().position(|byte| *byte == 0xf7) else {
                eprintln!(
                    "Incomplete SysEx frame received ({} bytes)",
                    remaining.len()
                );
                return;
            };
            handle_midi_message(&remaining[..=end], control, decoder);
            remaining = &remaining[end + 1..];
        }
        return;
    }
    handle_midi_message(message, control, decoder);
}

fn handle_midi_message(
    message: &[u8],
    control: &SynthEngineControl,
    decoder: &mut Rev2MidiDecoder,
) {
    if message.first() == Some(&0xf0) {
        match message.get(3) {
            Some(0x02) => match Rev2MidiDecoder::program_data(message) {
                Ok(program) => {
                    if !control.queue_midi_program(program) {
                        eprintln!("Rev2 program import queue is full");
                    }
                }
                Err(err) => eprintln!(
                    "Invalid Rev2 Program Data message: {err:?} ({} bytes)",
                    message.len()
                ),
            },
            Some(0x03) => match Rev2MidiDecoder::program_edit_buffer(message) {
                Ok(patch) => control.load_midi_patch(&patch),
                Err(err) => eprintln!(
                    "Invalid Rev2 Program Edit Buffer message: {err:?} ({} bytes)",
                    message.len()
                ),
            },
            _ => eprintln!("Unsupported Rev2 SysEx message"),
        }
        return;
    }
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
        MidiMessage::NoteOff(_, note, _) => control.note_off(u8::from(note)),
        MidiMessage::PitchBendChange(_, bend) => {
            let value = u16::from(bend) as f32 / 16_383.0 * 2.0 - 1.0;
            control.pitch_bend(value);
        }
        MidiMessage::PolyphonicKeyPressure(_, _, pressure)
        | MidiMessage::ChannelPressure(_, pressure) => {
            control.pressure(u8::from(pressure) as f32 / 127.0);
        }
        MidiMessage::ControlChange(channel, controller, value) => {
            let controller = u8::from(controller);
            let value = u8::from(value);
            match controller {
                1 => control.mod_wheel(value as f32 / 127.0),
                64 => control.sustain_pedal(value >= 64),
                120 | 123 => control.all_notes_off(),
                _ if decoder.control_change(channel.index(), controller, value, |update| {
                    dispatch_inbound_update(control, update);
                }) => {}
                _ => control.control_change(controller, value as f32 / 127.0),
            }
        }
        _ => {}
    }
}

fn dispatch_inbound_update(control: &SynthEngineControl, update: Rev2MidiUpdate) {
    match update {
        Rev2MidiUpdate::Param(param, value) => control.set_midi_param(param, value),
        Rev2MidiUpdate::Modulation { route, parameter } => {
            control.set_midi_modulation_param(route, parameter);
        }
    }
}

struct MidiOutputState {
    connection: Option<MidiOutputConnection>,
    encoder: Rev2MidiEncoder,
    last_nrpn_values: [Option<u16>; REV2_NRPN_PARAMETER_COUNT],
}

const REV2_NRPN_PARAMETER_COUNT: usize = 159;

/// Cloneable MIDI output connection shared by every UI-facing control handle.
#[derive(Clone)]
pub struct MidiOutputHandle {
    state: Arc<Mutex<MidiOutputState>>,
}

impl Default for MidiOutputHandle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(MidiOutputState {
                connection: None,
                encoder: Rev2MidiEncoder::default(),
                last_nrpn_values: [None; REV2_NRPN_PARAMETER_COUNT],
            })),
        }
    }
}

impl MidiOutputHandle {
    pub fn connect(&self, port_name: Option<&str>) -> bool {
        let Some(filter) = port_name else {
            clear_midi_output(&mut self.state.lock());
            return true;
        };
        let Ok(midi_out) = MidiOutput::new("analog-synth-output") else {
            clear_midi_output(&mut self.state.lock());
            return false;
        };
        let filter_lower = filter.to_lowercase();
        let Some(port) = midi_out.ports().into_iter().find(|port| {
            midi_out
                .port_name(port)
                .map(|name| name.to_lowercase().contains(&filter_lower))
                .unwrap_or(false)
        }) else {
            clear_midi_output(&mut self.state.lock());
            return false;
        };
        let Ok(connection) = midi_out.connect(&port, "analog-synth-midi-output") else {
            clear_midi_output(&mut self.state.lock());
            return false;
        };
        let mut state = self.state.lock();
        state.connection = Some(connection);
        state.last_nrpn_values.fill(None);
        true
    }

    pub fn is_connected(&self) -> bool {
        self.state.lock().connection.is_some()
    }

    pub fn send_param(&self, param: ParamId, value: f32) {
        let mut state = self.state.lock();
        if state.connection.is_none() {
            return;
        }
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        if !state.encoder.param(0, param, value, |message| {
            messages[len] = message;
            len += 1;
        }) {
            return;
        }
        send_messages(&mut state, &messages[..len]);
    }

    pub fn send_modulation(
        &self,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        let mut state = self.state.lock();
        if state.connection.is_none() {
            return;
        }
        let mut messages = [[0_u8; 3]; 12];
        let mut len = 0;
        state
            .encoder
            .modulation(0, route, enabled, source, destination, amount, |message| {
                messages[len] = message;
                len += 1;
            });
        send_messages(&mut state, &messages[..len]);
    }

    pub fn send_patch(&self, patch: &Patch) -> bool {
        let mut state = self.state.lock();
        let Some(connection) = state.connection.as_mut() else {
            return false;
        };
        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        let Ok(length) = Rev2MidiEncoder::program_edit_buffer(patch, &mut message) else {
            return false;
        };
        if connection.send(&message[..length]).is_err() {
            clear_midi_output(&mut state);
            false
        } else {
            state.last_nrpn_values.fill(None);
            true
        }
    }
}

fn send_messages(state: &mut MidiOutputState, messages: &[[u8; 3]]) {
    let MidiOutputState {
        connection,
        last_nrpn_values,
        ..
    } = state;
    let Some(connection) = connection.as_mut() else {
        return;
    };
    if !send_changed_nrpn_messages(last_nrpn_values, messages, |message| {
        connection.send(message).is_ok()
    }) {
        clear_midi_output(state);
    }
}

fn clear_midi_output(state: &mut MidiOutputState) {
    state.connection = None;
    state.last_nrpn_values.fill(None);
}

/// Sends only NRPN sequences whose quantized value differs from the last value
/// successfully sent on this connection.
fn send_changed_nrpn_messages(
    last_values: &mut [Option<u16>; REV2_NRPN_PARAMETER_COUNT],
    messages: &[[u8; 3]],
    mut send: impl FnMut(&[u8; 3]) -> bool,
) -> bool {
    debug_assert_eq!(messages.len() % 4, 0);
    for sequence in messages.chunks_exact(4) {
        debug_assert_eq!(sequence[0][1], 99);
        debug_assert_eq!(sequence[1][1], 98);
        debug_assert_eq!(sequence[2][1], 6);
        debug_assert_eq!(sequence[3][1], 38);

        let number = usize::from(sequence[0][2]) * 128 + usize::from(sequence[1][2]);
        let value = u16::from(sequence[2][2]) * 128 + u16::from(sequence[3][2]);
        let Some(previous) = last_values.get_mut(number) else {
            debug_assert!(false, "Rev2 NRPN number is outside the cache");
            return false;
        };
        if *previous == Some(value) {
            continue;
        }
        if sequence.iter().any(|message| !send(message)) {
            return false;
        }
        *previous = Some(value);
    }
    true
}

pub fn list_input_ports() -> Vec<String> {
    let Ok(midi_in) = MidiInput::new("analog-synth") else {
        return Vec::new();
    };
    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}

pub fn list_output_ports() -> Vec<String> {
    let Ok(midi_out) = MidiOutput::new("analog-synth-output-list") else {
        return Vec::new();
    };
    midi_out
        .ports()
        .iter()
        .filter_map(|port| midi_out.port_name(port).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MidiUiUpdate, create_synth_engine_bridge};
    use synth_core::ControlMessage;

    fn stored_program_message(
        bank: u8,
        program: u8,
    ) -> [u8; synth_core::REV2_PROGRAM_DATA_SYSEX_LEN] {
        let mut edit = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&Patch::default(), &mut edit).unwrap();
        let mut message = [0_u8; synth_core::REV2_PROGRAM_DATA_SYSEX_LEN];
        message[..6].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02, bank, program]);
        let payload_end = message.len() - 1;
        message[6..payload_end].copy_from_slice(&edit[4..edit.len() - 1]);
        message[payload_end] = 0xf7;
        message
    }

    #[test]
    fn inbound_nrpn_fans_out_to_engine_and_ui_without_output_path() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = Rev2MidiDecoder::default();
        for (controller, value) in [(99, 0), (98, 20), (6, 1), (38, 126)] {
            assert!(decoder.control_change(0, controller, value, |update| {
                dispatch_inbound_update(&bridge.control, update);
            }));
        }
        assert!(matches!(
            audio.control.0.pop(),
            Ok(ControlMessage::SetParam(ParamId::FilterEnvAmount, 1.0))
        ));
        let mut ui_update = None;
        bridge
            .view
            .drain_midi_ui_updates(|update| ui_update = Some(update));
        assert_eq!(
            ui_update,
            Some(MidiUiUpdate::Param(ParamId::FilterEnvAmount, 1.0))
        );
    }

    #[test]
    fn output_handle_starts_disconnected() {
        let output = MidiOutputHandle::default();
        assert!(!output.is_connected());
        assert!(!output.send_patch(&Patch::default()));
    }

    #[test]
    fn midi_output_suppresses_repeated_quantized_nrpn_values() {
        let mut cache = [None; REV2_NRPN_PARAMETER_COUNT];
        let messages = [[0xb0, 99, 0], [0xb0, 98, 33], [0xb0, 6, 0], [0xb0, 38, 13]];
        let mut sent = 0;

        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        assert_eq!(sent, 4);
    }

    #[test]
    fn midi_output_resends_after_cache_reset() {
        let mut cache = [None; REV2_NRPN_PARAMETER_COUNT];
        let messages = [[0xb0, 99, 0], [0xb0, 98, 33], [0xb0, 6, 0], [0xb0, 38, 13]];
        let mut sent = 0;

        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        cache.fill(None);
        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        assert_eq!(sent, 8);
    }

    #[test]
    fn inbound_edit_buffer_updates_engine_and_ui_path() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let mut patch = Patch::default();
        patch.filter.resonance = 1.0;
        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&patch, &mut message).unwrap();
        let mut decoder = Rev2MidiDecoder::default();
        handle_midi(&message, &bridge.control, &mut decoder);

        let mut found = false;
        bridge.view.drain_midi_ui_updates(|update| {
            if update == MidiUiUpdate::Param(ParamId::FilterResonance, 1.0) {
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn inbound_stored_program_is_queued_without_updating_ui() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let message = stored_program_message(4, 0);
        let mut decoder = Rev2MidiDecoder::default();
        handle_midi(&message, &bridge.control, &mut decoder);

        let mut imported = None;
        bridge
            .view
            .drain_midi_program_imports(|program| imported = Some(program));
        let imported = imported.unwrap();
        assert_eq!((imported.bank, imported.program), (4, 0));
        let mut ui_updates = 0;
        bridge.view.drain_midi_ui_updates(|_| ui_updates += 1);
        assert_eq!(ui_updates, 0);
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn splits_batched_program_sysex_frames() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let first = stored_program_message(4, 0);
        let second = stored_program_message(4, 1);
        let mut batch = Vec::with_capacity(first.len() + second.len());
        batch.extend_from_slice(&first);
        batch.extend_from_slice(&second);
        let mut decoder = Rev2MidiDecoder::default();
        handle_midi(&batch, &bridge.control, &mut decoder);

        let mut locations = Vec::new();
        bridge
            .view
            .drain_midi_program_imports(|program| locations.push((program.bank, program.program)));
        assert_eq!(locations, [(4, 0), (4, 1)]);
    }
}
