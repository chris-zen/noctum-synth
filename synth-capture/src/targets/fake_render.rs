use std::sync::{Arc, Mutex};

use crate::{
    audio::fake::RenderEngine,
    domain::{
        DurationSecs, MidiChannel, MidiNote, MidiVelocity, OscillatorId, OscillatorWaveform,
        ParameterSetting,
    },
    midi::MidiTransport,
    protocols::TargetCapabilities,
    targets::{
        AudioRequirements, SettlePolicy, SynthTarget, TargetDescriptor, TargetError,
        fingerprint_mapping_table, note_off_status, note_on_status, send_cc,
    },
};

const MAPPING_ROWS: &[(u8, &str, u8)] = &[
    (106, "osc2_saw", 0),
    (107, "osc2_triangle", 0),
    (108, "osc2_pulse", 0),
    (110, "osc2_pulse_width", 64),
];

pub const TARGET_ID: &str = "fake-render-v1";
pub const TARGET_REVISION: &str = "fake-render-v1";
pub const ADAPTER_REVISION: &str = "1";

pub struct FakeRenderTarget {
    channel: MidiChannel,
    engine: Arc<Mutex<RenderEngine>>,
}

impl FakeRenderTarget {
    pub fn new(channel: MidiChannel, engine: Arc<Mutex<RenderEngine>>) -> Self {
        Self { channel, engine }
    }

    fn engine_mut(&self) -> std::sync::MutexGuard<'_, RenderEngine> {
        self.engine.lock().unwrap_or_else(|err| err.into_inner())
    }
}

pub fn descriptor() -> TargetDescriptor {
    TargetDescriptor {
        id: TARGET_ID.to_string(),
        revision: TARGET_REVISION.to_string(),
        adapter_revision: ADAPTER_REVISION.to_string(),
        mapping_fingerprint: mapping_fingerprint(),
    }
}

pub fn mapping_fingerprint() -> String {
    fingerprint_mapping_table(MAPPING_ROWS)
}

impl SynthTarget for FakeRenderTarget {
    fn descriptor(&self) -> TargetDescriptor {
        descriptor()
    }

    fn capabilities(&self) -> TargetCapabilities {
        TargetCapabilities {
            oscillators: vec![OscillatorId::Two],
            waveforms: vec![
                OscillatorWaveform::Saw,
                OscillatorWaveform::Triangle,
                OscillatorWaveform::Pulse,
            ],
            min_midi_note: MidiNote::try_new(0).expect("valid"),
            max_midi_note: MidiNote::try_new(127).expect("valid"),
            supports_silence: true,
        }
    }

    fn audio_requirements(&self) -> AudioRequirements {
        AudioRequirements {
            required_sample_rate_hz: Some(96_000),
            require_native_float32: true,
        }
    }

    fn reset(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        send_cc(midi, self.channel, 123, 0)?;
        send_cc(midi, self.channel, 64, 0)?;
        midi.flush()?;
        self.engine_mut().reset_neutral();
        Ok(())
    }

    fn set_parameter(
        &mut self,
        midi: &mut dyn MidiTransport,
        setting: &ParameterSetting,
    ) -> Result<(), TargetError> {
        match setting {
            ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::Two,
                waveform,
            } => {
                send_cc(midi, self.channel, 106, 0)?;
                send_cc(midi, self.channel, 107, 0)?;
                send_cc(midi, self.channel, 108, 0)?;
                let cc = match waveform {
                    OscillatorWaveform::Saw => 106,
                    OscillatorWaveform::Triangle => 107,
                    OscillatorWaveform::Pulse => 108,
                };
                send_cc(midi, self.channel, cc, 127)?;
                self.engine_mut().set_waveform(*waveform);
                Ok(())
            }
            ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::One,
                ..
            } => Err(TargetError::UnsupportedParameter(
                "oscillator 1 waveform unsupported on fake target",
            )),
            ParameterSetting::OscillatorPulseWidth {
                oscillator: OscillatorId::Two,
                normalized,
            } => {
                send_cc(
                    midi,
                    self.channel,
                    110,
                    crate::targets::unit_to_cc(*normalized),
                )?;
                self.engine_mut().set_pulse_width(normalized.get());
                Ok(())
            }
            _ => Err(TargetError::UnsupportedParameter(
                "parameter unsupported on fake target",
            )),
        }
    }

    fn note_on(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
        velocity: MidiVelocity,
    ) -> Result<(), TargetError> {
        midi.send(&[note_on_status(self.channel), note.get(), velocity.get()])?;
        self.engine_mut().note_on(note.frequency_hz().get() as f32);
        Ok(())
    }

    fn note_off(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
    ) -> Result<(), TargetError> {
        midi.send(&[note_off_status(self.channel), note.get(), 0])?;
        self.engine_mut().note_off();
        Ok(())
    }

    fn panic(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        send_cc(midi, self.channel, 123, 0)?;
        self.engine_mut().note_off();
        midi.flush()?;
        Ok(())
    }

    fn settle_policy(&self) -> SettlePolicy {
        SettlePolicy {
            reset_settle: DurationSecs::try_new(0.0).expect("valid"),
            parameter_settle: DurationSecs::try_new(0.0).expect("valid"),
        }
    }
}
