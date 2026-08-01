use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod cpal_input;
pub mod fake;
pub mod wav;

pub use fake::{FakeAudioInput, RenderEngine};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioError {
    #[error("audio device `{requested}` not found; available: {available}")]
    DeviceNotFound {
        requested: String,
        available: String,
    },
    #[error("audio configuration error: {0}")]
    Config(String),
    #[error("audio stream error: {0}")]
    Stream(String),
    #[error("audio ring overflow ({overflow_frames} frame(s) dropped)")]
    Overflow { overflow_frames: u64 },
    #[error("audio stream callback reported {callback_errors} error(s)")]
    Callback { callback_errors: u64 },
    #[error("audio underrun while waiting for {expected} frames (got {got})")]
    Underrun { expected: usize, got: usize },
    #[error("io error: {0}")]
    Io(String),
}

impl AudioError {
    pub fn from_health(health: &AudioHealth) -> Option<Self> {
        if health.callback_errors > 0 {
            Some(Self::Callback {
                callback_errors: health.callback_errors,
            })
        } else if health.overflow_frames > 0 {
            Some(Self::Overflow {
                overflow_frames: health.overflow_frames,
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub input_channel: u32,
    pub native_float32: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AudioHealth {
    pub overflow_frames: u64,
    pub callback_errors: u64,
}

impl AudioHealth {
    pub fn is_clean(&self) -> bool {
        self.overflow_frames == 0 && self.callback_errors == 0
    }
}

#[derive(Clone, Default)]
pub struct AudioCounters {
    overflow_frames: Arc<AtomicU64>,
    callback_errors: Arc<AtomicU64>,
}

impl AudioCounters {
    pub fn snapshot(&self) -> AudioHealth {
        AudioHealth {
            overflow_frames: self.overflow_frames.load(Ordering::Acquire),
            callback_errors: self.callback_errors.load(Ordering::Acquire),
        }
    }

    pub fn reset(&self) {
        self.overflow_frames.store(0, Ordering::Release);
        self.callback_errors.store(0, Ordering::Release);
    }

    pub fn record_overflow(&self, frames: u64) {
        self.overflow_frames.fetch_add(frames, Ordering::AcqRel);
    }

    pub fn record_error(&self) {
        self.callback_errors.fetch_add(1, Ordering::AcqRel);
    }

    pub fn overflow_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.overflow_frames)
    }

    pub fn error_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.callback_errors)
    }
}

pub trait AudioInput {
    fn format(&self) -> AudioFormat;
    fn drain_frames(&mut self, frame_count: usize, dest: &mut Vec<f32>) -> Result<(), AudioError>;
    fn health(&self) -> AudioHealth;
    fn reset_health(&mut self);
}

#[derive(Clone)]
pub struct StopFlag {
    flag: Arc<AtomicBool>,
}

impl StopFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn from_arc(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }

    pub fn handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    pub fn request_stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Default for StopFlag {
    fn default() -> Self {
        Self::new()
    }
}
