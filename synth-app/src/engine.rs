//! Lock-free bridge between the UI/MIDI threads and the audio thread.

use parking_lot::{Mutex, RwLock};
use rtrb::RingBuffer;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use synth_core::{
    ControlMessage, FilterOversampling, FilterType, ModDestination, ModRoute, ModSource,
    ModulationParam, ParamId, Patch, Rev2ProgramData,
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
}

/// Read-only view of engine state for the UI (active voices, captured audio).
#[derive(Clone)]
pub struct SynthEngineView {
    active_voices: Arc<AtomicUsize>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
    metrics: Arc<RwLock<Option<AudioMetrics>>>,
    midi_ui_receiver: Arc<Mutex<rtrb::Consumer<MidiUiUpdate>>>,
    midi_program_receiver: Arc<Mutex<rtrb::Consumer<Box<Rev2ProgramData>>>>,
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

    pub fn drain_audio_blocks(&self) -> VecDeque<AudioBlock> {
        std::mem::take(&mut *self.audio_blocks.write())
    }

    pub fn drain_midi_ui_updates(&self, mut handler: impl FnMut(MidiUiUpdate)) {
        let mut receiver = self.midi_ui_receiver.lock();
        while let Ok(update) = receiver.pop() {
            handler(update);
        }
    }

    pub fn drain_midi_program_imports(&self, mut handler: impl FnMut(Rev2ProgramData)) {
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
    midi_program_sender: Arc<Mutex<rtrb::Producer<Box<Rev2ProgramData>>>>,
    midi_output: MidiOutputHandle,
    input_enabled: Arc<AtomicBool>,
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
        self.midi_output.send_param(param, value);
    }

    /// Sends a MIDI-originated parameter change to audio and mirrors it to UI.
    pub fn set_midi_param(&self, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam(param, value));
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

    pub fn note_on(&self, note: u8, velocity: f32) {
        self.send(ControlMessage::NoteOn { note, velocity });
    }

    pub fn note_off(&self, note: u8) {
        self.send(ControlMessage::NoteOff { note });
    }

    pub fn all_notes_off(&self) {
        self.send(ControlMessage::AllNotesOff);
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

    pub fn midi_output_connected(&self) -> bool {
        self.midi_output.is_connected()
    }

    /// Enables or disables mixing of the audio input into the output at runtime.
    pub fn set_input_enabled(&self, enabled: bool) {
        self.input_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn load_patch(&self, patch: &Patch) {
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

    /// Sends a complete patch to the selected MIDI output without changing local state.
    pub fn send_midi_patch(&self, patch: &Patch) -> bool {
        self.midi_output.send_patch(patch)
    }

    /// Applies a complete MIDI-originated patch without echoing it to MIDI output.
    pub fn load_midi_patch(&self, patch: &Patch) {
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

    pub fn queue_midi_program(&self, program: Rev2ProgramData) -> bool {
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
        let _ = self.sender.lock().push(message);
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

/// Creates the control ring buffer and spawns the UI feedback thread.
pub fn create_synth_engine_bridge(total_voices: usize) -> (SynthEngineAudio, SynthEngineBridge) {
    let (feedback_sender, feedback_receiver) = RingBuffer::new(64);
    let (control_sender, control_receiver) = RingBuffer::new(256);
    let (midi_ui_sender, midi_ui_receiver) = RingBuffer::new(MIDI_UI_QUEUE_CAPACITY);
    let (midi_program_sender, midi_program_receiver) =
        RingBuffer::new(MIDI_PROGRAM_IMPORT_QUEUE_CAPACITY);
    let active_voices = Arc::new(AtomicUsize::new(0));
    let audio_blocks = Arc::new(RwLock::new(VecDeque::new()));
    let metrics = Arc::new(RwLock::new(None));
    let input_enabled = Arc::new(AtomicBool::new(true));
    spawn_view_thread(feedback_receiver, audio_blocks.clone(), metrics.clone());

    let bridge = SynthEngineBridge {
        control: SynthEngineControl {
            sender: Arc::new(Mutex::new(control_sender)),
            midi_ui_sender: Arc::new(Mutex::new(midi_ui_sender)),
            midi_program_sender: Arc::new(Mutex::new(midi_program_sender)),
            midi_output: MidiOutputHandle::default(),
            input_enabled: input_enabled.clone(),
        },
        view: SynthEngineView {
            active_voices: active_voices.clone(),
            audio_blocks,
            metrics,
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

fn spawn_view_thread(
    mut receiver: rtrb::Consumer<FeedbackMessage>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
    metrics: Arc<RwLock<Option<AudioMetrics>>>,
) {
    std::thread::spawn(move || {
        loop {
            while let Ok(message) = receiver.pop() {
                match message {
                    FeedbackMessage::Audio(block) => {
                        let mut blocks = audio_blocks.write();
                        if blocks.len() >= MAX_PENDING_BLOCKS {
                            blocks.pop_front();
                        }
                        blocks.push_back(block);
                    }
                    FeedbackMessage::Metrics(m) => {
                        *metrics.write() = Some(m);
                    }
                }
            }
            if receiver.is_abandoned() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
}
