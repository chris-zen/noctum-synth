use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{
    CaptureCase, DurationSecs, MidiChannel, MidiNote, MidiVelocity, OscillatorId,
    OscillatorWaveform, PitchErrorCents, SampleRateHz, UnitInterval,
};

pub mod oscillator_static_v1;

pub use oscillator_static_v1::{
    OSCILLATOR_STATIC_V1_ID, OSCILLATOR_STATIC_V1_REVISION, OscillatorStaticV1,
    scientific_role_for_note,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("target is missing required capability: {0}")]
    MissingCapability(&'static str),
    #[error("domain error: {0}")]
    Domain(#[from] crate::domain::DomainError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolDescriptor {
    pub id: String,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProtocolConfig {
    pub capture_order_seed: String,
    pub target_revision: String,
    pub sample_rate: SampleRateHz,
    pub midi_channel: MidiChannel,
    pub velocity: MidiVelocity,
    pub settle: DurationSecs,
    pub attack_discard: DurationSecs,
    pub stimulated_capture: DurationSecs,
    pub post_note: DurationSecs,
    pub silence_duration: DurationSecs,
    pub permitted_pitch_error_cents: PitchErrorCents,
    pub pulse_width: UnitInterval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetCapabilities {
    pub oscillators: Vec<OscillatorId>,
    pub waveforms: Vec<OscillatorWaveform>,
    pub min_midi_note: MidiNote,
    pub max_midi_note: MidiNote,
    pub supports_silence: bool,
}

pub trait CaptureProtocol {
    fn descriptor(&self) -> ProtocolDescriptor;
    fn validate_target(&self, capabilities: &TargetCapabilities) -> Result<(), ProtocolError>;
    fn build_cases(&self, config: &ProtocolConfig) -> Result<Vec<CaptureCase>, ProtocolError>;
}
