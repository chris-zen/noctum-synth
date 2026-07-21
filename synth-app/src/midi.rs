use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use synth_core::{
    MidiProgramImport, MidiRealtimeEvent, ModDestination, ModRoute, ModSource, P08MidiDecoder,
    ParamId, Patch, REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN, Rev2MidiDecoder, Rev2MidiEncoder,
    Rev2MidiUpdate,
};
use wmidi::MidiMessage;

use crate::engine::SynthEngineControl;
use crate::ui::settings_view::MidiInputEntry;

const RECONNECT_INTERVAL: Duration = Duration::from_millis(500);
const MIDI_ECHO_TTL: Duration = Duration::from_secs(1);
const MIDI_ECHO_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortConnectionState {
    Connected,
    Unavailable,
    Failed,
}

pub struct MidiInputFlags {
    control: AtomicBool,
    patches: AtomicBool,
    forward: AtomicBool,
    clock: AtomicBool,
}

impl MidiInputFlags {
    fn from_entry(entry: &MidiInputEntry, clock: bool) -> Arc<Self> {
        Arc::new(Self {
            control: AtomicBool::new(entry.control),
            patches: AtomicBool::new(entry.patches),
            forward: AtomicBool::new(entry.forward),
            clock: AtomicBool::new(clock),
        })
    }

    fn sync(&self, entry: &MidiInputEntry, clock: bool) {
        self.control.store(entry.control, Ordering::Relaxed);
        self.patches.store(entry.patches, Ordering::Relaxed);
        self.forward.store(entry.forward, Ordering::Relaxed);
        self.clock.store(clock, Ordering::Relaxed);
    }
}

struct ManagedInput {
    #[allow(dead_code)]
    connection: MidiInputConnection<()>,
    flags: Arc<MidiInputFlags>,
}

pub struct MidiInputManager {
    control: SynthEngineControl,
    output: MidiOutputHandle,
    connections: HashMap<String, ManagedInput>,
    pending: HashMap<String, Arc<MidiInputFlags>>,
    available_ports: Vec<String>,
    last_tick: Instant,
}

impl MidiInputManager {
    pub fn new(control: SynthEngineControl, output: MidiOutputHandle) -> Self {
        let available_ports = list_input_ports();
        Self {
            control,
            output,
            connections: HashMap::new(),
            pending: HashMap::new(),
            available_ports,
            last_tick: Instant::now(),
        }
    }

    pub fn sync(&mut self, entries: &[MidiInputEntry], clock_source: Option<&str>) {
        let configured: HashSet<&str> = entries.iter().map(|entry| entry.port.as_str()).collect();

        // Clear the old source first so changing ports never creates a window
        // where two callback threads both feed the clock follower.
        for managed in self.connections.values() {
            managed.flags.clock.store(false, Ordering::Relaxed);
        }
        for flags in self.pending.values() {
            flags.clock.store(false, Ordering::Relaxed);
        }

        self.connections.retain(|port, managed| {
            if configured.contains(port.as_str()) {
                true
            } else {
                self.pending.remove(port);
                let _ = managed;
                false
            }
        });

        for entry in entries {
            let is_clock_source = clock_source == Some(entry.port.as_str());
            if let Some(managed) = self.connections.get(&entry.port) {
                managed.flags.sync(entry, is_clock_source);
                self.pending
                    .insert(entry.port.clone(), managed.flags.clone());
                continue;
            }

            let flags = self
                .pending
                .get(&entry.port)
                .cloned()
                .unwrap_or_else(|| MidiInputFlags::from_entry(entry, is_clock_source));
            flags.sync(entry, is_clock_source);
            self.pending.insert(entry.port.clone(), flags.clone());

            if let Some(connection) = connect_input_port(
                &entry.port,
                self.control.clone(),
                self.output.clone(),
                flags.clone(),
            ) {
                eprintln!("MIDI connected: {}", entry.port);
                self.connections
                    .insert(entry.port.clone(), ManagedInput { connection, flags });
            }
        }
    }

    pub fn tick(&mut self) {
        if self.last_tick.elapsed() < RECONNECT_INTERVAL {
            return;
        }
        self.last_tick = Instant::now();
        self.available_ports = list_input_ports();

        self.connections
            .retain(|port, _| port_is_available(port, &self.available_ports));

        let entries: Vec<(String, Arc<MidiInputFlags>)> = self
            .pending
            .iter()
            .map(|(port, flags)| (port.clone(), flags.clone()))
            .collect();
        for (port, flags) in entries {
            if self.connections.contains_key(&port) {
                continue;
            }
            if !port_is_available(&port, &self.available_ports) {
                continue;
            }
            if let Some(connection) = connect_input_port(
                &port,
                self.control.clone(),
                self.output.clone(),
                flags.clone(),
            ) {
                eprintln!("MIDI reconnected: {port}");
                self.connections
                    .insert(port, ManagedInput { connection, flags });
            }
        }
    }

    pub fn connection_state(&self, port: &str) -> PortConnectionState {
        if !port_is_available(port, &self.available_ports) {
            return PortConnectionState::Unavailable;
        }
        if self.connections.contains_key(port) {
            PortConnectionState::Connected
        } else if self.pending.contains_key(port) {
            PortConnectionState::Failed
        } else {
            PortConnectionState::Unavailable
        }
    }

    pub fn refresh_available_ports(&mut self) {
        self.available_ports = list_input_ports();
    }
}

fn connect_input_port(
    port_name: &str,
    control: SynthEngineControl,
    output: MidiOutputHandle,
    flags: Arc<MidiInputFlags>,
) -> Option<MidiInputConnection<()>> {
    let mut midi_in = MidiInput::new("analog-synth").ok()?;
    midi_in.ignore(Ignore::None);
    let ports = midi_in.ports();
    let filter_lower = port_name.to_lowercase();
    let port = ports.iter().find(|port| {
        midi_in
            .port_name(port)
            .map(|name| name.to_lowercase().contains(&filter_lower))
            .unwrap_or(false)
    })?;
    let mut decoder = Rev2MidiDecoder::default();
    let input_port = port_name.to_owned();
    midi_in
        .connect(
            &port,
            &format!("midi-in-{port_name}"),
            move |timestamp, message, _| {
                handle_midi_with_flags(
                    timestamp,
                    message,
                    &input_port,
                    &control,
                    &mut decoder,
                    &output,
                    &flags,
                );
            },
            (),
        )
        .ok()
}

fn port_is_available(port: &str, available_ports: &[String]) -> bool {
    let port_lower = port.to_lowercase();
    available_ports.iter().any(|name| {
        name.to_lowercase().contains(&port_lower) || port_lower.contains(&name.to_lowercase())
    })
}

pub fn merged_port_list(available: &[String], configured: &[String]) -> Vec<String> {
    let mut merged = configured.to_vec();
    for port in available {
        if !merged.iter().any(|existing| existing == port) {
            merged.push(port.clone());
        }
    }
    merged
}

fn handle_midi_with_flags(
    timestamp_micros: u64,
    message: &[u8],
    input_port: &str,
    control: &SynthEngineControl,
    decoder: &mut Rev2MidiDecoder,
    output: &MidiOutputHandle,
    flags: &MidiInputFlags,
) {
    let event = match message {
        [0xf8] => Some(MidiRealtimeEvent::TimingClock { timestamp_micros }),
        [0xfa] => Some(MidiRealtimeEvent::Start),
        [0xfc] => Some(MidiRealtimeEvent::Stop),
        _ => None,
    };
    if let Some(event) = event {
        if !flags.clock.load(Ordering::Relaxed) {
            return;
        }
        control.midi_realtime(event);
    }

    if output.consume_echo_from(input_port, message) {
        return;
    }

    if flags.forward.load(Ordering::Relaxed) {
        output.send_raw(message);
    }

    if message.first() == Some(&0xf0) {
        if flags.patches.load(Ordering::Relaxed) {
            handle_midi_sysex(message, control, decoder);
        }
        return;
    }

    if flags.control.load(Ordering::Relaxed) {
        handle_midi_control(message, control, decoder);
    }
}

fn handle_midi_sysex(message: &[u8], control: &SynthEngineControl, _decoder: &mut Rev2MidiDecoder) {
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
        handle_midi_sysex_message(&remaining[..=end], control);
        remaining = &remaining[end + 1..];
    }
}

fn handle_midi_sysex_message(message: &[u8], control: &SynthEngineControl) {
    let Some(model) = message.get(2) else {
        eprintln!("Unsupported SysEx message");
        return;
    };
    let Some(command) = message.get(3) else {
        eprintln!("Unsupported SysEx message");
        return;
    };
    match (*model, *command) {
        (0x2f, 0x02) => match Rev2MidiDecoder::program_data(message) {
            Ok(program) => {
                if !control.queue_midi_program(MidiProgramImport::Rev2(program)) {
                    eprintln!("MIDI program import queue is full");
                }
            }
            Err(err) => eprintln!(
                "Invalid Rev2 Program Data message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x2f, 0x03) => match Rev2MidiDecoder::program_edit_buffer(message) {
            Ok(patch) => control.load_midi_patch(&patch),
            Err(err) => eprintln!(
                "Invalid Rev2 Program Edit Buffer message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x23, 0x02) => match P08MidiDecoder::program_data(message) {
            Ok(program) => {
                if !control.queue_midi_program(MidiProgramImport::P08(program)) {
                    eprintln!("MIDI program import queue is full");
                }
            }
            Err(err) => eprintln!(
                "Invalid Prophet '08 Program Data message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x23, 0x03) => match P08MidiDecoder::program_edit_buffer(message) {
            Ok(patch) => control.load_midi_patch(&patch),
            Err(err) => eprintln!(
                "Invalid Prophet '08 Program Edit Buffer message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        _ => eprintln!("Unsupported SysEx message"),
    }
}

fn handle_midi_control(
    message: &[u8],
    control: &SynthEngineControl,
    decoder: &mut Rev2MidiDecoder,
) {
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

#[cfg(test)]
fn handle_midi(message: &[u8], control: &SynthEngineControl, decoder: &mut Rev2MidiDecoder) {
    if message.first() == Some(&0xf0) {
        handle_midi_sysex(message, control, decoder);
        return;
    }
    handle_midi_control(message, control, decoder);
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
    configured_port: Option<String>,
    available_ports: Vec<String>,
    last_tick: Instant,
    recent_messages: VecDeque<(Instant, Vec<u8>)>,
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
                configured_port: None,
                available_ports: Vec::new(),
                last_tick: Instant::now(),
                recent_messages: VecDeque::new(),
            })),
        }
    }
}

impl MidiOutputHandle {
    pub fn connect(&self, port_name: Option<&str>) -> bool {
        let mut state = self.state.lock();
        state.configured_port = port_name.map(str::to_owned);
        Self::connect_locked(&mut state)
    }

    fn connect_locked(state: &mut MidiOutputState) -> bool {
        let Some(filter) = state.configured_port.as_deref() else {
            clear_midi_output(state);
            return true;
        };
        let Ok(midi_out) = MidiOutput::new("analog-synth-output") else {
            clear_midi_output(state);
            return false;
        };
        let ports = midi_out.ports();
        let filter_lower = filter.to_lowercase();
        let Some(port) = ports.iter().find(|port| {
            midi_out
                .port_name(port)
                .map(|name| name.to_lowercase().contains(&filter_lower))
                .unwrap_or(false)
        }) else {
            clear_midi_output(state);
            return false;
        };
        let Ok(connection) = midi_out.connect(&port, "analog-synth-midi-output") else {
            clear_midi_output(state);
            return false;
        };
        state.connection = Some(connection);
        state.last_nrpn_values.fill(None);
        true
    }

    pub fn is_connected(&self) -> bool {
        self.state.lock().connection.is_some()
    }

    pub fn connection_state(&self) -> PortConnectionState {
        let state = self.state.lock();
        let Some(port) = state.configured_port.as_deref() else {
            return PortConnectionState::Unavailable;
        };
        if !port_is_available(port, &state.available_ports) {
            return PortConnectionState::Unavailable;
        }
        if state.connection.is_some() {
            PortConnectionState::Connected
        } else {
            PortConnectionState::Failed
        }
    }

    pub fn tick(&self) {
        let mut state = self.state.lock();
        if state.last_tick.elapsed() < RECONNECT_INTERVAL {
            return;
        }
        state.last_tick = Instant::now();
        state.available_ports = list_output_ports();
        if let Some(port) = state.configured_port.as_deref() {
            if !port_is_available(port, &state.available_ports) {
                clear_midi_output(&mut state);
                return;
            }
            if state.connection.is_none() {
                let _ = Self::connect_locked(&mut state);
            }
        }
    }

    pub fn refresh_available_ports(&self) {
        self.state.lock().available_ports = list_output_ports();
    }

    pub fn send_raw(&self, message: &[u8]) {
        let mut state = self.state.lock();
        let Some(connection) = state.connection.as_mut() else {
            return;
        };
        if connection.send(message).is_err() {
            clear_midi_output(&mut state);
        } else {
            record_midi_echo(&mut state.recent_messages, message);
        }
    }

    fn consume_echo_from(&self, input_port: &str, message: &[u8]) -> bool {
        let mut state = self.state.lock();
        let Some(output_port) = state.configured_port.as_deref() else {
            return false;
        };
        if !midi_port_names_match(input_port, output_port) {
            return false;
        }
        consume_midi_echo(&mut state.recent_messages, message)
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
            record_midi_echo(&mut state.recent_messages, &message[..length]);
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
    let mut sent = Vec::new();
    if !send_changed_nrpn_messages(last_nrpn_values, messages, |message| {
        if connection.send(message).is_ok() {
            sent.push(message.to_vec());
            true
        } else {
            false
        }
    }) {
        clear_midi_output(state);
    } else {
        for message in sent {
            record_midi_echo(&mut state.recent_messages, &message);
        }
    }
}

fn record_midi_echo(recent: &mut VecDeque<(Instant, Vec<u8>)>, message: &[u8]) {
    let now = Instant::now();
    recent.retain(|(sent_at, _)| now.duration_since(*sent_at) <= MIDI_ECHO_TTL);
    if recent.len() >= MIDI_ECHO_CAPACITY {
        recent.pop_front();
    }
    recent.push_back((now, message.to_vec()));
}

fn consume_midi_echo(recent: &mut VecDeque<(Instant, Vec<u8>)>, message: &[u8]) -> bool {
    let now = Instant::now();
    recent.retain(|(sent_at, _)| now.duration_since(*sent_at) <= MIDI_ECHO_TTL);
    let Some(index) = recent.iter().position(|(_, sent)| sent == message) else {
        return false;
    };
    recent.remove(index);
    true
}

fn clear_midi_output(state: &mut MidiOutputState) {
    state.connection = None;
    state.last_nrpn_values.fill(None);
    state.recent_messages.clear();
}

fn midi_port_names_match(left: &str, right: &str) -> bool {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
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
        let value = u16::from(sequence[2][2]) * 128 + usize::from(sequence[3][2]) as u16;
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
    let Ok(midi_in) = MidiInput::new("analog-synth-list") else {
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

    fn all_flags() -> Arc<MidiInputFlags> {
        Arc::new(MidiInputFlags {
            control: AtomicBool::new(true),
            patches: AtomicBool::new(true),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(false),
        })
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
    fn midi_echo_cache_consumes_only_matching_recent_output() {
        let mut recent = VecDeque::new();
        record_midi_echo(&mut recent, &[0xf0, 0x01, 0xf7]);

        assert!(!consume_midi_echo(&mut recent, &[0x90, 60, 100]));
        assert!(consume_midi_echo(&mut recent, &[0xf0, 0x01, 0xf7]));
        assert!(!consume_midi_echo(&mut recent, &[0xf0, 0x01, 0xf7]));
    }

    #[test]
    fn midi_echo_cache_does_not_consume_expired_output() {
        let mut recent = VecDeque::from([(
            Instant::now() - MIDI_ECHO_TTL - Duration::from_millis(1),
            vec![0xb0, 14, 90],
        )]);

        assert!(!consume_midi_echo(&mut recent, &[0xb0, 14, 90]));
        assert!(recent.is_empty());
    }

    #[test]
    fn output_echo_cache_does_not_swallow_note_off_from_another_input_port() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let output = MidiOutputHandle::default();
        {
            let mut state = output.state.lock();
            state.configured_port = Some("Analog Synth USB MIDI (development)".to_owned());
            // These model identical NoteOff messages forwarded during the
            // immediately preceding chord, still inside the echo TTL.
            record_midi_echo(&mut state.recent_messages, &[0x80, 64, 0]);
            record_midi_echo(&mut state.recent_messages, &[0x80, 62, 0]);
        }
        let mut decoder = Rev2MidiDecoder::default();

        for message in [[0x80, 64, 0], [0x80, 62, 0]] {
            handle_midi_with_flags(
                0,
                &message,
                "Arturia MiniLab mkII",
                &bridge.control,
                &mut decoder,
                &output,
                &all_flags(),
            );
        }

        assert!(matches!(
            audio.control.0.pop(),
            Ok(ControlMessage::NoteOff { note: 64 })
        ));
        assert!(matches!(
            audio.control.0.pop(),
            Ok(ControlMessage::NoteOff { note: 62 })
        ));
        assert_eq!(output.state.lock().recent_messages.len(), 2);
    }

    #[test]
    fn echoed_patch_is_not_applied_to_ui() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let output = MidiOutputHandle::default();
        let mut message = [0_u8; REV2_PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        Rev2MidiEncoder::program_edit_buffer(&Patch::default(), &mut message).unwrap();
        {
            let mut state = output.state.lock();
            state.configured_port = Some("loopback".to_owned());
            record_midi_echo(&mut state.recent_messages, &message);
        }
        let mut decoder = Rev2MidiDecoder::default();

        handle_midi_with_flags(
            0,
            &message,
            "loopback",
            &bridge.control,
            &mut decoder,
            &output,
            &all_flags(),
        );

        let mut updates = 0;
        bridge.view.drain_midi_ui_updates(|_| updates += 1);
        assert_eq!(updates, 0);
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
        assert_eq!((imported.bank(), imported.program()), (4, 0));
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
        bridge.view.drain_midi_program_imports(|program| {
            locations.push((program.bank(), program.program()))
        });
        assert_eq!(locations, [(4, 0), (4, 1)]);
    }

    #[test]
    fn control_flag_gates_note_messages() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = Rev2MidiDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(false),
            patches: AtomicBool::new(true),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(false),
        });
        handle_midi_with_flags(
            0,
            &[0x90, 60, 100],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &flags,
        );
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn selected_clock_source_routes_realtime_with_midir_timestamp() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = Rev2MidiDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(false),
            patches: AtomicBool::new(false),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(true),
        });
        handle_midi_with_flags(
            123_456,
            &[0xf8],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &flags,
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(ControlMessage::MidiRealtime(
                MidiRealtimeEvent::TimingClock {
                    timestamp_micros: 123_456
                }
            ))
        ));

        handle_midi_with_flags(
            0,
            &[0xfb],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &flags,
        );
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn patches_flag_gates_sysex_messages() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = Rev2MidiDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(true),
            patches: AtomicBool::new(false),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(false),
        });
        let message = stored_program_message(4, 0);
        handle_midi_with_flags(
            0,
            &message,
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &flags,
        );

        let mut imported = None;
        bridge
            .view
            .drain_midi_program_imports(|program| imported = Some(program));
        assert!(imported.is_none());
    }

    #[test]
    fn merged_port_list_includes_missing_configured_ports() {
        let available = vec!["Port A".to_string()];
        let configured = vec!["Port B".to_string()];
        let merged = merged_port_list(&available, &configured);
        assert_eq!(merged, vec!["Port B".to_string(), "Port A".to_string()]);
    }

    #[test]
    fn all_flags_process_control_and_sysex() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = Rev2MidiDecoder::default();
        let output = MidiOutputHandle::default();
        handle_midi_with_flags(
            0,
            &[0x90, 60, 100],
            "test input",
            &bridge.control,
            &mut decoder,
            &output,
            &all_flags(),
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(ControlMessage::NoteOn { note: 60, .. })
        ));
    }
}
