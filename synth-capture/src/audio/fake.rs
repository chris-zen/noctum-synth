use std::sync::{Arc, Mutex};

use crate::{
    audio::{AudioError, AudioFormat, AudioHealth, AudioInput},
    domain::OscillatorWaveform,
};

#[derive(Clone, Debug)]
pub struct RenderEngine {
    sample_rate: f32,
    phase: f64,
    gate: bool,
    frequency_hz: f32,
    waveform: Option<OscillatorWaveform>,
    pulse_width: f32,
    level: f32,
}

impl RenderEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            gate: false,
            frequency_hz: 440.0,
            waveform: None,
            pulse_width: 0.5,
            level: 0.2,
        }
    }

    pub fn reset_neutral(&mut self) {
        self.waveform = None;
        self.gate = false;
        self.pulse_width = 0.5;
        self.phase = 0.0;
        self.level = 0.2;
    }

    pub fn set_waveform(&mut self, waveform: OscillatorWaveform) {
        self.waveform = Some(waveform);
    }

    pub fn clear_waveform(&mut self) {
        self.waveform = None;
    }

    pub fn set_pulse_width(&mut self, width: f32) {
        self.pulse_width = width.clamp(0.01, 0.99);
    }

    pub fn note_on(&mut self, frequency_hz: f32) {
        self.frequency_hz = frequency_hz;
        self.gate = true;
    }

    pub fn note_off(&mut self) {
        self.gate = false;
    }

    pub fn render(&mut self, out: &mut [f32]) {
        for sample in out {
            if !self.gate || self.waveform.is_none() {
                *sample = 0.0;
            } else {
                let t = self.phase as f32;
                *sample = match self.waveform {
                    Some(OscillatorWaveform::Saw) => (2.0 * t - 1.0) * self.level,
                    Some(OscillatorWaveform::Triangle) => {
                        let tri = if t < 0.5 {
                            4.0 * t - 1.0
                        } else {
                            3.0 - 4.0 * t
                        };
                        tri * self.level
                    }
                    Some(OscillatorWaveform::Pulse) => {
                        if t < self.pulse_width {
                            self.level
                        } else {
                            -self.level
                        }
                    }
                    None => 0.0,
                };
                self.phase += f64::from(self.frequency_hz) / f64::from(self.sample_rate);
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
            }
        }
    }
}

pub struct FakeAudioInput {
    engine: Arc<Mutex<RenderEngine>>,
    format: AudioFormat,
    force_overflow: bool,
    overflow_seen: bool,
}

impl FakeAudioInput {
    pub fn new(engine: Arc<Mutex<RenderEngine>>) -> Self {
        Self {
            engine,
            format: default_fake_format(),
            force_overflow: false,
            overflow_seen: false,
        }
    }

    pub fn with_format(engine: Arc<Mutex<RenderEngine>>, format: AudioFormat) -> Self {
        Self {
            engine,
            format,
            force_overflow: false,
            overflow_seen: false,
        }
    }

    pub fn with_forced_overflow(engine: Arc<Mutex<RenderEngine>>) -> Self {
        Self {
            engine,
            format: default_fake_format(),
            force_overflow: true,
            overflow_seen: false,
        }
    }
}

impl AudioInput for FakeAudioInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn drain_frames(&mut self, frame_count: usize, dest: &mut Vec<f32>) -> Result<(), AudioError> {
        if self.force_overflow {
            self.overflow_seen = true;
            return Err(AudioError::Overflow { overflow_frames: 1 });
        }
        dest.resize(frame_count, 0.0);
        self.engine
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .render(dest);
        Ok(())
    }

    fn health(&self) -> AudioHealth {
        AudioHealth {
            overflow_frames: u64::from(self.overflow_seen),
            callback_errors: 0,
        }
    }

    fn reset_health(&mut self) {
        self.overflow_seen = false;
    }
}

fn default_fake_format() -> AudioFormat {
    AudioFormat {
        sample_rate_hz: 96_000,
        channels: 1,
        input_channel: 0,
        native_float32: true,
    }
}
