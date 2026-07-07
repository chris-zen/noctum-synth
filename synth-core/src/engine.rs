//! Top-level synthesis engine and audio render entry point.

use crate::voices::Voices;
use crate::{ActiveNotes, ControlMessage, FilterOversampling, ParamId, VOICE_PACKS};

/// Owns all voices and renders stereo audio from [`ControlMessage`] input.
///
/// Construct with [`SynthEngine::new`], feed control messages from the host
/// thread, then call [`SynthEngine::process`] or
/// [`SynthEngine::process_interleaved`] on the audio thread.
pub struct SynthEngine<const PACKS: usize = VOICE_PACKS> {
    voices: Voices<PACKS>,
    master_volume: f32,
}

impl<const PACKS: usize> SynthEngine<PACKS> {
    /// Creates an engine at `sample_rate` with default patch settings.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: Voices::<PACKS>::new(sample_rate),
            master_volume: 1.0,
        }
    }

    /// Applies a single control or performance message.
    pub fn handle_control(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::SetParam(ParamId::MasterVolume, value) => {
                self.master_volume = value.clamp(0.0, 1.0);
            }
            ControlMessage::SetFilterOversampling(oversampling) => {
                self.set_filter_oversampling(oversampling);
            }
            message => self.voices.handle_control(message),
        }
    }

    pub fn set_param(&mut self, param: ParamId, value: f32) {
        self.handle_control(ControlMessage::SetParam(param, value));
    }

    /// Applies the nonlinear filter oversampling policy to all voices.
    pub fn set_filter_oversampling(&mut self, oversampling: FilterOversampling) {
        self.voices.set_filter_oversampling(oversampling);
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.handle_control(ControlMessage::NoteOn { note, velocity });
    }

    pub fn note_off(&mut self, note: u8) {
        self.handle_control(ControlMessage::NoteOff { note });
    }

    pub fn all_notes_off(&mut self) {
        self.handle_control(ControlMessage::AllNotesOff);
    }

    pub fn pitch_bend(&mut self, value: f32) {
        self.handle_control(ControlMessage::PitchBend { value });
    }

    pub fn mod_wheel(&mut self, value: f32) {
        self.handle_control(ControlMessage::ModWheel { value });
    }

    pub fn pressure(&mut self, value: f32) {
        self.handle_control(ControlMessage::Pressure { value });
    }

    pub fn sustain_pedal(&mut self, pressed: bool) {
        self.handle_control(ControlMessage::SustainPedal { pressed });
    }

    pub fn control_change(&mut self, controller: u8, value: f32) {
        self.handle_control(ControlMessage::ControlChange { controller, value });
    }

    /// Renders mono audio into `buffer` (duplicated internally from the stereo mix).
    pub fn process(&mut self, buffer: &mut [f32]) {
        self.process_interleaved(buffer, 2);
    }

    /// Renders interleaved audio with `channels` samples per frame (1 = mono, 2 = stereo).
    pub fn process_interleaved(&mut self, buffer: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }

        for frame in buffer.chunks_exact_mut(channels) {
            let (left, right) = self.next();
            if channels == 1 {
                frame[0] = (0.5 * (left + right)).clamp(-1.0, 1.0);
            } else {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = if channel % 2 == 0 { left } else { right };
                }
            }
        }
    }

    fn next(&mut self) -> (f32, f32) {
        let (left, right) = self.voices.next();

        let gain = self.master_volume;
        (
            (left * gain).clamp(-1.0, 1.0),
            (right * gain).clamp(-1.0, 1.0),
        )
    }

    pub fn active_notes(&self) -> ActiveNotes<PACKS> {
        self.voices.active_notes()
    }

    pub fn active_voice_count(&self) -> usize {
        self.voices.active_voice_count()
    }
}
