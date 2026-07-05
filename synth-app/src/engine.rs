//! Lock-free bridge between the UI/MIDI threads and the audio thread.

use parking_lot::{Mutex, RwLock};
use rtrb::RingBuffer;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use synth_core::{ControlMessage, ParamId};

pub const MAX_AUDIO_BUF: usize = 1024;

/// Stereo audio block returned from the engine for spectrum analysis.
pub struct AudioBlock {
    pub left: [f32; MAX_AUDIO_BUF],
    pub right: [f32; MAX_AUDIO_BUF],
    pub len: u16,
}

impl Default for AudioBlock {
    fn default() -> Self {
        Self {
            left: [0.0; MAX_AUDIO_BUF],
            right: [0.0; MAX_AUDIO_BUF],
            len: 0,
        }
    }
}

const MAX_PENDING_BLOCKS: usize = 32;

pub enum FeedbackMessage {
    Audio(AudioBlock),
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
}

/// Read-only view of engine state for the UI (active voices, captured audio).
#[derive(Clone)]
pub struct SynthEngineView {
    active_voices: Arc<AtomicUsize>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
    total_voices: usize,
}

impl SynthEngineView {
    pub fn active_voices(&self) -> usize {
        self.active_voices.load(Ordering::Relaxed)
    }

    pub fn total_voices(&self) -> usize {
        self.total_voices
    }

    pub fn drain_audio_blocks(&self) -> VecDeque<AudioBlock> {
        std::mem::take(&mut *self.audio_blocks.write())
    }
}

type ControlProducer = rtrb::Producer<ControlMessage>;
type ControlConsumer = rtrb::Consumer<ControlMessage>;

/// Thread-safe handle for sending [`ControlMessage`] values to the audio thread.
#[derive(Clone)]
pub struct SynthEngineControl {
    sender: Arc<Mutex<ControlProducer>>,
}

impl SynthEngineControl {
    pub fn set_param(&self, param: ParamId, value: f32) {
        self.send(ControlMessage::SetParam(param, value));
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

    pub fn sustain_pedal(&self, pressed: bool) {
        self.send(ControlMessage::SustainPedal { pressed });
    }

    pub fn control_change(&self, controller: u8, value: f32) {
        self.send(ControlMessage::ControlChange { controller, value });
    }

    fn send(&self, message: ControlMessage) {
        let _ = self.sender.lock().push(message);
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
}

/// Creates the control ring buffer and spawns the UI feedback thread.
pub fn create_synth_engine_bridge(total_voices: usize) -> (SynthEngineAudio, SynthEngineBridge) {
    let (feedback_sender, feedback_receiver) = RingBuffer::new(64);
    let (control_sender, control_receiver) = RingBuffer::new(256);
    let active_voices = Arc::new(AtomicUsize::new(0));
    let audio_blocks = Arc::new(RwLock::new(VecDeque::new()));
    spawn_view_thread(feedback_receiver, audio_blocks.clone());

    let bridge = SynthEngineBridge {
        control: SynthEngineControl {
            sender: Arc::new(Mutex::new(control_sender)),
        },
        view: SynthEngineView {
            active_voices: active_voices.clone(),
            audio_blocks,
            total_voices,
        },
    };
    let audio = SynthEngineAudio {
        control: SynthEngineControlReceiver(control_receiver),
        feedback: SynthEngineFeedback {
            active_voices,
            sender: feedback_sender,
        },
    };
    (audio, bridge)
}

fn spawn_view_thread(
    mut receiver: rtrb::Consumer<FeedbackMessage>,
    audio_blocks: Arc<RwLock<VecDeque<AudioBlock>>>,
) {
    std::thread::spawn(move || {
        loop {
            while let Ok(FeedbackMessage::Audio(block)) = receiver.pop() {
                let mut blocks = audio_blocks.write();
                if blocks.len() >= MAX_PENDING_BLOCKS {
                    blocks.pop_front();
                }
                blocks.push_back(block);
            }
            if receiver.is_abandoned() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
}
