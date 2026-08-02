use sha2::{Digest, Sha256};

use crate::{
    domain::{
        CaptureCase, CaseKind, CaseTags, DurationSecs, MidiChannel, MidiNote, MidiVelocity,
        NoteStimulus, OscillatorId, OscillatorWaveform, ParameterSetting, PitchErrorCents,
        SampleRateHz, ScientificRole, UnitInterval,
    },
    protocols::{
        CaptureProtocol, ProtocolConfig, ProtocolDescriptor, ProtocolError, TargetCapabilities,
    },
};

pub const OSCILLATOR_STATIC_V1_REVISION: &str = "oscillator-static-v1";
pub const OSCILLATOR_STATIC_V1_ID: &str = "oscillator-static-v1";
pub const DEFAULT_CAPTURE_ORDER_SEED: &str = "analog-osc-capture-order-v1";

pub const FIRST_NOTE: u8 = 16;
pub const LAST_PLAYABLE_NOTE: u8 = 88;
pub const LAST_GUARD_NOTE: u8 = 90;
pub const GUARD_VALIDATION_NOTE: u8 = 89;
pub const GUARD_TRAINING_NOTE: u8 = 90;

const WAVEFORMS: [OscillatorWaveform; 3] = [
    OscillatorWaveform::Saw,
    OscillatorWaveform::Triangle,
    OscillatorWaveform::Pulse,
];

#[derive(Clone, Debug, Default)]
pub struct OscillatorStaticV1;

impl OscillatorStaticV1 {
    pub fn default_config(
        target_revision: impl Into<String>,
    ) -> Result<ProtocolConfig, ProtocolError> {
        Ok(ProtocolConfig {
            capture_order_seed: DEFAULT_CAPTURE_ORDER_SEED.to_string(),
            target_revision: target_revision.into(),
            sample_rate: SampleRateHz::try_new(96_000)?,
            midi_channel: MidiChannel::try_new(1)?,
            velocity: MidiVelocity::try_new(100)?,
            settle: DurationSecs::try_new(0.250)?,
            attack_discard: DurationSecs::try_new(0.500)?,
            stimulated_capture: DurationSecs::try_new(8.0)?,
            post_note: DurationSecs::try_new(0.100)?,
            silence_duration: DurationSecs::try_new(10.0)?,
            permitted_pitch_error_cents: PitchErrorCents::try_new(50.0)?,
            pulse_width: UnitInterval::try_new(0.5)?,
        })
    }
}

impl CaptureProtocol for OscillatorStaticV1 {
    fn descriptor(&self) -> ProtocolDescriptor {
        ProtocolDescriptor {
            id: OSCILLATOR_STATIC_V1_ID.to_string(),
            revision: OSCILLATOR_STATIC_V1_REVISION.to_string(),
        }
    }

    fn validate_target(&self, capabilities: &TargetCapabilities) -> Result<(), ProtocolError> {
        if !capabilities.oscillators.contains(&OscillatorId::Two) {
            return Err(ProtocolError::MissingCapability("oscillator 2"));
        }
        for waveform in WAVEFORMS {
            if !capabilities.waveforms.contains(&waveform) {
                return Err(ProtocolError::MissingCapability("required waveform"));
            }
        }
        if capabilities.min_midi_note.get() > FIRST_NOTE
            || capabilities.max_midi_note.get() < LAST_GUARD_NOTE
        {
            return Err(ProtocolError::MissingCapability("midi note range 16..=90"));
        }
        if !capabilities.supports_silence {
            return Err(ProtocolError::MissingCapability("silence capture"));
        }
        Ok(())
    }

    fn build_cases(&self, config: &ProtocolConfig) -> Result<Vec<CaptureCase>, ProtocolError> {
        let mut cases = Vec::with_capacity(226);
        cases.push(silence_case(config)?);

        for note_number in FIRST_NOTE..=LAST_GUARD_NOTE {
            let note = MidiNote::try_new(note_number)?;
            let role = scientific_role_for_note(note)?;
            for waveform in WAVEFORMS {
                cases.push(stimulated_case(config, note, waveform, role)?);
            }
        }

        cases.sort_by(|left, right| {
            order_key(&config.capture_order_seed, &left.id)
                .cmp(&order_key(&config.capture_order_seed, &right.id))
                .then_with(|| left.id.cmp(&right.id))
        });

        Ok(cases)
    }
}

pub fn scientific_role_for_note(note: MidiNote) -> Result<ScientificRole, ProtocolError> {
    match note.get() {
        GUARD_VALIDATION_NOTE => Ok(ScientificRole::GuardValidation),
        GUARD_TRAINING_NOTE => Ok(ScientificRole::GuardTraining),
        value if (FIRST_NOTE..=LAST_PLAYABLE_NOTE).contains(&value) => {
            let offset = u16::from(value - FIRST_NOTE);
            if offset % 2 == 0 {
                Ok(ScientificRole::Training)
            } else if offset % 4 == 1 {
                Ok(ScientificRole::Validation)
            } else {
                Ok(ScientificRole::Test)
            }
        }
        _ => Err(ProtocolError::MissingCapability(
            "note outside protocol range",
        )),
    }
}

fn silence_case(config: &ProtocolConfig) -> Result<CaptureCase, ProtocolError> {
    Ok(CaptureCase {
        id: format!("{OSCILLATOR_STATIC_V1_REVISION}/silence"),
        kind: CaseKind::Silence,
        settings: Vec::new(),
        stimulus: None,
        settle: config.settle,
        attack_discard: DurationSecs::try_new(0.0)?,
        capture: config.silence_duration,
        post_note: DurationSecs::try_new(0.0)?,
        expected_fundamental_hz: None,
        permitted_pitch_error_cents: config.permitted_pitch_error_cents,
        role: ScientificRole::NoiseFloor,
        tags: CaseTags {
            waveform: None,
            note: None,
            pulse_width: None,
            oscillator: None,
            protocol_revision: OSCILLATOR_STATIC_V1_REVISION.to_string(),
            target_revision: config.target_revision.clone(),
        },
    })
}

fn stimulated_case(
    config: &ProtocolConfig,
    note: MidiNote,
    waveform: OscillatorWaveform,
    role: ScientificRole,
) -> Result<CaptureCase, ProtocolError> {
    let pulse_width = match waveform {
        OscillatorWaveform::Pulse => Some(config.pulse_width),
        OscillatorWaveform::Saw | OscillatorWaveform::Triangle => None,
    };

    let mut settings = vec![ParameterSetting::OscillatorWaveform {
        oscillator: OscillatorId::Two,
        waveform,
    }];
    if let Some(width) = pulse_width {
        settings.push(ParameterSetting::OscillatorPulseWidth {
            oscillator: OscillatorId::Two,
            normalized: width,
        });
    }

    let wave_slug = match waveform {
        OscillatorWaveform::Saw => "saw",
        OscillatorWaveform::Triangle => "triangle",
        OscillatorWaveform::Pulse => "pulse50",
    };

    Ok(CaptureCase {
        id: format!(
            "{OSCILLATOR_STATIC_V1_REVISION}/osc2/{wave_slug}/midi-{:03}",
            note.get()
        ),
        kind: CaseKind::Stimulated,
        settings,
        stimulus: Some(NoteStimulus {
            note,
            velocity: config.velocity,
        }),
        settle: config.settle,
        attack_discard: config.attack_discard,
        capture: config.stimulated_capture,
        post_note: config.post_note,
        expected_fundamental_hz: Some(note.frequency_hz()),
        permitted_pitch_error_cents: config.permitted_pitch_error_cents,
        role,
        tags: CaseTags {
            waveform: Some(waveform),
            note: Some(note),
            pulse_width,
            oscillator: Some(OscillatorId::Two),
            protocol_revision: OSCILLATOR_STATIC_V1_REVISION.to_string(),
            target_revision: config.target_revision.clone(),
        },
    })
}

fn order_key(seed: &str, case_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update((seed.len() as u64).to_le_bytes());
    hasher.update(seed.as_bytes());
    hasher.update((case_id.len() as u64).to_le_bytes());
    hasher.update(case_id.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{MidiNote, OscillatorId, OscillatorWaveform, ScientificRole},
        protocols::{
            CaptureProtocol, TargetCapabilities,
            oscillator_static_v1::{
                FIRST_NOTE, GUARD_TRAINING_NOTE, GUARD_VALIDATION_NOTE, LAST_GUARD_NOTE,
                LAST_PLAYABLE_NOTE, OscillatorStaticV1,
            },
        },
    };

    #[test]
    fn builds_exact_226_case_matrix() {
        let protocol = OscillatorStaticV1;
        let config = OscillatorStaticV1::default_config("prophet5-v1").unwrap();
        let cases = protocol.build_cases(&config).unwrap();

        assert_eq!(cases.len(), 226);
        assert_eq!(
            cases
                .iter()
                .filter(|case| case.tags.waveform == Some(OscillatorWaveform::Saw))
                .count(),
            75
        );
        assert_eq!(
            cases
                .iter()
                .filter(|case| case.tags.waveform == Some(OscillatorWaveform::Triangle))
                .count(),
            75
        );
        assert_eq!(
            cases
                .iter()
                .filter(|case| case.tags.waveform == Some(OscillatorWaveform::Pulse))
                .count(),
            75
        );
        assert_eq!(
            cases
                .iter()
                .filter(|case| case.kind == crate::domain::CaseKind::Silence)
                .count(),
            1
        );
    }

    #[test]
    fn role_split_and_guards() {
        let protocol = OscillatorStaticV1;
        let config = OscillatorStaticV1::default_config("prophet5-v1").unwrap();
        let cases = protocol.build_cases(&config).unwrap();

        let stimulated: Vec<_> = cases
            .iter()
            .filter(|case| case.kind == crate::domain::CaseKind::Stimulated)
            .collect();

        let playable = stimulated
            .iter()
            .filter(|case| {
                let note = case.tags.note.unwrap().get();
                (FIRST_NOTE..=LAST_PLAYABLE_NOTE).contains(&note)
            })
            .count();
        assert_eq!(playable, 73 * 3);

        let training = stimulated
            .iter()
            .filter(|case| case.role == ScientificRole::Training)
            .count();
        let validation = stimulated
            .iter()
            .filter(|case| case.role == ScientificRole::Validation)
            .count();
        let test = stimulated
            .iter()
            .filter(|case| case.role == ScientificRole::Test)
            .count();
        let guard_validation = stimulated
            .iter()
            .filter(|case| case.role == ScientificRole::GuardValidation)
            .count();
        let guard_training = stimulated
            .iter()
            .filter(|case| case.role == ScientificRole::GuardTraining)
            .count();

        assert_eq!(training, 37 * 3);
        assert_eq!(validation, 18 * 3);
        assert_eq!(test, 18 * 3);
        assert_eq!(guard_validation, 3);
        assert_eq!(guard_training, 3);

        let note_89 = MidiNote::try_new(GUARD_VALIDATION_NOTE).unwrap();
        let note_90 = MidiNote::try_new(GUARD_TRAINING_NOTE).unwrap();
        assert_eq!(
            super::scientific_role_for_note(note_89).unwrap(),
            ScientificRole::GuardValidation
        );
        assert_eq!(
            super::scientific_role_for_note(note_90).unwrap(),
            ScientificRole::GuardTraining
        );
    }

    #[test]
    fn case_ids_and_order_are_stable() {
        let protocol = OscillatorStaticV1;
        let config = OscillatorStaticV1::default_config("prophet5-v1").unwrap();
        let first = protocol.build_cases(&config).unwrap();
        let second = protocol.build_cases(&config).unwrap();

        let ids: Vec<_> = first.iter().map(|case| case.id.as_str()).collect();
        assert!(ids.contains(&"oscillator-static-v1/silence"));
        assert!(ids.contains(&"oscillator-static-v1/osc2/saw/midi-016"));
        assert!(ids.contains(&"oscillator-static-v1/osc2/pulse50/midi-090"));
        assert_eq!(
            first.iter().map(|case| &case.id).collect::<Vec<_>>(),
            second.iter().map(|case| &case.id).collect::<Vec<_>>()
        );

        let mut resorted_seed = config.clone();
        resorted_seed.capture_order_seed = "different-seed".to_string();
        let reordered = protocol.build_cases(&resorted_seed).unwrap();
        assert_ne!(
            first.iter().map(|case| &case.id).collect::<Vec<_>>(),
            reordered.iter().map(|case| &case.id).collect::<Vec<_>>()
        );

        let ambiguous_a = super::order_key("ab", "c");
        let ambiguous_b = super::order_key("a", "bc");
        assert_ne!(ambiguous_a, ambiguous_b);

        let ids: std::collections::BTreeSet<_> =
            first.iter().map(|case| case.id.as_str()).collect();
        assert_eq!(ids.len(), first.len());
    }

    #[test]
    fn stimulated_frames_and_pulse_settings() {
        let protocol = OscillatorStaticV1;
        let config = OscillatorStaticV1::default_config("prophet5-v1").unwrap();
        let cases = protocol.build_cases(&config).unwrap();

        let saw = cases
            .iter()
            .find(|case| case.id == "oscillator-static-v1/osc2/saw/midi-069")
            .unwrap();
        assert_eq!(saw.capture.frames(config.sample_rate), 768_000);
        assert_eq!(saw.settings.len(), 1);

        let pulse = cases
            .iter()
            .find(|case| case.id == "oscillator-static-v1/osc2/pulse50/midi-016")
            .unwrap();
        assert_eq!(pulse.settings.len(), 2);
        assert_eq!(pulse.tags.oscillator, Some(OscillatorId::Two));
        assert_eq!(pulse.tags.pulse_width.unwrap().get(), 0.5);

        let silence = cases
            .iter()
            .find(|case| case.id == "oscillator-static-v1/silence")
            .unwrap();
        assert_eq!(silence.capture.frames(config.sample_rate), 960_000);
        assert_eq!(silence.role, ScientificRole::NoiseFloor);
    }

    #[test]
    fn validate_target_requires_osc2_waveforms_and_range() {
        let protocol = OscillatorStaticV1;
        let ok = TargetCapabilities {
            oscillators: vec![OscillatorId::Two],
            waveforms: vec![
                OscillatorWaveform::Saw,
                OscillatorWaveform::Triangle,
                OscillatorWaveform::Pulse,
            ],
            min_midi_note: MidiNote::try_new(FIRST_NOTE).unwrap(),
            max_midi_note: MidiNote::try_new(LAST_GUARD_NOTE).unwrap(),
            supports_silence: true,
        };
        assert!(protocol.validate_target(&ok).is_ok());

        let mut missing_wave = ok.clone();
        missing_wave.waveforms.pop();
        assert!(protocol.validate_target(&missing_wave).is_err());
    }
}
