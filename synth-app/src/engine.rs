//! Lock-free bridge between the UI/MIDI threads and the audio thread.

use parking_lot::{Mutex, RwLock};
use rtrb::{PushError, RingBuffer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use synth_core::dsp::{FilterOversampling, FilterType};
use synth_core::{
    ChordMemory, ControlMessage, MidiClockMode, MidiClockStatus, MidiProgramImport,
    MidiRealtimeEvent, ModDestination, ModRoute, ModSource, ModulationParam, ParamId, Patch,
};

use crate::midi::MidiOutputHandle;

pub const MAX_AUDIO_BUF: usize = 1024;

/// Synchronized stereo input and synth-output samples for real-time analysis.
pub struct AudioBlock {
    pub input_left: [f32; MAX_AUDIO_BUF],
    pub input_right: [f32; MAX_AUDIO_BUF],
    pub output_left: [f32; MAX_AUDIO_BUF],
    pub output_right: [f32; MAX_AUDIO_BUF],
    pub len: u16,
}

impl Default for AudioBlock {
    fn default() -> Self {
        Self {
            input_left: [0.0; MAX_AUDIO_BUF],
            input_right: [0.0; MAX_AUDIO_BUF],
            output_left: [0.0; MAX_AUDIO_BUF],
            output_right: [0.0; MAX_AUDIO_BUF],
            len: 0,
        }
    }
}

const MAX_PENDING_BLOCKS: usize = 32;
const CONTROL_QUEUE_CAPACITY: usize = 256;
const FEEDBACK_QUEUE_CAPACITY: usize = 64;
const MIDI_UI_QUEUE_CAPACITY: usize = 1024;
const MIDI_PROGRAM_IMPORT_QUEUE_CAPACITY: usize = 1024;

/// A parameter change originating in the MIDI decoder that the UI must mirror.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiUiUpdate {
    Param(ParamId, f32),
    Modulation {
        route: ModRoute,
        parameter: ModulationParam,
    },
}

/// Snapshot of audio-thread timing metrics, reported roughly once per second.
#[derive(Clone, Copy, Default)]
pub struct AudioMetrics {
    pub deadline_ms: f64,
    pub callback_avg_ms: f64,
    pub callback_max_ms: f64,
    pub render_avg_ms: f64,
    pub render_max_ms: f64,
    pub overruns: u64,
    pub render_overruns: u64,
    pub callbacks: u64,
}

pub enum FeedbackMessage {
    Audio(AudioBlock),
    Metrics(AudioMetrics),
    MidiClock(MidiClockStatus),
}

pub struct SynthEngineFeedback {
    active_voices: Arc<AtomicUsize>,
    sender: rtrb::Producer<FeedbackMessage>,
}

impl SynthEngineFeedback {
    pub fn set_active_voices(&self, count: usize) {
        self.active_voices.store(count, Ordering::Relaxed);
    }

    pub fn push_audio_block(&mut self, block: AudioBlock) {
        let _ = self.sender.push(FeedbackMessage::Audio(block));
    }

    pub fn push_metrics(&mut self, metrics: AudioMetrics) {
        let _ = self.sender.push(FeedbackMessage::Metrics(metrics));
    }

    pub fn push_midi_clock(&mut self, status: MidiClockStatus) -> bool {
        self.sender.push(FeedbackMessage::MidiClock(status)).is_ok()
    }
}

/// Read-only view of engine state for the UI (active voices, captured audio).
#[derive(Clone)]
pub struct SynthEngineView {
    active_voices: Arc<AtomicUsize>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
    metrics: Arc<RwLock<Option<AudioMetrics>>>,
    midi_clock: Arc<RwLock<Option<MidiClockStatus>>>,
    feedback_receiver: Arc<Mutex<rtrb::Consumer<FeedbackMessage>>>,
    midi_ui_receiver: Arc<Mutex<rtrb::Consumer<MidiUiUpdate>>>,
    midi_program_receiver: Arc<Mutex<rtrb::Consumer<Box<MidiProgramImport>>>>,
    total_voices: usize,
}

impl SynthEngineView {
    pub fn active_voices(&self) -> usize {
        self.active_voices.load(Ordering::Relaxed)
    }

    pub fn total_voices(&self) -> usize {
        self.total_voices
    }

    pub fn metrics(&self) -> Option<AudioMetrics> {
        *self.metrics.read()
    }

    pub fn drain_feedback(&self) {
        let mut receiver = self.feedback_receiver.lock();
        while let Ok(message) = receiver.pop() {
            match message {
                FeedbackMessage::Audio(block) => {
                    let mut blocks = self.audio_blocks.write();
                    if blocks.len() >= MAX_PENDING_BLOCKS {
                        blocks.pop_front();
                    }
                    blocks.push_back(block);
                }
                FeedbackMessage::Metrics(m) => {
                    *self.metrics.write() = Some(m);
                }
                FeedbackMessage::MidiClock(status) => {
                    *self.midi_clock.write() = Some(status);
                }
            }
        }
    }

    pub fn drain_audio_blocks(&self) -> VecDeque<AudioBlock> {
        std::mem::take(&mut *self.audio_blocks.write())
    }

    pub fn drain_midi_ui_updates(&self, mut handler: impl FnMut(MidiUiUpdate)) {
        let mut receiver = self.midi_ui_receiver.lock();
        while let Ok(update) = receiver.pop() {
            handler(update);
        }
    }

    pub fn drain_midi_program_imports(&self, mut handler: impl FnMut(MidiProgramImport)) {
        let mut receiver = self.midi_program_receiver.lock();
        while let Ok(program) = receiver.pop() {
            handler(*program);
        }
    }
}

type ControlProducer = rtrb::Producer<ControlMessage>;
type ControlConsumer = rtrb::Consumer<ControlMessage>;

/// Thread-safe handle for sending [`ControlMessage`] values to the audio thread.
#[derive(Clone)]
pub struct SynthEngineControl {
    sender: Arc<Mutex<ControlProducer>>,
    midi_ui_sender: Arc<Mutex<rtrb::Producer<MidiUiUpdate>>>,
    midi_program_sender: Arc<Mutex<rtrb::Producer<Box<MidiProgramImport>>>>,
    midi_output: MidiOutputHandle,
    midi_clock_status: Arc<RwLock<Option<MidiClockStatus>>>,
    input_enabled: Arc<AtomicBool>,
    held_notes: Arc<Mutex<[bool; 128]>>,
}

impl SynthEngineControl {
    pub fn set_modulation(
        &self,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        self.send(ControlMessage::SetModulation {
            route,
            enabled,
            source,
            destination,
            amount,
        });
        self.midi_output
            .send_modulation(route, enabled, source, destination, amount);
    }

    pub fn set_param(&self, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam(param, value));
        if param == ParamId::Bpm {
            self.midi_output.set_master_bpm(value);
        }
        self.midi_output.send_param(param, value);
    }

    pub fn set_param_audio_only(&self, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam(param, value));
    }

    /// Sends a MIDI-originated parameter change to audio and mirrors it to UI.
    pub fn set_midi_param(&self, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam(param, value));
        if param == ParamId::Bpm {
            self.midi_output.set_master_bpm(value);
        }
        self.send_midi_ui(MidiUiUpdate::Param(param, value));
    }

    /// Sends one MIDI-originated modulation field to audio and UI.
    pub fn set_midi_modulation_param(&self, route: ModRoute, parameter: ModulationParam) {
        self.send(ControlMessage::SetModulationParam { route, parameter });
        self.send_midi_ui(MidiUiUpdate::Modulation { route, parameter });
    }

    pub fn set_filter_oversampling(&self, oversampling: FilterOversampling) {
        self.send(ControlMessage::SetFilterOversampling(oversampling));
    }

    pub fn set_filter_type(&self, filter_type: FilterType) {
        self.send(ControlMessage::SetFilterType(filter_type));
    }

    pub fn set_midi_clock_mode(&self, mode: MidiClockMode) {
        self.send(ControlMessage::SetMidiClockMode(mode));
        self.midi_output.set_clock_mode(mode);
    }

    pub fn set_midi_output_clock_mode(&self, mode: MidiClockMode) -> bool {
        self.midi_output.set_output_clock_mode(mode)
    }

    pub fn midi_realtime(&self, event: MidiRealtimeEvent) {
        self.send(ControlMessage::MidiRealtime(event));
    }

    pub fn clock_status_for_ui(&self) -> Option<MidiClockStatus> {
        *self.midi_clock_status.read()
    }

    pub fn note_on(&self, note: u8, velocity: f32) {
        if note < 128 && velocity > 0.0 {
            self.held_notes.lock()[usize::from(note)] = true;
        } else if note < 128 {
            self.held_notes.lock()[usize::from(note)] = false;
        }
        self.send(ControlMessage::NoteOn { note, velocity });
    }

    pub fn note_off(&self, note: u8) {
        if note < 128 {
            self.held_notes.lock()[usize::from(note)] = false;
        }
        self.send(ControlMessage::NoteOff { note });
    }

    pub fn all_notes_off(&self) {
        *self.held_notes.lock() = [false; 128];
        self.send(ControlMessage::AllNotesOff);
        for channel in 0..16 {
            self.midi_output.send_raw(&[0xB0 | channel, 123, 0]);
            self.midi_output.send_raw(&[0xB0 | channel, 120, 0]);
        }
    }

    /// Captures the unique notes currently held by UI or MIDI input.
    pub fn capture_unison_chord(&self) -> Option<ChordMemory> {
        let held = self.held_notes.lock();
        let chord = ChordMemory::from_notes(
            held.iter()
                .copied()
                .enumerate()
                .filter_map(|(note, is_held)| is_held.then_some(note as u8)),
        );
        (!chord.is_empty()).then_some(chord)
    }

    pub fn set_unison_chord(&self, chord: ChordMemory) {
        self.send(ControlMessage::SetUnisonChord(chord));
    }

    pub fn pitch_bend(&self, value: f32) {
        self.send(ControlMessage::PitchBend { value });
    }

    pub fn mod_wheel(&self, value: f32) {
        self.send(ControlMessage::ModWheel { value });
    }

    pub fn pressure(&self, value: f32) {
        self.send(ControlMessage::Pressure { value });
    }

    pub fn sustain_pedal(&self, pressed: bool) {
        self.send(ControlMessage::SustainPedal { pressed });
    }

    pub fn control_change(&self, controller: u8, value: f32) {
        self.send(ControlMessage::ControlChange { controller, value });
    }

    pub fn set_midi_output_port(&self, port_name: Option<&str>) -> bool {
        self.midi_output.connect(port_name)
    }

    pub fn midi_output(&self) -> MidiOutputHandle {
        self.midi_output.clone()
    }

    pub fn midi_output_connected(&self) -> bool {
        self.midi_output.is_connected()
    }

    /// Enables or disables mixing of the audio input into the output at runtime.
    pub fn set_input_enabled(&self, enabled: bool) {
        self.input_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn load_patch(&self, patch: &Patch) {
        self.midi_output.set_master_bpm(patch.bpm);
        self.set_unison_chord(patch.unison_chord);
        patch.for_each_param(|id, value| self.send(ControlMessage::SetParam(id, value)));
        patch.for_each_modulation(|route, slot| {
            self.send(ControlMessage::SetModulation {
                route,
                enabled: slot.enabled,
                source: slot.source,
                destination: slot.destination,
                amount: slot.amount,
            });
        });
        let _ = self.midi_output.send_patch(patch);
    }

    pub fn load_patch_respecting_mute(&self, patch: &Patch, muted: bool) {
        self.load_patch(patch);
        if muted {
            self.set_param_audio_only(ParamId::MasterVolume, 0.0);
        }
    }

    /// Sends a complete patch to the selected MIDI output without changing local state.
    pub fn send_midi_patch(&self, patch: &Patch) -> bool {
        self.midi_output.send_patch(patch)
    }

    /// Applies a complete MIDI-originated patch without echoing it to MIDI output.
    pub fn load_midi_patch(&self, patch: &Patch) {
        self.midi_output.set_master_bpm(patch.bpm);
        self.set_unison_chord(patch.unison_chord);
        patch.for_each_param(|id, value| self.set_midi_param(id, value));
        patch.for_each_modulation(|route, slot| {
            self.send(ControlMessage::SetModulation {
                route,
                enabled: slot.enabled,
                source: slot.source,
                destination: slot.destination,
                amount: slot.amount,
            });
            self.send_midi_ui(MidiUiUpdate::Modulation {
                route,
                parameter: ModulationParam::Source(slot.source),
            });
            self.send_midi_ui(MidiUiUpdate::Modulation {
                route,
                parameter: ModulationParam::Amount(slot.amount),
            });
            self.send_midi_ui(MidiUiUpdate::Modulation {
                route,
                parameter: ModulationParam::Destination(slot.destination),
            });
        });
    }

    pub fn queue_midi_program(&self, program: MidiProgramImport) -> bool {
        self.midi_program_sender
            .lock()
            .push(Box::new(program))
            .is_ok()
    }

    /// Whether audio input is currently mixed into the output.
    pub fn input_enabled(&self) -> bool {
        self.input_enabled.load(Ordering::Relaxed)
    }

    fn send(&self, message: ControlMessage) {
        let performance_event = matches!(
            &message,
            ControlMessage::NoteOn { .. }
                | ControlMessage::NoteOff { .. }
                | ControlMessage::AllNotesOff
                | ControlMessage::SustainPedal { .. }
        );
        let mut pending = message;
        loop {
            let result = {
                let mut sender = self.sender.lock();
                sender.push(pending)
            };
            match result {
                Ok(()) => return,
                Err(PushError::Full(message)) if performance_event => {
                    pending = message;
                    // Drop the producer lock before yielding so audio-session
                    // rebind can replace a queue whose consumer has stopped.
                    std::thread::yield_now();
                }
                Err(PushError::Full(_)) => return,
            }
        }
    }

    fn send_midi_ui(&self, update: MidiUiUpdate) {
        let _ = self.midi_ui_sender.lock().push(update);
    }
}

pub struct SynthEngineControlReceiver(pub ControlConsumer);

impl SynthEngineControlReceiver {
    pub fn drain<F: FnMut(ControlMessage)>(&mut self, mut handler: F) {
        while let Ok(message) = self.0.pop() {
            handler(message);
        }
    }
}

/// UI/MIDI-facing half of the engine bridge (control sender + state view).
#[derive(Clone)]
pub struct SynthEngineBridge {
    pub control: SynthEngineControl,
    pub view: SynthEngineView,
}

/// Audio-thread half of the engine bridge (control receiver + feedback sender).
pub struct SynthEngineAudio {
    pub control: SynthEngineControlReceiver,
    pub feedback: SynthEngineFeedback,
    /// Shared flag toggled from the UI to mute the audio input at runtime.
    pub input_enabled: Arc<AtomicBool>,
}

/// Creates the control ring buffer and UI-facing engine bridge.
pub fn create_synth_engine_bridge(total_voices: usize) -> (SynthEngineAudio, SynthEngineBridge) {
    let (feedback_sender, feedback_receiver) = RingBuffer::new(FEEDBACK_QUEUE_CAPACITY);
    let (control_sender, control_receiver) = RingBuffer::new(CONTROL_QUEUE_CAPACITY);
    let (midi_ui_sender, midi_ui_receiver) = RingBuffer::new(MIDI_UI_QUEUE_CAPACITY);
    let (midi_program_sender, midi_program_receiver) =
        RingBuffer::new(MIDI_PROGRAM_IMPORT_QUEUE_CAPACITY);
    let active_voices = Arc::new(AtomicUsize::new(0));
    let audio_blocks = Arc::new(RwLock::new(VecDeque::new()));
    let metrics = Arc::new(RwLock::new(None));
    let midi_clock = Arc::new(RwLock::new(None));
    let input_enabled = Arc::new(AtomicBool::new(true));
    let feedback_receiver = Arc::new(Mutex::new(feedback_receiver));

    let bridge = SynthEngineBridge {
        control: SynthEngineControl {
            sender: Arc::new(Mutex::new(control_sender)),
            midi_ui_sender: Arc::new(Mutex::new(midi_ui_sender)),
            midi_program_sender: Arc::new(Mutex::new(midi_program_sender)),
            midi_output: MidiOutputHandle::default(),
            midi_clock_status: midi_clock.clone(),
            input_enabled: input_enabled.clone(),
            held_notes: Arc::new(Mutex::new([false; 128])),
        },
        view: SynthEngineView {
            active_voices: active_voices.clone(),
            audio_blocks,
            metrics,
            midi_clock,
            feedback_receiver,
            midi_ui_receiver: Arc::new(Mutex::new(midi_ui_receiver)),
            midi_program_receiver: Arc::new(Mutex::new(midi_program_receiver)),
            total_voices,
        },
    };
    let audio = SynthEngineAudio {
        control: SynthEngineControlReceiver(control_receiver),
        feedback: SynthEngineFeedback {
            active_voices,
            sender: feedback_sender,
        },
        input_enabled,
    };
    (audio, bridge)
}

/// Allocates fresh control and feedback ring buffers and rebinds the UI/view
/// ends. Returns the audio-thread ends for a new CPAL session.
pub fn rebind_audio_channels(bridge: &SynthEngineBridge) -> SynthEngineAudio {
    let (control_sender, control_receiver) = RingBuffer::new(CONTROL_QUEUE_CAPACITY);
    let (feedback_sender, feedback_receiver) = RingBuffer::new(FEEDBACK_QUEUE_CAPACITY);
    *bridge.control.sender.lock() = control_sender;
    *bridge.view.feedback_receiver.lock() = feedback_receiver;
    SynthEngineAudio {
        control: SynthEngineControlReceiver(control_receiver),
        feedback: SynthEngineFeedback {
            active_voices: bridge.view.active_voices.clone(),
            sender: feedback_sender,
        },
        input_enabled: bridge.control.input_enabled.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_capture_tracks_unique_held_notes_and_releases() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        bridge.control.note_on(64, 1.0);
        bridge.control.note_on(67, 0.8);
        bridge.control.note_on(72, 0.7);
        assert_eq!(
            bridge.control.capture_unison_chord().unwrap().intervals(),
            &[0, 3, 8]
        );
        bridge.control.note_off(67);
        assert_eq!(
            bridge.control.capture_unison_chord().unwrap().intervals(),
            &[0, 8]
        );
        bridge.control.all_notes_off();
        assert!(bridge.control.capture_unison_chord().is_none());
    }

    #[test]
    fn patch_load_queues_chord_memory_before_parameter_updates() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut patch = Patch::default();
        patch.unison_chord = ChordMemory::from_notes([60, 64, 67]);
        bridge.control.load_patch(&patch);
        let mut first = None;
        audio.control.drain(|message| {
            if first.is_none() {
                first = Some(message);
            }
        });
        match first {
            Some(ControlMessage::SetUnisonChord(chord)) => {
                assert_eq!(chord, patch.unison_chord)
            }
            _ => panic!("patch load must queue chord memory first"),
        }
    }

    #[test]
    fn note_event_waits_for_control_ring_capacity_instead_of_being_dropped() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        for value in 0..CONTROL_QUEUE_CAPACITY {
            bridge
                .control
                .set_param_audio_only(ParamId::FilterCutoff, value as f32);
        }

        let control = bridge.control.clone();
        let note_thread = std::thread::spawn(move || control.note_on(72, 1.0));
        let mut first = None;
        audio.control.drain(|message| {
            if first.is_none() {
                first = Some(message);
            }
        });
        note_thread.join().unwrap();

        let mut found_note = false;
        audio.control.drain(|message| {
            found_note |= matches!(message, ControlMessage::NoteOn { note: 72, .. });
        });
        assert!(found_note, "NoteOn must survive a saturated control ring");
    }

    #[test]
    fn local_patch_load_does_not_generate_midi_ui_updates() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        bridge.control.load_patch(&Patch::default());
        let mut updates = 0;
        bridge.view.drain_midi_ui_updates(|_| updates += 1);
        assert_eq!(updates, 0);
    }
}
