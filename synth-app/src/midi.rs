use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use wmidi::MidiMessage;

use synth_core::midi::clock::{MidiClockMode, MidiRealtimeEvent};
use synth_core::midi::program::ProgramData;
use synth_core::midi::{p08, rev2};
use synth_core::{
    LayerId, LayerMode, LayerTarget, ModDestination, ModRoute, ModSource, ModulationParam, ParamId,
    Patch, SequenceClear, SequenceUpdate,
};

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

pub fn merged_port_list(available: &[String], configured: &[String]) -> Vec<String> {
    let mut merged = configured.to_vec();
    for port in available {
        if !merged.iter().any(|existing| existing == port) {
            merged.push(port.clone());
        }
    }
    merged
}

pub fn list_input_ports() -> Vec<String> {
    let Ok(midi_in) = MidiInput::new("noctum-list") else {
        return Vec::new();
    };
    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}

pub fn list_output_ports() -> Vec<String> {
    let Ok(midi_out) = MidiOutput::new("noctum-output-list") else {
        return Vec::new();
    };
    midi_out
        .ports()
        .iter()
        .filter_map(|port| midi_out.port_name(port).ok())
        .collect()
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

    fn handle_message(
        &self,
        timestamp_micros: u64,
        message: &[u8],
        input_port: &str,
        control: &SynthEngineControl,
        decoder: &mut rev2::ControllerDecoder,
        output: &MidiOutputHandle,
        clock_mode: &SharedMidiClockMode,
    ) {
        if output.consume_echo_from(input_port, message) {
            return;
        }

        let is_clock_realtime = matches!(message, [0xf8] | [0xfa] | [0xfb] | [0xfc]);
        let event = match message {
            [0xf8] => Some(MidiRealtimeEvent::TimingClock { timestamp_micros }),
            [0xfa] => Some(MidiRealtimeEvent::Start),
            [0xfb] => Some(MidiRealtimeEvent::Continue),
            [0xfc] => Some(MidiRealtimeEvent::Stop),
            _ => None,
        };
        if is_clock_realtime {
            if !self.clock.load(Ordering::Relaxed) {
                return;
            }
            let mode = clock_mode.get();
            if self.forward.load(Ordering::Relaxed) && mode != MidiClockMode::Master {
                output.send_raw(message);
            }
            if mode.receives_clock() {
                if let Some(event) = event {
                    control.midi_realtime(event);
                }
            }
            return;
        }

        if self.forward.load(Ordering::Relaxed) {
            output.send_raw(message);
        }

        if message.first() == Some(&0xf0) {
            if self.patches.load(Ordering::Relaxed) {
                handle_midi_sysex(message, control, decoder);
            }
            return;
        }

        if self.control.load(Ordering::Relaxed) {
            handle_midi_control(message, control, decoder);
        }
    }
}

#[derive(Clone)]
struct SharedMidiClockMode(Arc<AtomicU8>);

impl SharedMidiClockMode {
    fn new(mode: MidiClockMode) -> Self {
        Self(Arc::new(AtomicU8::new(mode.index() as u8)))
    }

    fn set(&self, mode: MidiClockMode) {
        self.0.store(mode.index() as u8, Ordering::Relaxed);
    }

    fn get(&self) -> MidiClockMode {
        MidiClockMode::from_index(self.0.load(Ordering::Relaxed) as usize)
    }
}

struct ManagedInput {
    #[allow(dead_code)]
    connection: MidiInputConnection<()>,
    flags: Arc<MidiInputFlags>,
}

struct MidiOutputState {
    connection: Option<MidiOutputConnection>,
    encoder: rev2::ControllerEncoder,
    last_nrpn_values: HashMap<u16, u16>,
    configured_port: Option<String>,
    available_ports: Vec<String>,
    last_tick: Instant,
    recent_messages: VecDeque<(Instant, Vec<u8>)>,
    clock_mode: MidiClockMode,
    output_clock_mode: MidiClockMode,
    master_bpm: f32,
    next_clock_deadline: Option<Instant>,
    desired_patch: Option<Patch>,
    selected_layer: LayerId,
}

impl MidiOutputState {
    fn clear(&mut self) {
        self.connection = None;
        self.last_nrpn_values.clear();
        self.recent_messages.clear();
    }

    fn record_echo(&mut self, message: &[u8]) {
        let now = Instant::now();
        self.recent_messages
            .retain(|(sent_at, _)| now.duration_since(*sent_at) <= MIDI_ECHO_TTL);
        if self.recent_messages.len() >= MIDI_ECHO_CAPACITY {
            self.recent_messages.pop_front();
        }
        self.recent_messages.push_back((now, message.to_vec()));
    }

    fn consume_echo(&mut self, message: &[u8]) -> bool {
        let now = Instant::now();
        self.recent_messages
            .retain(|(sent_at, _)| now.duration_since(*sent_at) <= MIDI_ECHO_TTL);
        let Some(index) = self
            .recent_messages
            .iter()
            .position(|(_, sent)| sent == message)
        else {
            return false;
        };
        self.recent_messages.remove(index);
        true
    }

    fn resend_desired_patch(&mut self) -> bool {
        if self.connection.is_none() {
            return false;
        }
        if let Some(patch) = self.desired_patch.as_ref() {
            let mut message = [0_u8; rev2::PROGRAM_EDIT_BUFFER_SYSEX_LEN];
            let Ok(length) = rev2::encode::program_edit_buffer(patch, &mut message) else {
                return false;
            };
            if self
                .connection
                .as_mut()
                .is_none_or(|connection| connection.send(&message[..length]).is_err())
            {
                self.clear();
                return false;
            }
            self.last_nrpn_values.clear();
            self.record_echo(&message[..length]);
        }
        self.send_selected_layer();
        self.connection.is_some()
    }

    fn send_messages(&mut self, messages: &[[u8; 3]]) {
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        let mut sent = Vec::new();
        if !send_changed_nrpn_messages(&mut self.last_nrpn_values, messages, |message| {
            if connection.send(message).is_ok() {
                sent.push(message.to_vec());
                true
            } else {
                false
            }
        }) {
            self.clear();
        } else {
            for message in sent {
                self.record_echo(&message);
            }
        }
    }

    fn send_controller_messages(&mut self, messages: &[[u8; 3]]) {
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        let mut sent = Vec::new();
        for message in messages {
            if connection.send(message).is_err() {
                self.clear();
                return;
            }
            sent.push(message.to_vec());
        }
        for message in sent {
            self.record_echo(&message);
        }
    }

    fn send_selected_layer(&mut self) {
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        self.encoder.edit_layer(0, self.selected_layer, |message| {
            messages[len] = message;
            len += 1;
        });
        self.send_messages(&messages[..len]);
    }

    fn sync_encoder_from_patch(&mut self, patch: &Patch) {
        for layer in [LayerId::A, LayerId::B] {
            patch.layer(layer).for_each_param(|param, value| {
                let _ = self.encoder.param_for_layer(0, layer, param, value, |_| {});
            });
        }
    }

    fn send_output_clock_mode(&mut self) -> bool {
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        self.encoder
            .midi_clock_mode(0, self.output_clock_mode, |message| {
                messages[len] = message;
                len += 1;
            });
        self.send_messages(&messages[..len]);
        self.connection.is_some()
    }

    fn master_clock_period(&self) -> Duration {
        Duration::from_secs_f64(60.0 / (f64::from(self.master_bpm.clamp(30.0, 250.0)) * 24.0))
    }
}

/// Cloneable MIDI output connection shared by every UI-facing control handle.
#[derive(Clone)]
pub struct MidiOutputHandle {
    state: Arc<Mutex<MidiOutputState>>,
}

impl Default for MidiOutputHandle {
    fn default() -> Self {
        let state = Arc::new(Mutex::new(MidiOutputState {
            connection: None,
            encoder: rev2::ControllerEncoder::default(),
            last_nrpn_values: HashMap::new(),
            configured_port: None,
            available_ports: Vec::new(),
            last_tick: Instant::now(),
            recent_messages: VecDeque::new(),
            clock_mode: MidiClockMode::Off,
            output_clock_mode: MidiClockMode::Off,
            master_bpm: 120.0,
            next_clock_deadline: None,
            desired_patch: None,
            selected_layer: LayerId::A,
        }));
        spawn_master_clock_worker(&state);
        Self { state }
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
            state.clear();
            return true;
        };
        let Ok(midi_out) = MidiOutput::new("noctum-output") else {
            state.clear();
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
            state.clear();
            return false;
        };
        let Ok(connection) = midi_out.connect(&port, "noctum-midi-output") else {
            state.clear();
            return false;
        };
        state.connection = Some(connection);
        state.last_nrpn_values.clear();
        if !state.send_output_clock_mode() {
            return false;
        }
        state.resend_desired_patch()
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
                state.clear();
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

    pub fn set_clock_mode(&self, mode: MidiClockMode) {
        let mut state = self.state.lock();
        if state.clock_mode != mode {
            state.clock_mode = mode;
            state.next_clock_deadline = None;
        }
    }

    pub fn set_master_bpm(&self, bpm: f32) {
        self.state.lock().master_bpm = bpm.clamp(30.0, 250.0);
    }

    pub fn set_output_clock_mode(&self, mode: MidiClockMode) -> bool {
        let mut state = self.state.lock();
        state.output_clock_mode = mode;
        state.connection.is_none() || state.send_output_clock_mode()
    }

    pub fn send_master_volume(&self, volume: f32) {
        let mut state = self.state.lock();
        let mut message = [0_u8; 3];
        state.encoder.master_volume(0, volume, |encoded| {
            message = encoded;
        });
        state.send_controller_messages(core::slice::from_ref(&message));
    }

    pub fn send_raw(&self, message: &[u8]) {
        let mut state = self.state.lock();
        let Some(connection) = state.connection.as_mut() else {
            return;
        };
        if connection.send(message).is_err() {
            state.clear();
        } else {
            state.record_echo(message);
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
        state.consume_echo(message)
    }

    pub fn send_param(&self, layer: LayerId, param: ParamId, value: f32) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            patch.layer_mut(layer).set_param(param, value);
        }
        if param == ParamId::PanModMode {
            state.selected_layer = layer;
            // CC 10 is edit-targeted on the Rev2, so explicitly select the
            // requested layer even when our cached selection already matches.
            state.last_nrpn_values.remove(&4190);
            state.send_selected_layer();
        }
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        if !state
            .encoder
            .param_for_layer(0, layer, param, value, |message| {
                messages[len] = message;
                len += 1;
            })
        {
            return;
        }
        if param == ParamId::PanModMode {
            state.send_controller_messages(&messages[..len]);
        } else {
            state.send_messages(&messages[..len]);
        }
    }

    pub fn send_sequencer_running(&self, layer: LayerId, running: bool) {
        let mut state = self.state.lock();
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        state
            .encoder
            .sequencer_running(0, layer, running, |message| {
                messages[len] = message;
                len += 1;
            });
        state.send_controller_messages(&messages[..len]);
    }

    pub fn send_sequencer_recording(&self, layer: LayerId, recording: bool) {
        let mut state = self.state.lock();
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        state
            .encoder
            .sequencer_recording(0, layer, recording, |message| {
                messages[len] = message;
                len += 1;
            });
        state.send_controller_messages(&messages[..len]);
    }

    pub fn send_sequence(&self, layer: LayerId, update: SequenceUpdate) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            patch.layer_mut(layer).sequence.apply(update);
        }
        let mut messages = [[0_u8; 3]; 8];
        let mut len = 0;
        state.encoder.sequence(0, layer, update, |message| {
            messages[len] = message;
            len += 1;
        });
        state.send_messages(&messages[..len]);
    }

    pub fn clear_sequence(&self, layer: LayerId, section: SequenceClear) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            match section {
                SequenceClear::Gated => {
                    patch.layer_mut(layer).sequence.gated = synth_core::GatedSequence::default();
                }
                SequenceClear::Polyphonic => {
                    patch.layer_mut(layer).sequence.poly = synth_core::PolySequence::default();
                }
            }
        }
        let _ = state.resend_desired_patch();
    }

    pub fn send_modulation(
        &self,
        layer: LayerId,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            let layer_patch = patch.layer_mut(layer);
            match route {
                ModRoute::Free(index) => {
                    if let Some(slot) = layer_patch.mod_matrix.free_slots.get_mut(index) {
                        slot.enabled = enabled;
                        slot.source = source;
                        slot.destination = destination;
                        slot.amount = amount;
                    }
                }
                ModRoute::Dedicated(dedicated) => {
                    if let Some(slot) = layer_patch.mod_matrix.dedicated.get_mut(dedicated.index())
                    {
                        slot.enabled = enabled;
                        slot.destination = destination;
                        slot.amount = amount;
                    }
                }
            }
        }
        if state.connection.is_none() {
            return;
        }
        let mut messages = [[0_u8; 3]; 12];
        let mut len = 0;
        state.encoder.modulation_for_layer(
            0,
            layer,
            route,
            enabled,
            source,
            destination,
            amount,
            |message| {
                messages[len] = message;
                len += 1;
            },
        );
        state.send_messages(&messages[..len]);
    }

    pub fn send_edit_layer(&self, layer: LayerId) {
        let mut state = self.state.lock();
        state.selected_layer = layer;
        if let Some(patch) = state.desired_patch.as_ref() {
            state.master_bpm = patch.layer(layer).bpm.clamp(30.0, 250.0);
        }
        state.send_selected_layer();
    }

    pub fn send_layer_mode(&self, mode: LayerMode) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            patch.mode = mode;
        }
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        state.encoder.layer_mode(0, mode, |message| {
            messages[len] = message;
            len += 1;
        });
        state.send_messages(&messages[..len]);
    }

    pub fn send_split_point(&self, split_point: u8) {
        let mut state = self.state.lock();
        if let Some(patch) = state.desired_patch.as_mut() {
            patch.set_split_point(split_point);
        }
        let mut messages = [[0_u8; 3]; 4];
        let mut len = 0;
        state.encoder.split_point(0, split_point, |message| {
            messages[len] = message;
            len += 1;
        });
        state.send_messages(&messages[..len]);
    }

    pub fn send_patch(&self, patch: &Patch) -> bool {
        let mut state = self.state.lock();
        state.sync_encoder_from_patch(patch);
        state.desired_patch = Some(patch.clone());
        state.selected_layer = LayerId::A;
        state.resend_desired_patch()
    }

    pub fn cache_patch(&self, patch: &Patch) {
        let mut state = self.state.lock();
        state.sync_encoder_from_patch(patch);
        state.desired_patch = Some(patch.clone());
        state.selected_layer = LayerId::A;
    }

    pub fn cache_edit_layer(&self, layer: LayerId) {
        let mut state = self.state.lock();
        state.selected_layer = layer;
        if let Some(patch) = state.desired_patch.as_ref() {
            state.master_bpm = patch.layer(layer).bpm.clamp(30.0, 250.0);
        }
    }

    pub fn cache_layer_mode(&self, mode: LayerMode) {
        if let Some(patch) = self.state.lock().desired_patch.as_mut() {
            patch.mode = mode;
        }
    }

    pub fn cache_split_point(&self, split_point: u8) {
        if let Some(patch) = self.state.lock().desired_patch.as_mut() {
            patch.set_split_point(split_point);
        }
    }

    pub fn cache_param(&self, target: LayerTarget, param: ParamId, value: f32) {
        let mut state = self.state.lock();
        let layer = match target {
            LayerTarget::Edit => state.selected_layer,
            LayerTarget::Explicit(layer) => layer,
        };
        if let Some(patch) = state.desired_patch.as_mut() {
            patch.layer_mut(layer).set_param(param, value);
        }
        if param == ParamId::Bpm && layer == state.selected_layer {
            state.master_bpm = value.clamp(30.0, 250.0);
        }
        let _ = state
            .encoder
            .param_for_layer(0, layer, param, value, |_| {});
    }

    pub fn cache_modulation_param(
        &self,
        target: LayerTarget,
        route: ModRoute,
        parameter: ModulationParam,
    ) {
        let mut state = self.state.lock();
        let layer = match target {
            LayerTarget::Edit => state.selected_layer,
            LayerTarget::Explicit(layer) => layer,
        };
        if let Some(patch) = state.desired_patch.as_mut() {
            patch
                .layer_mut(layer)
                .set_modulation_param(route, parameter);
        }
    }
}

pub struct MidiInputManager {
    control: SynthEngineControl,
    output: MidiOutputHandle,
    clock_mode: SharedMidiClockMode,
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
            clock_mode: SharedMidiClockMode::new(MidiClockMode::Off),
            connections: HashMap::new(),
            pending: HashMap::new(),
            available_ports,
            last_tick: Instant::now(),
        }
    }

    pub fn sync(
        &mut self,
        entries: &[MidiInputEntry],
        clock_source: Option<&str>,
        clock_mode: MidiClockMode,
    ) {
        self.clock_mode.set(clock_mode);
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

            if let Some(connection) = self.connect_port(&entry.port, flags.clone()) {
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
            if let Some(connection) = self.connect_port(&port, flags.clone()) {
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

    fn connect_port(
        &self,
        port_name: &str,
        flags: Arc<MidiInputFlags>,
    ) -> Option<MidiInputConnection<()>> {
        let mut midi_in = MidiInput::new("noctum").ok()?;
        midi_in.ignore(Ignore::None);
        let ports = midi_in.ports();
        let filter_lower = port_name.to_lowercase();
        let port = ports.iter().find(|port| {
            midi_in
                .port_name(port)
                .map(|name| name.to_lowercase().contains(&filter_lower))
                .unwrap_or(false)
        })?;
        let mut decoder = rev2::ControllerDecoder::default();
        let input_port = port_name.to_owned();
        let control = self.control.clone();
        let output = self.output.clone();
        let clock_mode = self.clock_mode.clone();
        midi_in
            .connect(
                &port,
                &format!("midi-in-{port_name}"),
                move |timestamp, message, _| {
                    flags.handle_message(
                        timestamp,
                        message,
                        &input_port,
                        &control,
                        &mut decoder,
                        &output,
                        &clock_mode,
                    );
                },
                (),
            )
            .ok()
    }
}

fn handle_midi_sysex(
    message: &[u8],
    control: &SynthEngineControl,
    _decoder: &mut rev2::ControllerDecoder,
) {
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
        (0x2f, 0x02) => match rev2::decode::program_data(message) {
            Ok(program) => {
                if !control.queue_midi_program(ProgramData::Rev2(program)) {
                    eprintln!("MIDI program import queue is full");
                }
            }
            Err(err) => eprintln!(
                "Invalid Rev2 Program Data message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x2f, 0x03) => match rev2::decode::program_edit_buffer(message) {
            Ok(program) => control.load_midi_edit_buffer(&program),
            Err(err) => eprintln!(
                "Invalid Rev2 Program Edit Buffer message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x23, 0x02) => match p08::decode::program_data(message) {
            Ok(program) => {
                if !control.queue_midi_program(ProgramData::P08(program)) {
                    eprintln!("MIDI program import queue is full");
                }
            }
            Err(err) => eprintln!(
                "Invalid Prophet '08 Program Data message: {err:?} ({} bytes)",
                message.len()
            ),
        },
        (0x23, 0x03) => match p08::decode::program_edit_buffer(message) {
            Ok(patch) => control.load_midi_edit_buffer(&patch),
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
    decoder: &mut rev2::ControllerDecoder,
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

fn port_is_available(port: &str, available_ports: &[String]) -> bool {
    let port_lower = port.to_lowercase();
    available_ports.iter().any(|name| {
        name.to_lowercase().contains(&port_lower) || port_lower.contains(&name.to_lowercase())
    })
}

fn dispatch_inbound_update(control: &SynthEngineControl, update: rev2::MidiUpdate) {
    match update {
        rev2::MidiUpdate::Param {
            target,
            param,
            value,
        } => {
            control.set_midi_param(target, param, value);
        }
        rev2::MidiUpdate::Modulation {
            target,
            route,
            parameter,
        } => {
            control.set_midi_modulation_param(target, route, parameter);
        }
        rev2::MidiUpdate::MidiClockMode(mode) => control.set_midi_clock_mode(mode),
        rev2::MidiUpdate::MasterVolume(volume) => control.set_midi_master_volume(volume),
        rev2::MidiUpdate::LayerMode(mode) => control.set_midi_layer_mode(mode),
        rev2::MidiUpdate::SplitPoint(split_point) => control.set_midi_split_point(split_point),
        rev2::MidiUpdate::EditLayer(layer) => control.set_midi_edit_layer(layer),
        rev2::MidiUpdate::Sequence { target, update } => control.set_midi_sequence(target, update),
        rev2::MidiUpdate::SequencerRunning { target, running } => {
            control.set_midi_sequencer_running(target, running)
        }
        rev2::MidiUpdate::SequencerRecording { target, recording } => {
            control.set_midi_sequencer_recording(target, recording)
        }
    }
}

fn spawn_master_clock_worker(state: &Arc<Mutex<MidiOutputState>>) {
    let weak = Arc::downgrade(state);
    let _ = thread::Builder::new()
        .name("midi-clock-master".to_owned())
        .spawn(move || {
            loop {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let sleep_for = {
                    let mut state = state.lock();
                    if state.clock_mode != MidiClockMode::Master || state.connection.is_none() {
                        state.next_clock_deadline = None;
                        Duration::from_millis(10)
                    } else {
                        let now = Instant::now();
                        let deadline = state.next_clock_deadline.unwrap_or(now);
                        if now >= deadline {
                            let sent = state
                                .connection
                                .as_mut()
                                .is_some_and(|connection| connection.send(&[0xf8]).is_ok());
                            if sent {
                                state.record_echo(&[0xf8]);
                            } else {
                                state.clear();
                            }
                            let period = state.master_clock_period();
                            let next = deadline + period;
                            state.next_clock_deadline =
                                Some(if next <= now { now + period } else { next });
                        }
                        state
                            .next_clock_deadline
                            .unwrap_or(now + Duration::from_millis(1))
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(5))
                    }
                };
                thread::sleep(sleep_for.max(Duration::from_micros(100)));
            }
        });
}

fn midi_port_names_match(left: &str, right: &str) -> bool {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
}

/// Sends only NRPN sequences whose quantized value differs from the last value
/// successfully sent on this connection.
fn send_changed_nrpn_messages(
    last_values: &mut HashMap<u16, u16>,
    messages: &[[u8; 3]],
    mut send: impl FnMut(&[u8; 3]) -> bool,
) -> bool {
    debug_assert_eq!(messages.len() % 4, 0);
    for sequence in messages.chunks_exact(4) {
        debug_assert_eq!(sequence[0][1], 99);
        debug_assert_eq!(sequence[1][1], 98);
        debug_assert_eq!(sequence[2][1], 6);
        debug_assert_eq!(sequence[3][1], 38);

        let number = u16::from(sequence[0][2]) * 128 + u16::from(sequence[1][2]);
        let value = u16::from(sequence[2][2]) * 128 + usize::from(sequence[3][2]) as u16;
        if last_values.get(&number) == Some(&value) {
            continue;
        }
        if sequence.iter().any(|message| !send(message)) {
            return false;
        }
        last_values.insert(number, value);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{AudioCommand, MidiUiUpdate, create_synth_engine_bridge};
    use synth_core::{ControlMessage, LayerTarget, SequenceUpdate};

    fn handle_midi(
        message: &[u8],
        control: &SynthEngineControl,
        decoder: &mut rev2::ControllerDecoder,
    ) {
        if message.first() == Some(&0xf0) {
            handle_midi_sysex(message, control, decoder);
            return;
        }
        handle_midi_control(message, control, decoder);
    }

    fn test_output_state() -> MidiOutputState {
        MidiOutputState {
            connection: None,
            encoder: rev2::ControllerEncoder::default(),
            last_nrpn_values: HashMap::new(),
            configured_port: None,
            available_ports: Vec::new(),
            last_tick: Instant::now(),
            recent_messages: VecDeque::new(),
            clock_mode: MidiClockMode::Off,
            output_clock_mode: MidiClockMode::Off,
            master_bpm: 120.0,
            next_clock_deadline: None,
            desired_patch: None,
            selected_layer: LayerId::A,
        }
    }

    fn stored_program_message(bank: u8, program: u8) -> [u8; rev2::PROGRAM_DATA_SYSEX_LEN] {
        let mut edit = [0_u8; rev2::PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        rev2::encode::program_edit_buffer(&Patch::default(), &mut edit).unwrap();
        let mut message = [0_u8; rev2::PROGRAM_DATA_SYSEX_LEN];
        message[..6].copy_from_slice(&[0xf0, 0x01, 0x2f, 0x02, bank, program]);
        let payload_end = message.len() - 1;
        message[6..payload_end].copy_from_slice(&edit[4..edit.len() - 1]);
        message[payload_end] = 0xf7;
        message
    }

    fn p08_stored_program_message(bank: u8, program: u8) -> [u8; p08::PROGRAM_DATA_SYSEX_LEN] {
        // An all-zero packed payload is a valid default Prophet '08 program.
        let mut message = [0_u8; p08::PROGRAM_DATA_SYSEX_LEN];
        message[..6].copy_from_slice(&[0xf0, 0x01, 0x23, 0x02, bank, program]);
        message[p08::PROGRAM_DATA_SYSEX_LEN - 1] = 0xf7;
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

    fn shared_clock_mode(mode: MidiClockMode) -> SharedMidiClockMode {
        SharedMidiClockMode::new(mode)
    }

    #[test]
    fn inbound_nrpn_fans_out_to_engine_and_ui_without_output_path() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = rev2::ControllerDecoder::default();
        for (controller, value) in [(99, 0), (98, 20), (6, 1), (38, 126)] {
            assert!(decoder.control_change(0, controller, value, |update| {
                dispatch_inbound_update(&bridge.control, update);
            }));
        }
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::SetParam {
                target: LayerTarget::Explicit(LayerId::A),
                param: ParamId::FilterEnvAmount,
                value: 1.0,
            }))
        ));
        let mut ui_update = None;
        bridge
            .view
            .drain_midi_ui_updates(|update| ui_update = Some(update));
        assert!(matches!(
            ui_update,
            Some(MidiUiUpdate::Param {
                target: LayerTarget::Explicit(LayerId::A),
                param: ParamId::FilterEnvAmount,
                value: 1.0,
            })
        ));
    }

    #[test]
    fn inbound_layer_mode_and_split_update_engine_and_ui_without_echo() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let output = bridge.control.midi_output();
        output.cache_patch(&Patch::default());

        dispatch_inbound_update(
            &bridge.control,
            rev2::MidiUpdate::LayerMode(LayerMode::Split),
        );
        dispatch_inbound_update(&bridge.control, rev2::MidiUpdate::SplitPoint(72));

        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::SetLayerMode(
                LayerMode::Split
            )))
        ));
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::SetSplitPoint(72)))
        ));

        let mut updates = Vec::new();
        bridge
            .view
            .drain_midi_ui_updates(|update| updates.push(update));
        assert!(
            updates
                .iter()
                .any(|update| matches!(update, MidiUiUpdate::LayerMode(LayerMode::Split)))
        );
        assert!(
            updates
                .iter()
                .any(|update| matches!(update, MidiUiUpdate::SplitPoint(72)))
        );

        let state = output.state.lock();
        let cached = state.desired_patch.as_ref().unwrap();
        assert_eq!(cached.mode, LayerMode::Split);
        assert_eq!(cached.split_point, 72);
        assert!(state.last_nrpn_values.is_empty());
    }

    #[test]
    fn inbound_layer_targets_are_preserved_for_the_dual_layer_engine() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        for target in [LayerTarget::Edit, LayerTarget::Explicit(LayerId::A)] {
            dispatch_inbound_update(
                &bridge.control,
                rev2::MidiUpdate::Param {
                    target,
                    param: ParamId::FilterResonance,
                    value: 0.5,
                },
            );
            assert!(matches!(
                audio.control.0.pop(),
                Ok(AudioCommand::Control(ControlMessage::SetParam {
                    target: queued_target,
                    param: ParamId::FilterResonance,
                    value: 0.5,
                })) if queued_target == target
            ));
        }

        dispatch_inbound_update(
            &bridge.control,
            rev2::MidiUpdate::Param {
                target: LayerTarget::Explicit(LayerId::B),
                param: ParamId::FilterResonance,
                value: 1.0,
            },
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::SetParam {
                target: LayerTarget::Explicit(LayerId::B),
                param: ParamId::FilterResonance,
                value: 1.0,
            }))
        ));
    }

    #[test]
    fn output_handle_starts_disconnected() {
        let output = MidiOutputHandle::default();
        assert!(!output.is_connected());
        assert!(!output.send_patch(&Patch::default()));
    }

    #[test]
    fn disconnected_output_retains_patch_edits_and_clock_mode() {
        let output = MidiOutputHandle::default();
        assert!(!output.send_patch(&Patch::default()));

        output.send_param(LayerId::A, ParamId::Bpm, 137.0);
        assert!(output.set_output_clock_mode(MidiClockMode::SlaveNoStartStop));

        let state = output.state.lock();
        assert_eq!(state.desired_patch.as_ref().unwrap().layer_a.bpm, 137.0);
        assert_eq!(state.output_clock_mode, MidiClockMode::SlaveNoStartStop);
    }

    #[test]
    fn midi_output_suppresses_repeated_quantized_nrpn_values() {
        let mut cache = HashMap::new();
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
        let mut cache = HashMap::new();
        let messages = [[0xb0, 99, 0], [0xb0, 98, 33], [0xb0, 6, 0], [0xb0, 38, 13]];
        let mut sent = 0;

        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        cache.clear();
        assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
            sent += 1;
            true
        }));
        assert_eq!(sent, 8);
    }

    #[test]
    fn midi_output_cache_accepts_program_and_global_nrpn_numbers() {
        let mut cache = HashMap::new();
        let mut sent = 0;
        for messages in [
            [[0xb0, 99, 1], [0xb0, 98, 51], [0xb0, 6, 0], [0xb0, 38, 120]],
            [[0xb0, 99, 32], [0xb0, 98, 3], [0xb0, 6, 0], [0xb0, 38, 2]],
        ] {
            assert!(send_changed_nrpn_messages(&mut cache, &messages, |_| {
                sent += 1;
                true
            }));
        }
        assert_eq!(sent, 8);
        assert_eq!(cache.get(&179), Some(&120));
        assert_eq!(cache.get(&4099), Some(&2));
    }

    #[test]
    fn output_cache_tracks_both_layers_topology_and_selected_tempo() {
        let output = MidiOutputHandle::default();
        let mut patch = Patch::default();
        patch.layer_a.bpm = 90.0;
        patch.layer_b.bpm = 150.0;
        output.cache_patch(&patch);
        output.cache_edit_layer(LayerId::B);
        output.cache_layer_mode(LayerMode::Split);
        output.cache_split_point(72);
        output.cache_param(
            LayerTarget::Explicit(LayerId::A),
            ParamId::FilterResonance,
            0.25,
        );
        output.send_param(LayerId::B, ParamId::PanModMode, 1.0);

        let state = output.state.lock();
        let cached = state.desired_patch.as_ref().unwrap();
        assert_eq!(state.selected_layer, LayerId::B);
        assert_eq!(state.master_bpm, 150.0);
        assert_eq!(cached.mode, LayerMode::Split);
        assert_eq!(cached.split_point, 72);
        assert_eq!(cached.layer_a.filter.resonance, 0.25);
        assert_eq!(
            cached.layer_b.amplifier.pan_mod_mode,
            synth_core::PanModMode::Fixed
        );
    }

    #[test]
    fn ui_topology_and_sequencer_sends_update_the_output_cache() {
        let output = MidiOutputHandle::default();
        output.cache_patch(&Patch::default());

        output.send_layer_mode(LayerMode::Stack);
        output.send_split_point(72);
        output.send_edit_layer(LayerId::B);
        output.send_sequencer_running(LayerId::B, true);
        output.send_sequencer_recording(LayerId::B, true);
        output.send_sequence(
            LayerId::B,
            SequenceUpdate::Type(synth_core::SequencerType::Gated),
        );

        let state = output.state.lock();
        let cached = state.desired_patch.as_ref().unwrap();
        assert_eq!(cached.mode, LayerMode::Stack);
        assert_eq!(cached.split_point, 72);
        assert_eq!(state.selected_layer, LayerId::B);
        assert_eq!(
            cached.layer_b.sequence.sequencer_type,
            synth_core::SequencerType::Gated
        );
    }

    #[test]
    fn master_clock_period_is_24_ppqn() {
        let mut state = test_output_state();
        state.master_bpm = 120.0;
        assert!((state.master_clock_period().as_secs_f64() - 1.0 / 48.0).abs() < 1.0e-9);
        state.master_bpm = 60.0;
        assert!((state.master_clock_period().as_secs_f64() - 1.0 / 24.0).abs() < 1.0e-9);
    }

    #[test]
    fn selected_clock_is_consumed_only_in_slave_mode() {
        for (mode, expected) in [
            (MidiClockMode::Off, false),
            (MidiClockMode::Slave, true),
            (MidiClockMode::Master, false),
        ] {
            let (mut audio, bridge) = create_synth_engine_bridge(16);
            let output = MidiOutputHandle::default();
            let flags = all_flags();
            flags.clock.store(true, Ordering::Relaxed);
            let mut decoder = rev2::ControllerDecoder::default();
            flags.as_ref().handle_message(
                42,
                &[0xf8],
                "clock source",
                &bridge.control,
                &mut decoder,
                &output,
                &shared_clock_mode(mode),
            );
            assert_eq!(audio.control.0.pop().is_ok(), expected, "mode={mode:?}");
        }
    }

    #[test]
    fn midi_echo_cache_consumes_only_matching_recent_output() {
        let mut state = test_output_state();
        state.record_echo(&[0xf0, 0x01, 0xf7]);

        assert!(!state.consume_echo(&[0x90, 60, 100]));
        assert!(state.consume_echo(&[0xf0, 0x01, 0xf7]));
        assert!(!state.consume_echo(&[0xf0, 0x01, 0xf7]));
    }

    #[test]
    fn midi_echo_cache_does_not_consume_expired_output() {
        let mut state = test_output_state();
        state.recent_messages.push_back((
            Instant::now() - MIDI_ECHO_TTL - Duration::from_millis(1),
            vec![0xb0, 14, 90],
        ));

        assert!(!state.consume_echo(&[0xb0, 14, 90]));
        assert!(state.recent_messages.is_empty());
    }

    #[test]
    fn output_echo_cache_does_not_swallow_note_off_from_another_input_port() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let output = MidiOutputHandle::default();
        {
            let mut state = output.state.lock();
            state.configured_port = Some("Noctum USB MIDI (development)".to_owned());
            // These model identical NoteOff messages forwarded during the
            // immediately preceding chord, still inside the echo TTL.
            state.record_echo(&[0x80, 64, 0]);
            state.record_echo(&[0x80, 62, 0]);
        }
        let mut decoder = rev2::ControllerDecoder::default();
        let flags = all_flags();

        for message in [[0x80, 64, 0], [0x80, 62, 0]] {
            flags.as_ref().handle_message(
                0,
                &message,
                "Arturia MiniLab mkII",
                &bridge.control,
                &mut decoder,
                &output,
                &shared_clock_mode(MidiClockMode::Off),
            );
        }

        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::NoteOff { note: 64 }))
        ));
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::NoteOff { note: 62 }))
        ));
        assert_eq!(output.state.lock().recent_messages.len(), 2);
    }

    #[test]
    fn echoed_patch_is_not_applied_to_ui() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let output = MidiOutputHandle::default();
        let mut message = [0_u8; rev2::PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        rev2::encode::program_edit_buffer(&Patch::default(), &mut message).unwrap();
        {
            let mut state = output.state.lock();
            state.configured_port = Some("loopback".to_owned());
            state.record_echo(&message);
        }
        let mut decoder = rev2::ControllerDecoder::default();

        all_flags().as_ref().handle_message(
            0,
            &message,
            "loopback",
            &bridge.control,
            &mut decoder,
            &output,
            &shared_clock_mode(MidiClockMode::Off),
        );

        let mut updates = 0;
        bridge.view.drain_midi_ui_updates(|_| updates += 1);
        assert_eq!(updates, 0);
    }

    #[test]
    fn inbound_edit_buffer_updates_engine_and_ui_path() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut program = Patch::default();
        program.mode = synth_core::LayerMode::Stack;
        program.layer_a.filter.resonance = 1.0;
        program.layer_b.filter.resonance = 0.5;
        program.layer_a.sequence.sequencer_type = synth_core::SequencerType::Polyphonic;
        program.layer_a.sequence.gated.tracks[0].steps[0] = synth_core::GatedStep::Value(125);
        program.layer_a.sequence.poly.steps[0].lanes[0].note = synth_core::PolyNote::Tie;
        program.layer_a.sequence.poly.steps[0].lanes[0].velocity =
            synth_core::PolyVelocity::Velocity(127);
        let mut message = [0_u8; rev2::PROGRAM_EDIT_BUFFER_SYSEX_LEN];
        rev2::encode::program_edit_buffer(&program, &mut message).unwrap();
        let mut decoder = rev2::ControllerDecoder::default();
        handle_midi(&message, &bridge.control, &mut decoder);

        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Program(patch))
                if patch.mode == synth_core::LayerMode::Stack
                    && (patch.layer_b.filter.resonance - 0.5).abs() < 0.01
                    && patch.layer_a.sequence.poly.steps[0].lanes[0].note
                        == synth_core::PolyNote::Tie
        ));
        let mut ui_program = None;
        bridge.view.drain_midi_ui_updates(|update| {
            if let MidiUiUpdate::Program(program) = update {
                ui_program = Some(program);
            }
        });
        let ui_program = ui_program.expect("program UI update");
        assert_eq!(
            ui_program.layer_a.sequence.gated.tracks[0].steps[0],
            synth_core::GatedStep::Value(125)
        );
        assert_eq!(
            ui_program.layer_a.sequence.poly.steps[0].lanes[0].velocity,
            synth_core::PolyVelocity::Velocity(127)
        );
        assert_eq!(ui_program.mode, synth_core::LayerMode::Stack);
        assert!((ui_program.layer_a.filter.resonance - 1.0).abs() < 0.01);
        assert!((ui_program.layer_b.filter.resonance - 0.5).abs() < 0.01);
    }

    #[test]
    fn inbound_stored_program_is_queued_without_changing_current_patch() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let message = stored_program_message(4, 0);
        let mut decoder = rev2::ControllerDecoder::default();
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
    fn inbound_p08_stored_program_is_queued_without_changing_current_patch() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let message = p08_stored_program_message(1, 99);
        let mut decoder = rev2::ControllerDecoder::default();
        handle_midi(&message, &bridge.control, &mut decoder);

        let mut imported = None;
        bridge
            .view
            .drain_midi_program_imports(|program| imported = Some(program));
        let imported = imported.unwrap();
        assert!(matches!(&imported, ProgramData::P08(_)));
        assert_eq!((imported.bank(), imported.program()), (1, 99));
        let mut ui_updates = 0;
        bridge.view.drain_midi_ui_updates(|_| ui_updates += 1);
        assert_eq!(ui_updates, 0);
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn splits_batched_program_sysex_frames() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let first = stored_program_message(4, 0);
        let second = stored_program_message(4, 1);
        let mut batch = Vec::with_capacity(first.len() + second.len());
        batch.extend_from_slice(&first);
        batch.extend_from_slice(&second);
        let mut decoder = rev2::ControllerDecoder::default();
        handle_midi(&batch, &bridge.control, &mut decoder);

        let mut locations = Vec::new();
        bridge.view.drain_midi_program_imports(|program| {
            locations.push((program.bank(), program.program()))
        });
        assert_eq!(locations, [(4, 0), (4, 1)]);
        let mut ui_updates = 0;
        bridge.view.drain_midi_ui_updates(|_| ui_updates += 1);
        assert_eq!(ui_updates, 0);
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn control_flag_gates_note_messages() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = rev2::ControllerDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(false),
            patches: AtomicBool::new(true),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(false),
        });
        flags.as_ref().handle_message(
            0,
            &[0x90, 60, 100],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &shared_clock_mode(MidiClockMode::Off),
        );
        assert!(audio.control.0.pop().is_err());
    }

    #[test]
    fn selected_clock_source_routes_realtime_with_midir_timestamp() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = rev2::ControllerDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(false),
            patches: AtomicBool::new(false),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(true),
        });
        let clock = shared_clock_mode(MidiClockMode::Slave);
        flags.as_ref().handle_message(
            123_456,
            &[0xf8],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &clock,
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::MidiRealtime(
                MidiRealtimeEvent::TimingClock {
                    timestamp_micros: 123_456
                }
            )))
        ));

        flags.as_ref().handle_message(
            0,
            &[0xfb],
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &clock,
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::MidiRealtime(
                MidiRealtimeEvent::Continue
            )))
        ));
    }

    #[test]
    fn patches_flag_gates_sysex_messages() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        let mut decoder = rev2::ControllerDecoder::default();
        let flags = Arc::new(MidiInputFlags {
            control: AtomicBool::new(true),
            patches: AtomicBool::new(false),
            forward: AtomicBool::new(false),
            clock: AtomicBool::new(false),
        });
        let message = stored_program_message(4, 0);
        flags.as_ref().handle_message(
            0,
            &message,
            "test input",
            &bridge.control,
            &mut decoder,
            &MidiOutputHandle::default(),
            &shared_clock_mode(MidiClockMode::Off),
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
        let mut decoder = rev2::ControllerDecoder::default();
        let output = MidiOutputHandle::default();
        all_flags().as_ref().handle_message(
            0,
            &[0x90, 60, 100],
            "test input",
            &bridge.control,
            &mut decoder,
            &output,
            &shared_clock_mode(MidiClockMode::Off),
        );
        assert!(matches!(
            audio.control.0.pop(),
            Ok(AudioCommand::Control(ControlMessage::NoteOn {
                note: 60,
                ..
            }))
        ));
    }
}
