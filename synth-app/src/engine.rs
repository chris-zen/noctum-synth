//! Lock-free bridge between the UI/MIDI threads and the audio thread.

use parking_lot::{Mutex, RwLock};
use rtrb::{PushError, RingBuffer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

#[cfg(feature = "experimental-oscillators")]
use synth_core::ExperimentalOscillatorModel;
use synth_core::{
    ChordMemory, ControlMessage, LayerId, LayerMode, LayerPlaybackStatus, LayerTarget,
    ModDestination, ModRoute, ModSource, ModulationParam, ParamId, Patch, SequenceClear,
    SequenceUpdate, SequencerFeedback, SequencerRecordCommand,
    dsp::{FilterOversampling, FilterType},
    midi::{
        clock::{MidiClockMode, MidiClockStatus, MidiRealtimeEvent},
        program::ProgramData,
    },
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
#[derive(Debug, Clone)]
pub enum MidiUiUpdate {
    Param {
        target: LayerTarget,
        param: ParamId,
        value: f32,
    },
    MasterVolume(f32),
    Modulation {
        target: LayerTarget,
        route: ModRoute,
        parameter: ModulationParam,
    },
    Sequence {
        target: LayerTarget,
        update: SequenceUpdate,
    },
    LayerMode(LayerMode),
    SplitPoint(u8),
    EditLayer(LayerId),
    Program(Box<Patch>),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequencerPlaybackStatus {
    pub running: bool,
    pub active_step: Option<u8>,
}

pub enum FeedbackMessage {
    Audio(AudioBlock),
    Metrics(AudioMetrics),
    MidiClock(MidiClockStatus),
    LayerPlayback(LayerPlaybackStatus),
    Sequencer(SequencerFeedback),
    SequencerPlayback {
        layer: LayerId,
        status: SequencerPlaybackStatus,
    },
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

    pub fn push_layer_playback(&mut self, status: LayerPlaybackStatus) -> bool {
        self.sender
            .push(FeedbackMessage::LayerPlayback(status))
            .is_ok()
    }

    pub fn push_sequencer_feedback(&mut self, feedback: SequencerFeedback) -> bool {
        self.sender
            .push(FeedbackMessage::Sequencer(feedback))
            .is_ok()
    }

    pub fn push_sequencer_playback(
        &mut self,
        layer: LayerId,
        status: SequencerPlaybackStatus,
    ) -> bool {
        self.sender
            .push(FeedbackMessage::SequencerPlayback { layer, status })
            .is_ok()
    }
}

/// Read-only view of engine state for the UI (active voices, captured audio).
#[derive(Clone)]
pub struct SynthEngineView {
    active_voices: Arc<AtomicUsize>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
    metrics: Arc<RwLock<Option<AudioMetrics>>>,
    midi_clock: Arc<RwLock<Option<MidiClockStatus>>>,
    layer_playback: Arc<RwLock<LayerPlaybackStatus>>,
    sequencer_feedback: Arc<Mutex<VecDeque<SequencerFeedback>>>,
    sequencer_playback: Arc<RwLock<[SequencerPlaybackStatus; 2]>>,
    feedback_receiver: Arc<Mutex<rtrb::Consumer<FeedbackMessage>>>,
    midi_ui_receiver: Arc<Mutex<rtrb::Consumer<MidiUiUpdate>>>,
    midi_program_receiver: Arc<Mutex<rtrb::Consumer<Box<ProgramData>>>>,
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
                FeedbackMessage::LayerPlayback(status) => {
                    *self.layer_playback.write() = status;
                }
                FeedbackMessage::Sequencer(feedback) => {
                    let mut queue = self.sequencer_feedback.lock();
                    if queue.len() >= 64 {
                        queue.pop_front();
                    }
                    queue.push_back(feedback);
                }
                FeedbackMessage::SequencerPlayback { layer, status } => {
                    self.sequencer_playback.write()[match layer {
                        LayerId::A => 0,
                        LayerId::B => 1,
                    }] = status;
                }
            }
        }
    }

    pub fn drain_audio_blocks(&self) -> VecDeque<AudioBlock> {
        std::mem::take(&mut *self.audio_blocks.write())
    }

    pub fn layer_playback_status(&self) -> LayerPlaybackStatus {
        *self.layer_playback.read()
    }

    pub fn drain_midi_ui_updates(&self, mut handler: impl FnMut(MidiUiUpdate)) {
        let mut receiver = self.midi_ui_receiver.lock();
        while let Ok(update) = receiver.pop() {
            handler(update);
        }
    }

    pub fn drain_sequencer_feedback(&self, mut handler: impl FnMut(SequencerFeedback)) {
        let mut queue = self.sequencer_feedback.lock();
        while let Some(feedback) = queue.pop_front() {
            handler(feedback);
        }
    }

    pub fn sequencer_playback_status(&self, layer: LayerId) -> SequencerPlaybackStatus {
        self.sequencer_playback.read()[match layer {
            LayerId::A => 0,
            LayerId::B => 1,
        }]
    }

    pub fn drain_midi_program_imports(&self, mut handler: impl FnMut(ProgramData)) {
        let mut receiver = self.midi_program_receiver.lock();
        while let Ok(program) = receiver.pop() {
            handler(*program);
        }
    }
}

pub enum AudioCommand {
    Control(ControlMessage),
    Program(Patch),
}

type ControlProducer = rtrb::Producer<AudioCommand>;
type ControlConsumer = rtrb::Consumer<AudioCommand>;

/// Thread-safe handle for sending [`ControlMessage`] values to the audio thread.
#[derive(Clone)]
pub struct SynthEngineControl {
    sender: Arc<Mutex<ControlProducer>>,
    midi_ui_sender: Arc<Mutex<rtrb::Producer<MidiUiUpdate>>>,
    midi_program_sender: Arc<Mutex<rtrb::Producer<Box<ProgramData>>>>,
    midi_output: MidiOutputHandle,
    midi_clock_status: Arc<RwLock<Option<MidiClockStatus>>>,
    input_enabled: Arc<AtomicBool>,
    analysis_enabled: Arc<AtomicBool>,
    output_muted: Arc<AtomicBool>,
    held_notes: Arc<Mutex<[bool; 128]>>,
    edit_layer: Arc<AtomicU8>,
}

impl SynthEngineControl {
    pub fn edit_layer(&self) -> LayerId {
        if self.edit_layer.load(Ordering::Relaxed) == 0 {
            LayerId::A
        } else {
            LayerId::B
        }
    }

    pub fn set_edit_layer(&self, layer: LayerId) {
        self.set_edit_layer_inner(layer);
        self.midi_output.send_edit_layer(layer);
    }

    pub fn set_midi_edit_layer(&self, layer: LayerId) {
        self.set_edit_layer_inner(layer);
        self.midi_output.cache_edit_layer(layer);
        self.send_midi_ui(MidiUiUpdate::EditLayer(layer));
    }

    fn set_edit_layer_inner(&self, layer: LayerId) {
        self.edit_layer.store(
            match layer {
                LayerId::A => 0,
                LayerId::B => 1,
            },
            Ordering::Relaxed,
        );
        self.send(ControlMessage::SetEditLayer(layer));
    }

    pub fn set_layer_mode(&self, mode: LayerMode) {
        self.send(ControlMessage::SetLayerMode(mode));
        self.midi_output.send_layer_mode(mode);
    }

    pub fn set_midi_layer_mode(&self, mode: LayerMode) {
        self.send(ControlMessage::SetLayerMode(mode));
        self.midi_output.cache_layer_mode(mode);
        self.send_midi_ui(MidiUiUpdate::LayerMode(mode));
    }

    pub fn set_split_point(&self, split_point: u8) {
        self.send(ControlMessage::SetSplitPoint(split_point));
        self.midi_output.send_split_point(split_point);
    }

    pub fn set_midi_split_point(&self, split_point: u8) {
        self.send(ControlMessage::SetSplitPoint(split_point));
        self.midi_output.cache_split_point(split_point);
        self.send_midi_ui(MidiUiUpdate::SplitPoint(split_point));
    }

    pub fn set_modulation(
        &self,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        let layer = self.edit_layer();
        self.send(ControlMessage::SetModulation {
            target: LayerTarget::Explicit(layer),
            route,
            enabled,
            source,
            destination,
            amount,
        });
        self.midi_output
            .send_modulation(layer, route, enabled, source, destination, amount);
    }

    pub fn set_param(&self, param: ParamId, value: f32) {
        let layer = self.edit_layer();
        self.send(ControlMessage::SetParam {
            target: LayerTarget::Explicit(layer),
            param,
            value,
        });
        if param == ParamId::Bpm {
            self.midi_output.set_master_bpm(value);
        }
        self.midi_output.send_param(layer, param, value);
    }

    #[cfg(test)]
    pub fn set_param_audio_only(&self, param: ParamId, value: f32) {
        self.set_target_param_audio_only(LayerTarget::Edit, param, value);
    }

    #[cfg(test)]
    pub fn set_target_param_audio_only(&self, target: LayerTarget, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam {
            target,
            param,
            value,
        });
    }

    /// Sends a MIDI-originated parameter change to audio and mirrors it to UI.
    pub fn set_midi_param(&self, target: LayerTarget, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam {
            target,
            param,
            value,
        });
        self.midi_output.cache_param(target, param, value);
        self.send_midi_ui(MidiUiUpdate::Param {
            target,
            param,
            value,
        });
    }

    /// Sends one MIDI-originated modulation field to audio and UI.
    pub fn set_midi_modulation_param(
        &self,
        target: LayerTarget,
        route: ModRoute,
        parameter: ModulationParam,
    ) {
        self.send(ControlMessage::SetModulationParam {
            target,
            route,
            parameter,
        });
        self.midi_output
            .cache_modulation_param(target, route, parameter);
        self.send_midi_ui(MidiUiUpdate::Modulation {
            target,
            route,
            parameter,
        });
    }

    /// Sends one MIDI-originated sequencer field to the cached patch and UI.
    pub fn set_midi_sequence(&self, target: LayerTarget, update: SequenceUpdate) {
        self.send(ControlMessage::SetSequence { target, update });
        self.send_midi_ui(MidiUiUpdate::Sequence { target, update });
    }

    pub fn set_sequence(&self, target: LayerTarget, update: SequenceUpdate) {
        let layer = match target {
            LayerTarget::Edit => self.edit_layer(),
            LayerTarget::Explicit(layer) => layer,
        };
        self.send(ControlMessage::SetSequence {
            target: LayerTarget::Explicit(layer),
            update,
        });
        self.midi_output.send_sequence(layer, update);
    }

    pub fn clear_sequence(&self, target: LayerTarget, section: SequenceClear) {
        let layer = match target {
            LayerTarget::Edit => self.edit_layer(),
            LayerTarget::Explicit(layer) => layer,
        };
        self.send(ControlMessage::ClearSequence {
            target: LayerTarget::Explicit(layer),
            section,
        });
        self.midi_output.clear_sequence(layer, section);
    }

    pub fn set_sequencer_running(&self, target: LayerTarget, running: bool) {
        let layer = match target {
            LayerTarget::Edit => self.edit_layer(),
            LayerTarget::Explicit(layer) => layer,
        };
        self.send(ControlMessage::SetSequencerRunning {
            target: LayerTarget::Explicit(layer),
            running,
        });
        self.midi_output.send_sequencer_running(layer, running);
    }

    /// Starts or stops the sequencers that are audible in the current patch
    /// topology. Dual-layer presets may keep their musical sequence on either
    /// layer, so Stack and Split transport addresses both layers.
    pub fn set_patch_sequencers_running(
        &self,
        mode: LayerMode,
        edit_layer: LayerId,
        running: bool,
    ) {
        match mode {
            LayerMode::Normal => {
                self.set_sequencer_running(LayerTarget::Explicit(edit_layer), running)
            }
            LayerMode::Stack | LayerMode::Split => {
                for layer in [LayerId::A, LayerId::B] {
                    self.set_sequencer_running(LayerTarget::Explicit(layer), running);
                }
            }
        }
    }

    pub fn set_midi_sequencer_running(&self, target: LayerTarget, running: bool) {
        self.send(ControlMessage::SetSequencerRunning { target, running });
    }

    pub fn sequencer_record_command(&self, target: LayerTarget, command: SequencerRecordCommand) {
        self.send(ControlMessage::SequencerRecord { target, command });
    }

    pub fn set_sequencer_recording(&self, target: LayerTarget, recording: bool) {
        let layer = match target {
            LayerTarget::Edit => self.edit_layer(),
            LayerTarget::Explicit(layer) => layer,
        };
        self.sequencer_record_command(
            LayerTarget::Explicit(layer),
            if recording {
                SequencerRecordCommand::Start
            } else {
                SequencerRecordCommand::Stop
            },
        );
        self.midi_output.send_sequencer_recording(layer, recording);
    }

    pub fn set_midi_sequencer_recording(&self, target: LayerTarget, recording: bool) {
        self.sequencer_record_command(
            target,
            if recording {
                SequencerRecordCommand::Start
            } else {
                SequencerRecordCommand::Stop
            },
        );
    }

    pub fn set_filter_oversampling(&self, oversampling: FilterOversampling) {
        self.send(ControlMessage::SetFilterOversampling(oversampling));
    }

    pub fn set_filter_type(&self, filter_type: FilterType) {
        self.send(ControlMessage::SetFilterType(filter_type));
    }

    #[cfg(feature = "experimental-oscillators")]
    pub fn set_experimental_oscillator_model(&self, model: ExperimentalOscillatorModel) {
        self.all_notes_off();
        self.send(ControlMessage::SetExperimentalOscillatorModel(model));
    }

    pub fn set_midi_clock_mode(&self, mode: MidiClockMode) {
        self.send(ControlMessage::SetMidiClockMode(mode));
        self.midi_output.set_clock_mode(mode);
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.send(ControlMessage::SetMasterVolume(volume));
        self.midi_output.send_master_volume(volume);
    }

    pub fn set_master_volume_audio_only(&self, volume: f32) {
        self.send(ControlMessage::SetMasterVolume(volume));
    }

    pub fn set_midi_master_volume(&self, volume: f32) {
        if self.output_muted.load(Ordering::Relaxed) {
            return;
        }
        self.send(ControlMessage::SetMasterVolume(volume));
        self.send_midi_ui(MidiUiUpdate::MasterVolume(volume));
    }

    #[cfg(test)]
    pub fn output_muted(&self) -> bool {
        self.output_muted.load(Ordering::Relaxed)
    }

    pub fn set_output_muted(&self, muted: bool) {
        self.output_muted.store(muted, Ordering::Relaxed);
        if muted {
            self.set_master_volume_audio_only(0.0);
        }
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
        self.send(ControlMessage::SetUnisonChord {
            target: LayerTarget::Explicit(self.edit_layer()),
            chord,
        });
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

    /// Enables audio-block capture only while the analysis viewport needs it.
    pub fn set_analysis_enabled(&self, enabled: bool) {
        self.analysis_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn load_program(&self, patch: &Patch) {
        self.edit_layer.store(0, Ordering::Relaxed);
        self.midi_output.set_master_bpm(patch.layer_a.bpm);
        self.send_program(patch.clone());
        let _ = self.midi_output.send_patch(patch);
    }

    pub fn load_program_respecting_mute(&self, patch: &Patch, muted: bool) {
        self.load_program(patch);
        if muted {
            self.mute_all_layers();
        }
    }

    /// Reloads a program after an audio-session rebind while preserving the
    /// layer currently selected in the desktop UI.
    pub fn reload_program_preserving_edit(&self, patch: &Patch, edit_layer: LayerId, muted: bool) {
        self.edit_layer.store(
            match edit_layer {
                LayerId::A => 0,
                LayerId::B => 1,
            },
            Ordering::Relaxed,
        );
        self.midi_output.set_master_bpm(patch.layer(edit_layer).bpm);
        self.send_program(patch.clone());
        self.send(ControlMessage::SetEditLayer(edit_layer));
        let _ = self.midi_output.send_patch(patch);
        self.midi_output.send_edit_layer(edit_layer);
        if muted {
            self.mute_all_layers();
        }
    }

    /// Sends a complete program to the selected MIDI output without changing local state.
    pub fn send_midi_program(&self, program: &Patch) -> bool {
        self.midi_output.send_patch(program)
    }

    /// Applies a MIDI edit-buffer dump without echoing it to MIDI output.
    /// Stored-program dumps use the background import queue instead.
    pub fn load_midi_edit_buffer(&self, patch: &Patch) {
        self.edit_layer.store(0, Ordering::Relaxed);
        self.midi_output.set_master_bpm(patch.layer_a.bpm);
        self.midi_output.cache_patch(patch);
        self.send_program(patch.clone());
        self.send_midi_ui(MidiUiUpdate::Program(Box::new(patch.clone())));
    }

    pub fn mute_all_layers(&self) {
        self.set_output_muted(true);
    }

    pub fn queue_midi_program(&self, program: ProgramData) -> bool {
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
        let reliable = matches!(
            &message,
            ControlMessage::NoteOn { .. }
                | ControlMessage::NoteOff { .. }
                | ControlMessage::AllNotesOff
                | ControlMessage::SustainPedal { .. }
                | ControlMessage::SetLayerMode(_)
                | ControlMessage::SetSplitPoint(_)
                | ControlMessage::SetEditLayer(_)
                | ControlMessage::SetSequence { .. }
                | ControlMessage::SetSequencerRunning { .. }
                | ControlMessage::SequencerRecord { .. }
        );
        let mut pending = AudioCommand::Control(message);
        loop {
            let result = {
                let mut sender = self.sender.lock();
                sender.push(pending)
            };
            match result {
                Ok(()) => return,
                Err(PushError::Full(command)) if reliable => {
                    pending = command;
                    // Drop the producer lock before yielding so audio-session
                    // rebind can replace a queue whose consumer has stopped.
                    std::thread::yield_now();
                }
                Err(PushError::Full(_)) => return,
            }
        }
    }

    fn send_program(&self, patch: Patch) {
        let mut pending = AudioCommand::Program(patch);
        loop {
            let result = {
                let mut sender = self.sender.lock();
                sender.push(pending)
            };
            match result {
                Ok(()) => return,
                Err(PushError::Full(command)) => {
                    pending = command;
                    std::thread::yield_now();
                }
            }
        }
    }

    fn send_midi_ui(&self, update: MidiUiUpdate) {
        let reliable = matches!(
            &update,
            MidiUiUpdate::LayerMode(_)
                | MidiUiUpdate::SplitPoint(_)
                | MidiUiUpdate::EditLayer(_)
                | MidiUiUpdate::Program(_)
        );
        let mut pending = update;
        loop {
            let result = self.midi_ui_sender.lock().push(pending);
            match result {
                Ok(()) => return,
                Err(PushError::Full(update)) if reliable => {
                    pending = update;
                    std::thread::yield_now();
                }
                Err(PushError::Full(_)) => return,
            }
        }
    }
}

pub struct SynthEngineControlReceiver(pub ControlConsumer);

impl SynthEngineControlReceiver {
    pub fn drain<F: FnMut(AudioCommand)>(&mut self, mut handler: F) {
        while let Ok(command) = self.0.pop() {
            handler(command);
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
    /// Shared flag which keeps analysis capture off the audio path while hidden.
    pub analysis_enabled: Arc<AtomicBool>,
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
    let layer_playback = Arc::new(RwLock::new(LayerPlaybackStatus {
        mode: LayerMode::Normal,
        edit_layer: LayerId::A,
        rendered_mask: 0b01,
        degraded: false,
    }));
    let sequencer_feedback = Arc::new(Mutex::new(VecDeque::new()));
    let sequencer_playback = Arc::new(RwLock::new([SequencerPlaybackStatus::default(); 2]));
    let input_enabled = Arc::new(AtomicBool::new(true));
    let analysis_enabled = Arc::new(AtomicBool::new(false));
    let edit_layer = Arc::new(AtomicU8::new(0));
    let feedback_receiver = Arc::new(Mutex::new(feedback_receiver));

    let bridge = SynthEngineBridge {
        control: SynthEngineControl {
            sender: Arc::new(Mutex::new(control_sender)),
            midi_ui_sender: Arc::new(Mutex::new(midi_ui_sender)),
            midi_program_sender: Arc::new(Mutex::new(midi_program_sender)),
            midi_output: MidiOutputHandle::default(),
            midi_clock_status: midi_clock.clone(),
            input_enabled: input_enabled.clone(),
            analysis_enabled: analysis_enabled.clone(),
            output_muted: Arc::new(AtomicBool::new(false)),
            held_notes: Arc::new(Mutex::new([false; 128])),
            edit_layer,
        },
        view: SynthEngineView {
            active_voices: active_voices.clone(),
            audio_blocks,
            metrics,
            midi_clock,
            layer_playback,
            sequencer_feedback,
            sequencer_playback,
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
        analysis_enabled,
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
        analysis_enabled: bridge.control.analysis_enabled.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::LayerPatch;

    #[test]
    fn analysis_capture_flag_survives_audio_channel_rebind() {
        let (audio, bridge) = create_synth_engine_bridge(16);
        assert!(!audio.analysis_enabled.load(Ordering::Relaxed));
        bridge.control.set_analysis_enabled(true);
        assert!(audio.analysis_enabled.load(Ordering::Relaxed));

        let rebound = rebind_audio_channels(&bridge);
        assert!(rebound.analysis_enabled.load(Ordering::Relaxed));
    }

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
    fn program_load_queues_one_complete_program_command() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let mut patch = LayerPatch::default();
        patch.unison_chord = ChordMemory::from_notes([60, 64, 67]);
        let program = Patch {
            layer_a: patch.clone(),
            ..Patch::default()
        };
        bridge.control.load_program(&program);
        let mut first = None;
        audio.control.drain(|message| {
            if first.is_none() {
                first = Some(message);
            }
        });
        match first {
            Some(AudioCommand::Program(queued)) => {
                assert_eq!(queued.layer_a.unison_chord, patch.unison_chord)
            }
            _ => panic!("program load must queue one complete program"),
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
        let mut found_note = false;
        audio.control.drain(|message| {
            found_note |= matches!(
                message,
                AudioCommand::Control(ControlMessage::NoteOn { note: 72, .. })
            );
        });
        note_thread.join().unwrap();

        audio.control.drain(|message| {
            found_note |= matches!(
                message,
                AudioCommand::Control(ControlMessage::NoteOn { note: 72, .. })
            );
        });
        assert!(found_note, "NoteOn must survive a saturated control ring");
    }

    #[test]
    fn topology_event_waits_for_control_ring_capacity_instead_of_being_dropped() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        for value in 0..CONTROL_QUEUE_CAPACITY {
            bridge
                .control
                .set_param_audio_only(ParamId::FilterCutoff, value as f32);
        }

        let control = bridge.control.clone();
        let topology_thread = std::thread::spawn(move || control.set_layer_mode(LayerMode::Split));
        let mut found_mode = false;
        audio.control.drain(|message| {
            found_mode |= matches!(
                message,
                AudioCommand::Control(ControlMessage::SetLayerMode(LayerMode::Split))
            );
        });
        topology_thread.join().unwrap();

        audio.control.drain(|message| {
            found_mode |= matches!(
                message,
                AudioCommand::Control(ControlMessage::SetLayerMode(LayerMode::Split))
            );
        });
        assert!(found_mode, "topology state must survive a saturated ring");
    }

    #[test]
    fn midi_master_volume_is_ignored_while_output_is_muted() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        bridge.control.mute_all_layers();
        assert!(bridge.control.output_muted());
        bridge.control.set_midi_master_volume(0.25);

        let mut saw_master = false;
        bridge.view.drain_midi_ui_updates(|update| {
            saw_master |= matches!(update, MidiUiUpdate::MasterVolume(_));
        });
        assert!(!saw_master);

        let mut saw_engine_master = false;
        audio.control.drain(|command| {
            saw_engine_master |= matches!(
                command,
                AudioCommand::Control(ControlMessage::SetMasterVolume(volume)) if volume > 0.0
            );
        });
        assert!(
            !saw_engine_master,
            "muted MIDI master must not unmute the engine"
        );

        bridge.control.set_output_muted(false);
        assert!(!bridge.control.output_muted());
        bridge.control.set_midi_master_volume(0.25);
        let mut restored = None;
        bridge.view.drain_midi_ui_updates(|update| {
            if let MidiUiUpdate::MasterVolume(volume) = update {
                restored = Some(volume);
            }
        });
        assert_eq!(restored, Some(0.25));
    }

    #[test]
    fn critical_midi_ui_update_waits_for_capacity_instead_of_being_dropped() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        for value in 0..MIDI_UI_QUEUE_CAPACITY {
            bridge.control.send_midi_ui(MidiUiUpdate::Param {
                target: LayerTarget::Edit,
                param: ParamId::FilterCutoff,
                value: value as f32,
            });
        }

        let control = bridge.control.clone();
        let update_thread = std::thread::spawn(move || control.set_midi_edit_layer(LayerId::B));
        let mut found_edit = false;
        bridge.view.drain_midi_ui_updates(|update| {
            found_edit |= matches!(update, MidiUiUpdate::EditLayer(LayerId::B));
        });
        update_thread.join().unwrap();

        bridge.view.drain_midi_ui_updates(|update| {
            found_edit |= matches!(update, MidiUiUpdate::EditLayer(LayerId::B));
        });
        assert!(
            found_edit,
            "critical UI state must survive a saturated ring"
        );
    }

    #[test]
    fn audio_rebind_reload_preserves_selected_edit_layer_in_command_order() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let patch = Patch {
            mode: LayerMode::Stack,
            ..Patch::default()
        };

        bridge
            .control
            .reload_program_preserving_edit(&patch, LayerId::B, false);

        assert_eq!(bridge.control.edit_layer(), LayerId::B);
        let mut commands = Vec::new();
        audio.control.drain(|command| commands.push(command));
        assert!(matches!(commands.first(), Some(AudioCommand::Program(_))));
        assert!(matches!(
            commands.get(1),
            Some(AudioCommand::Control(ControlMessage::SetEditLayer(
                LayerId::B
            )))
        ));
    }

    #[test]
    fn local_patch_load_does_not_generate_midi_ui_updates() {
        let (_audio, bridge) = create_synth_engine_bridge(16);
        bridge.control.load_program(&Patch::default());
        let mut updates = 0;
        bridge.view.drain_midi_ui_updates(|_| updates += 1);
        assert_eq!(updates, 0);
    }

    #[test]
    fn sequencer_record_feedback_crosses_the_bounded_audio_ui_bridge() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let feedback = SequencerFeedback::RecordOverflow {
            layer: LayerId::B,
            cursor: 63,
        };
        assert!(audio.feedback.push_sequencer_feedback(feedback));
        bridge.view.drain_feedback();
        let mut actual = None;
        bridge
            .view
            .drain_sequencer_feedback(|event| actual = Some(event));
        assert_eq!(actual, Some(feedback));
    }

    #[test]
    fn bulk_sequence_clear_uses_one_bounded_audio_command() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        bridge
            .control
            .clear_sequence(LayerTarget::Explicit(LayerId::B), SequenceClear::Polyphonic);
        let mut commands = Vec::new();
        audio.control.drain(|command| commands.push(command));
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            AudioCommand::Control(ControlMessage::ClearSequence {
                target: LayerTarget::Explicit(LayerId::B),
                section: SequenceClear::Polyphonic,
            })
        ));
    }

    #[test]
    fn semantic_poly_event_is_one_atomic_audio_command() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        let value = synth_core::PolyLaneStep {
            note: synth_core::PolyNote::Tie,
            velocity: synth_core::PolyVelocity::Velocity(127),
        };
        bridge.control.set_sequence(
            LayerTarget::Explicit(LayerId::A),
            SequenceUpdate::PolyLaneStep {
                step: 7,
                lane: 2,
                value,
            },
        );

        let mut commands = Vec::new();
        audio.control.drain(|command| commands.push(command));
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            AudioCommand::Control(ControlMessage::SetSequence {
                target: LayerTarget::Explicit(LayerId::A),
                update: SequenceUpdate::PolyLaneStep {
                    step: 7,
                    lane: 2,
                    value: actual,
                },
            }) if actual == value
        ));
    }

    #[test]
    fn dual_layer_patch_transport_addresses_both_sequencers() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        bridge
            .control
            .set_patch_sequencers_running(LayerMode::Stack, LayerId::A, true);

        let mut targets = Vec::new();
        audio.control.drain(|command| {
            if let AudioCommand::Control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Explicit(layer),
                running: true,
            }) = command
            {
                targets.push(layer);
            }
        });
        assert_eq!(targets, [LayerId::A, LayerId::B]);

        bridge
            .control
            .set_patch_sequencers_running(LayerMode::Normal, LayerId::B, true);
        targets.clear();
        audio.control.drain(|command| {
            if let AudioCommand::Control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Explicit(layer),
                running: true,
            }) = command
            {
                targets.push(layer);
            }
        });
        assert_eq!(targets, [LayerId::B]);
    }

    #[test]
    fn ui_layer_and_transport_controls_enqueue_audio_commands() {
        let (mut audio, bridge) = create_synth_engine_bridge(16);
        bridge.control.set_layer_mode(LayerMode::Split);
        bridge.control.set_split_point(72);
        bridge.control.set_edit_layer(LayerId::B);
        bridge
            .control
            .set_sequencer_running(LayerTarget::Explicit(LayerId::B), true);
        bridge
            .control
            .set_sequencer_recording(LayerTarget::Explicit(LayerId::B), true);

        let mut saw_mode = false;
        let mut saw_split = false;
        let mut saw_edit = false;
        let mut saw_running = false;
        let mut saw_recording = false;
        audio.control.drain(|command| match command {
            AudioCommand::Control(ControlMessage::SetLayerMode(LayerMode::Split)) => {
                saw_mode = true
            }
            AudioCommand::Control(ControlMessage::SetSplitPoint(72)) => saw_split = true,
            AudioCommand::Control(ControlMessage::SetEditLayer(LayerId::B)) => saw_edit = true,
            AudioCommand::Control(ControlMessage::SetSequencerRunning {
                target: LayerTarget::Explicit(LayerId::B),
                running: true,
            }) => saw_running = true,
            AudioCommand::Control(ControlMessage::SequencerRecord {
                target: LayerTarget::Explicit(LayerId::B),
                command: SequencerRecordCommand::Start,
            }) => saw_recording = true,
            _ => {}
        });
        assert!(saw_mode && saw_split && saw_edit && saw_running && saw_recording);
    }
}
