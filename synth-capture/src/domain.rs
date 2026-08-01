use core::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("{field} is not finite")]
    NotFinite { field: &'static str },
    #[error("{field} value {value} is outside {min}..={max}")]
    OutOfRange {
        field: &'static str,
        value: String,
        min: String,
        max: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct MidiChannel(u8);

impl MidiChannel {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 16;

    pub fn try_new(value: u8) -> Result<Self, DomainError> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DomainError::OutOfRange {
                field: "midi_channel",
                value: value.to_string(),
                min: Self::MIN.to_string(),
                max: Self::MAX.to_string(),
            })
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for MidiChannel {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<MidiChannel> for u8 {
    fn from(value: MidiChannel) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct MidiNote(u8);

impl MidiNote {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 127;

    pub fn try_new(value: u8) -> Result<Self, DomainError> {
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }

    pub fn frequency_hz(self) -> FrequencyHz {
        let hz = 440.0 * 2.0_f64.powf((f64::from(self.0) - 69.0) / 12.0);
        FrequencyHz(hz)
    }
}

impl TryFrom<u8> for MidiNote {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<MidiNote> for u8 {
    fn from(value: MidiNote) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct MidiVelocity(u8);

impl MidiVelocity {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 127;

    pub fn try_new(value: u8) -> Result<Self, DomainError> {
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for MidiVelocity {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<MidiVelocity> for u8 {
    fn from(value: MidiVelocity) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct UnitInterval(f32);

impl UnitInterval {
    pub fn try_new(value: f32) -> Result<Self, DomainError> {
        require_finite("unit_interval", value)?;
        if !(0.0..=1.0).contains(&value) {
            return Err(DomainError::OutOfRange {
                field: "unit_interval",
                value: value.to_string(),
                min: "0.0".to_string(),
                max: "1.0".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for UnitInterval {
    type Error = DomainError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<UnitInterval> for f32 {
    fn from(value: UnitInterval) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct BipolarUnit(f32);

impl BipolarUnit {
    pub fn try_new(value: f32) -> Result<Self, DomainError> {
        require_finite("bipolar_unit", value)?;
        if !(-1.0..=1.0).contains(&value) {
            return Err(DomainError::OutOfRange {
                field: "bipolar_unit",
                value: value.to_string(),
                min: "-1.0".to_string(),
                max: "1.0".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for BipolarUnit {
    type Error = DomainError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<BipolarUnit> for f32 {
    fn from(value: BipolarUnit) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct DurationSecs(f64);

impl DurationSecs {
    pub fn try_new(value: f64) -> Result<Self, DomainError> {
        require_finite_f64("duration_secs", value)?;
        if value < 0.0 {
            return Err(DomainError::OutOfRange {
                field: "duration_secs",
                value: value.to_string(),
                min: "0.0".to_string(),
                max: "inf".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }

    pub fn frames(self, sample_rate: SampleRateHz) -> u64 {
        (self.0 * f64::from(sample_rate.get())).round() as u64
    }
}

impl TryFrom<f64> for DurationSecs {
    type Error = DomainError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<DurationSecs> for f64 {
    fn from(value: DurationSecs) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct FrequencyHz(f64);

impl FrequencyHz {
    pub fn try_new(value: f64) -> Result<Self, DomainError> {
        require_finite_f64("frequency_hz", value)?;
        if value <= 0.0 {
            return Err(DomainError::OutOfRange {
                field: "frequency_hz",
                value: value.to_string(),
                min: "> 0.0".to_string(),
                max: "inf".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for FrequencyHz {
    type Error = DomainError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<FrequencyHz> for f64 {
    fn from(value: FrequencyHz) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct PitchErrorCents(f64);

impl PitchErrorCents {
    pub fn try_new(value: f64) -> Result<Self, DomainError> {
        require_finite_f64("pitch_error_cents", value)?;
        if value < 0.0 {
            return Err(DomainError::OutOfRange {
                field: "pitch_error_cents",
                value: value.to_string(),
                min: "0.0".to_string(),
                max: "inf".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for PitchErrorCents {
    type Error = DomainError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<PitchErrorCents> for f64 {
    fn from(value: PitchErrorCents) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct SampleRateHz(u32);

impl SampleRateHz {
    pub fn try_new(value: u32) -> Result<Self, DomainError> {
        if value == 0 {
            return Err(DomainError::OutOfRange {
                field: "sample_rate_hz",
                value: value.to_string(),
                min: "1".to_string(),
                max: u32::MAX.to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for SampleRateHz {
    type Error = DomainError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<SampleRateHz> for u32 {
    fn from(value: SampleRateHz) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OscillatorId {
    One,
    Two,
}

impl fmt::Display for OscillatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::One => write!(f, "osc1"),
            Self::Two => write!(f, "osc2"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OscillatorWaveform {
    Saw,
    Triangle,
    Pulse,
}

impl fmt::Display for OscillatorWaveform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Saw => write!(f, "saw"),
            Self::Triangle => write!(f, "triangle"),
            Self::Pulse => write!(f, "pulse"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeSetting {
    pub attack: UnitInterval,
    pub decay: UnitInterval,
    pub sustain: UnitInterval,
    pub release: UnitInterval,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterSetting {
    OscillatorWaveform {
        oscillator: OscillatorId,
        waveform: OscillatorWaveform,
    },
    OscillatorPulseWidth {
        oscillator: OscillatorId,
        normalized: UnitInterval,
    },
    OscillatorLevel {
        oscillator: OscillatorId,
        normalized: UnitInterval,
    },
    OscillatorTuneSemitones {
        oscillator: OscillatorId,
        semitones: i16,
    },
    OscillatorKeyboardTracking {
        oscillator: OscillatorId,
        enabled: bool,
    },
    OscillatorLowFrequencyMode {
        oscillator: OscillatorId,
        enabled: bool,
    },
    NoiseLevel(UnitInterval),
    FilterCutoffNormalized(UnitInterval),
    FilterResonance(UnitInterval),
    FilterEnvelopeAmount(BipolarUnit),
    AmplifierEnvelope(EnvelopeSetting),
    FilterEnvelope(EnvelopeSetting),
    UnisonEnabled(bool),
    OscillatorSyncEnabled(bool),
    VoiceDispersion(UnitInterval),
    MasterLevel(UnitInterval),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    Silence,
    Stimulated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScientificRole {
    Training,
    Validation,
    Test,
    GuardValidation,
    GuardTraining,
    NoiseFloor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseTags {
    pub waveform: Option<OscillatorWaveform>,
    pub note: Option<MidiNote>,
    pub pulse_width: Option<UnitInterval>,
    pub oscillator: Option<OscillatorId>,
    pub protocol_revision: String,
    pub target_revision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteStimulus {
    pub note: MidiNote,
    pub velocity: MidiVelocity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureCase {
    pub id: String,
    pub kind: CaseKind,
    pub settings: Vec<ParameterSetting>,
    pub stimulus: Option<NoteStimulus>,
    pub settle: DurationSecs,
    pub attack_discard: DurationSecs,
    pub capture: DurationSecs,
    pub post_note: DurationSecs,
    pub expected_fundamental_hz: Option<FrequencyHz>,
    pub permitted_pitch_error_cents: PitchErrorCents,
    pub role: ScientificRole,
    pub tags: CaseTags,
}

fn require_finite(field: &'static str, value: f32) -> Result<(), DomainError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DomainError::NotFinite { field })
    }
}

fn require_finite_f64(field: &'static str, value: f64) -> Result<(), DomainError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DomainError::NotFinite { field })
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        BipolarUnit, DomainError, DurationSecs, FrequencyHz, MidiChannel, MidiNote, MidiVelocity,
        PitchErrorCents, SampleRateHz, UnitInterval,
    };

    #[test]
    fn midi_channel_rejects_out_of_range() {
        assert!(MidiChannel::try_new(1).is_ok());
        assert!(MidiChannel::try_new(16).is_ok());
        assert!(matches!(
            MidiChannel::try_new(0),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(matches!(
            MidiChannel::try_new(17),
            Err(DomainError::OutOfRange { .. })
        ));
    }

    #[test]
    fn integer_newtypes_reject_illegal_json() {
        assert!(serde_json::from_str::<MidiChannel>("0").is_err());
        assert!(serde_json::from_str::<MidiChannel>("1").is_ok());
        assert!(serde_json::from_str::<SampleRateHz>("0").is_err());
        assert!(serde_json::from_str::<SampleRateHz>("96000").is_ok());
        assert!(serde_json::from_str::<MidiVelocity>("100").is_ok());
    }

    #[test]
    fn unit_interval_rejects_nan_and_out_of_range() {
        assert!(UnitInterval::try_new(0.0).is_ok());
        assert!(UnitInterval::try_new(1.0).is_ok());
        assert!(matches!(
            UnitInterval::try_new(-0.01),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(matches!(
            UnitInterval::try_new(1.01),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(matches!(
            UnitInterval::try_new(f32::NAN),
            Err(DomainError::NotFinite { .. })
        ));
        assert!(matches!(
            UnitInterval::try_new(f32::INFINITY),
            Err(DomainError::NotFinite { .. })
        ));
    }

    #[test]
    fn bipolar_unit_and_duration_validate() {
        assert!(BipolarUnit::try_new(-1.0).is_ok());
        assert!(BipolarUnit::try_new(1.0).is_ok());
        assert!(matches!(
            BipolarUnit::try_new(1.5),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(DurationSecs::try_new(0.0).is_ok());
        assert!(matches!(
            DurationSecs::try_new(-1.0),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(matches!(
            DurationSecs::try_new(f64::NAN),
            Err(DomainError::NotFinite { .. })
        ));
    }

    #[test]
    fn frequency_and_pitch_error_validate() {
        assert!(FrequencyHz::try_new(440.0).is_ok());
        assert!(matches!(
            FrequencyHz::try_new(0.0),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(matches!(
            FrequencyHz::try_new(f64::NAN),
            Err(DomainError::NotFinite { .. })
        ));
        assert!(PitchErrorCents::try_new(50.0).is_ok());
        assert!(matches!(
            PitchErrorCents::try_new(-1.0),
            Err(DomainError::OutOfRange { .. })
        ));
        assert!(serde_json::from_str::<PitchErrorCents>("-1.0").is_err());
        assert!(serde_json::from_str::<FrequencyHz>("0.0").is_err());
    }

    #[test]
    fn sample_rate_rejects_zero() {
        assert!(SampleRateHz::try_new(96_000).is_ok());
        assert!(matches!(
            SampleRateHz::try_new(0),
            Err(DomainError::OutOfRange { .. })
        ));
    }

    #[test]
    fn midi_note_a4_frequency() {
        let note = MidiNote::try_new(69).unwrap();
        assert!((note.frequency_hz().get() - 440.0).abs() < 1e-9);
        assert!(MidiVelocity::try_new(100).is_ok());
    }

    #[test]
    fn stimulated_frames_at_96k() {
        let duration = DurationSecs::try_new(8.0).unwrap();
        let rate = SampleRateHz::try_new(96_000).unwrap();
        assert_eq!(duration.frames(rate), 768_000);
    }
}
