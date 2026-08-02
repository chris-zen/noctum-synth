use crate::{
    domain::{
        DurationSecs, MidiChannel, MidiNote, MidiVelocity, OscillatorId, OscillatorWaveform,
        ParameterSetting,
    },
    midi::MidiTransport,
    protocols::TargetCapabilities,
    targets::{
        AudioRequirements, OperatorSetupStep, SettlePolicy, SynthTarget, TargetDescriptor,
        TargetError, fingerprint_mapping_table, note_off_status, note_on_status, send_cc,
        unit_to_cc,
    },
};

pub const TARGET_ID: &str = "prophet5-v1";
pub const TARGET_REVISION: &str = "prophet5-v1";
pub const ADAPTER_REVISION: &str = "7";
pub const REQUIRED_SAMPLE_RATE_HZ: u32 = 96_000;

const MAPPING_ROWS: &[(u8, &str, u8)] = &[
    (102, "osc1_saw", 0),
    (103, "osc1_triangle", 0),
    (104, "osc1_pulse", 0),
    (105, "osc1_level", 0),
    (106, "osc2_saw", 0),
    (107, "osc2_triangle", 0),
    (108, "osc2_pulse", 0),
    (109, "osc2_level", 127),
    (111, "osc2_keyboard_tracking", 127),
    (112, "osc2_lo_freq", 0),
    (114, "noise_level", 0),
    (115, "osc_sync", 0),
    (116, "filter_cutoff", 127),
    (117, "filter_resonance", 0),
    (13, "filter_keyboard_tracking", 0),
    (119, "amp_attack", 0),
    (14, "amp_decay", 0),
    (15, "amp_sustain", 127),
    (16, "amp_release", 0),
    (17, "filter_attack", 0),
    (18, "filter_decay", 0),
    (19, "filter_sustain", 0),
    (20, "filter_release", 0),
    (21, "unison", 0),
    (22, "oscillator_detune", 0),
    (23, "master_level", 110),
    (24, "polymod_osc2_amount", 0),
    (25, "polymod_noise_amount", 0),
    (26, "lfo_amount", 0),
    (27, "polymod_dest_freq1", 0),
    (28, "polymod_dest_pw1", 0),
    (29, "polymod_dest_filter", 0),
    (30, "lfo_dest_freq", 0),
    (31, "lfo_dest_pw", 0),
    (80, "modulations_enable", 0),
    (81, "keyboard_modulations_enable", 0),
    (82, "pitch_dispersion", 0),
    (83, "pulse_width_dispersion", 0),
    (84, "filter_cutoff_dispersion", 0),
    (85, "filter_resonance_dispersion", 0),
    (86, "envelope_time_dispersion", 0),
    (87, "modulation_dispersion", 0),
    (88, "level_dispersion", 0),
    (89, "fx1_dry_wet", 0),
    (90, "fx2_dry_wet", 0),
    (91, "fx3_dry_wet", 0),
    (92, "arpeggiator_enable", 0),
    (93, "chord_enable", 0),
    (94, "fx1_bypass", 127),
    (95, "fx2_bypass", 127),
    (96, "fx3_bypass", 127),
];

pub struct Prophet5V1 {
    channel: MidiChannel,
}

impl Prophet5V1 {
    pub fn new(channel: MidiChannel) -> Self {
        Self { channel }
    }

    fn lookup(semantic: &str) -> Result<(u8, u8), TargetError> {
        MAPPING_ROWS
            .iter()
            .find(|(_, name, _)| *name == semantic)
            .map(|(cc, _, neutral)| (*cc, *neutral))
            .ok_or_else(|| TargetError::Message(format!("missing mapping for {semantic}")))
    }

    fn cc(
        &self,
        midi: &mut dyn MidiTransport,
        semantic: &str,
        value: u8,
    ) -> Result<(), TargetError> {
        let (controller, _) = Self::lookup(semantic)?;
        send_cc(midi, self.channel, controller, value)
    }

    fn cc_neutral(&self, midi: &mut dyn MidiTransport, semantic: &str) -> Result<(), TargetError> {
        let (controller, neutral) = Self::lookup(semantic)?;
        send_cc(midi, self.channel, controller, neutral)
    }

    fn set_osc2_waveform(
        &self,
        midi: &mut dyn MidiTransport,
        waveform: OscillatorWaveform,
    ) -> Result<(), TargetError> {
        let (saw, triangle, pulse) = match waveform {
            OscillatorWaveform::Saw => (127, 0, 0),
            OscillatorWaveform::Triangle => (0, 127, 0),
            OscillatorWaveform::Pulse => (0, 0, 127),
        };
        self.cc(midi, "osc2_saw", saw)?;
        self.cc(midi, "osc2_triangle", triangle)?;
        self.cc(midi, "osc2_pulse", pulse)?;
        Ok(())
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

pub fn mapping_rows() -> &'static [(u8, &'static str, u8)] {
    MAPPING_ROWS
}

impl SynthTarget for Prophet5V1 {
    fn descriptor(&self) -> TargetDescriptor {
        descriptor()
    }

    fn capabilities(&self) -> TargetCapabilities {
        TargetCapabilities {
            oscillators: vec![OscillatorId::One, OscillatorId::Two],
            waveforms: vec![
                OscillatorWaveform::Saw,
                OscillatorWaveform::Triangle,
                OscillatorWaveform::Pulse,
            ],
            min_midi_note: MidiNote::try_new(0).expect("0 is valid"),
            max_midi_note: MidiNote::try_new(127).expect("127 is valid"),
            supports_silence: true,
        }
    }

    fn audio_requirements(&self) -> AudioRequirements {
        AudioRequirements {
            required_sample_rate_hz: Some(REQUIRED_SAMPLE_RATE_HZ),
            require_native_float32: true,
        }
    }

    fn operator_setup_steps(&self) -> Vec<OperatorSetupStep> {
        vec![
            OperatorSetupStep {
                id: "init_preset_and_mapping".to_string(),
                title: "Load Init preset and revision-7 MIDI mapping".to_string(),
                instructions: String::new(),
            },
            OperatorSetupStep {
                id: "osc2_fine_tune_zero".to_string(),
                title: "VCO 2 Fine Tune = 0.000".to_string(),
                instructions: String::new(),
            },
            OperatorSetupStep {
                id: "osc2_pulse_width_50".to_string(),
                title: "VCO 2 Pulse Width = 50%".to_string(),
                instructions: String::new(),
            },
            OperatorSetupStep {
                id: "filter_env_amount_center".to_string(),
                title: "Filter Envelope Amount = 5.0".to_string(),
                instructions: String::new(),
            },
        ]
    }

    fn reset(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        send_cc(midi, self.channel, 123, 0)?;
        send_cc(midi, self.channel, 64, 0)?;
        send_cc(midi, self.channel, 1, 0)?;

        self.cc_neutral(midi, "osc1_saw")?;
        self.cc_neutral(midi, "osc1_triangle")?;
        self.cc_neutral(midi, "osc1_pulse")?;
        self.cc_neutral(midi, "osc1_level")?;

        self.cc_neutral(midi, "osc2_level")?;
        self.cc_neutral(midi, "osc2_keyboard_tracking")?;
        self.cc_neutral(midi, "osc2_lo_freq")?;
        self.cc_neutral(midi, "osc2_triangle")?;
        self.cc_neutral(midi, "osc2_saw")?;
        self.cc_neutral(midi, "osc2_pulse")?;

        self.cc_neutral(midi, "noise_level")?;
        self.cc_neutral(midi, "osc_sync")?;
        self.cc_neutral(midi, "polymod_osc2_amount")?;
        self.cc_neutral(midi, "polymod_noise_amount")?;
        self.cc_neutral(midi, "polymod_dest_freq1")?;
        self.cc_neutral(midi, "polymod_dest_pw1")?;
        self.cc_neutral(midi, "polymod_dest_filter")?;
        self.cc_neutral(midi, "lfo_amount")?;
        self.cc_neutral(midi, "lfo_dest_freq")?;
        self.cc_neutral(midi, "lfo_dest_pw")?;

        self.cc_neutral(midi, "filter_cutoff")?;
        self.cc_neutral(midi, "filter_resonance")?;
        self.cc_neutral(midi, "filter_keyboard_tracking")?;

        self.cc_neutral(midi, "amp_attack")?;
        self.cc_neutral(midi, "amp_decay")?;
        self.cc_neutral(midi, "amp_sustain")?;
        self.cc_neutral(midi, "amp_release")?;

        self.cc_neutral(midi, "filter_attack")?;
        self.cc_neutral(midi, "filter_decay")?;
        self.cc_neutral(midi, "filter_sustain")?;
        self.cc_neutral(midi, "filter_release")?;

        self.cc_neutral(midi, "unison")?;
        self.cc_neutral(midi, "oscillator_detune")?;
        self.cc_neutral(midi, "master_level")?;

        self.cc_neutral(midi, "modulations_enable")?;
        self.cc_neutral(midi, "keyboard_modulations_enable")?;
        self.cc_neutral(midi, "pitch_dispersion")?;
        self.cc_neutral(midi, "pulse_width_dispersion")?;
        self.cc_neutral(midi, "filter_cutoff_dispersion")?;
        self.cc_neutral(midi, "filter_resonance_dispersion")?;
        self.cc_neutral(midi, "envelope_time_dispersion")?;
        self.cc_neutral(midi, "modulation_dispersion")?;
        self.cc_neutral(midi, "level_dispersion")?;
        self.cc_neutral(midi, "arpeggiator_enable")?;
        self.cc_neutral(midi, "chord_enable")?;
        self.cc_neutral(midi, "fx1_dry_wet")?;
        self.cc_neutral(midi, "fx2_dry_wet")?;
        self.cc_neutral(midi, "fx3_dry_wet")?;
        self.cc_neutral(midi, "fx1_bypass")?;
        self.cc_neutral(midi, "fx2_bypass")?;
        self.cc_neutral(midi, "fx3_bypass")?;

        midi.flush()?;
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
            } => self.set_osc2_waveform(midi, *waveform),
            ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::One,
                ..
            } => Err(TargetError::UnsupportedParameter(
                "oscillator 1 waveform is disabled for prophet5-v1 capture",
            )),
            ParameterSetting::OscillatorPulseWidth {
                oscillator: OscillatorId::Two,
                normalized,
            } => {
                if (normalized.get() - 0.5).abs() <= 1e-9 {
                    Ok(())
                } else {
                    Err(TargetError::UnsupportedParameter(
                        "only exact 50% pulse width is supported; set it manually at session start",
                    ))
                }
            }
            ParameterSetting::OscillatorPulseWidth {
                oscillator: OscillatorId::One,
                ..
            } => Err(TargetError::UnsupportedParameter(
                "oscillator 1 pulse width is unsupported",
            )),
            ParameterSetting::OscillatorLevel {
                oscillator: OscillatorId::Two,
                normalized,
            } => self.cc(midi, "osc2_level", unit_to_cc(*normalized)),
            ParameterSetting::MasterLevel(level) => {
                self.cc(midi, "master_level", unit_to_cc(*level))
            }
            ParameterSetting::NoiseLevel(_)
            | ParameterSetting::OscillatorLevel {
                oscillator: OscillatorId::One,
                ..
            }
            | ParameterSetting::OscillatorTuneSemitones { .. }
            | ParameterSetting::OscillatorKeyboardTracking { .. }
            | ParameterSetting::OscillatorLowFrequencyMode { .. }
            | ParameterSetting::FilterCutoffNormalized(_)
            | ParameterSetting::FilterResonance(_)
            | ParameterSetting::FilterEnvelopeAmount(_)
            | ParameterSetting::AmplifierEnvelope(_)
            | ParameterSetting::FilterEnvelope(_)
            | ParameterSetting::UnisonEnabled(_)
            | ParameterSetting::OscillatorSyncEnabled(_)
            | ParameterSetting::VoiceDispersion(_) => Err(TargetError::UnsupportedParameter(
                "parameter is owned by reset() for prophet5-v1",
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
        Ok(())
    }

    fn note_off(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
    ) -> Result<(), TargetError> {
        midi.send(&[note_off_status(self.channel), note.get(), 0])?;
        Ok(())
    }

    fn panic(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        send_cc(midi, self.channel, 123, 0)?;
        send_cc(midi, self.channel, 64, 0)?;
        midi.flush()?;
        Ok(())
    }

    fn prepare_session(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        self.panic(midi)?;
        for note in 0u8..=127 {
            midi.send(&[note_off_status(self.channel), note, 0])?;
        }
        midi.flush()?;
        Ok(())
    }

    fn settle_policy(&self) -> SettlePolicy {
        SettlePolicy {
            reset_settle: DurationSecs::try_new(0.250).expect("0.250 is valid"),
            parameter_settle: DurationSecs::try_new(0.250).expect("0.250 is valid"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{
            MidiChannel, MidiNote, MidiVelocity, OscillatorId, OscillatorWaveform,
            ParameterSetting, UnitInterval,
        },
        midi::{FakeMidiTransport, TranscriptTransport},
        targets::{
            SynthTarget, TargetError, fingerprint_mapping_table,
            prophet5_v1::{MAPPING_ROWS, Prophet5V1, mapping_fingerprint},
        },
    };

    #[test]
    fn reset_sends_complete_osc2_initialization_and_is_idempotent() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        target.reset(&mut midi).unwrap();
        let first = midi.sent.clone();
        assert!(first.len() > 30);
        assert_eq!(first[0], vec![0xB0, 123, 0]);
        assert_eq!(first[1], vec![0xB0, 64, 0]);
        assert_eq!(first[2], vec![0xB0, 1, 0]);
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 105, 0]));
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 109, 127]));
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 23, 110]));
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 106, 0]));
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 107, 0]));
        assert!(first.iter().any(|msg| msg == &vec![0xB0, 108, 0]));
        for controller in 80..=93 {
            assert!(
                first.iter().any(|msg| msg == &vec![0xB0, controller, 0]),
                "missing neutral CC {controller}"
            );
        }
        for controller in 94..=96 {
            assert!(
                first.iter().any(|msg| msg == &vec![0xB0, controller, 127]),
                "missing bypass CC {controller}"
            );
        }
        assert!(first.iter().all(|msg| msg.get(1) != Some(&110)));
        assert!(first.iter().all(|msg| msg.get(1) != Some(&113)));
        assert!(first.iter().all(|msg| msg.get(1) != Some(&118)));
        assert_eq!(midi.flushed, 1);

        midi.sent.clear();
        midi.flushed = 0;
        target.reset(&mut midi).unwrap();
        assert_eq!(midi.sent, first);
        assert_eq!(midi.flushed, 1);
    }

    #[test]
    fn operator_setup_requests_manual_center_controls() {
        let target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let steps = target.operator_setup_steps();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].id, "init_preset_and_mapping");
        assert_eq!(steps[1].id, "osc2_fine_tune_zero");
        assert_eq!(steps[2].id, "osc2_pulse_width_50");
        assert_eq!(steps[3].id, "filter_env_amount_center");
        assert_eq!(
            steps[0].title,
            "Load Init preset and revision-7 MIDI mapping"
        );
        assert_eq!(steps[1].title, "VCO 2 Fine Tune = 0.000");
        assert_eq!(steps[2].title, "VCO 2 Pulse Width = 50%");
        assert_eq!(steps[3].title, "Filter Envelope Amount = 5.0");
        assert!(steps.iter().all(|step| step.instructions.is_empty()));
    }

    #[test]
    fn waveform_selection_sets_all_three_osc2_switches() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        target
            .set_parameter(
                &mut midi,
                &ParameterSetting::OscillatorWaveform {
                    oscillator: OscillatorId::Two,
                    waveform: OscillatorWaveform::Pulse,
                },
            )
            .unwrap();
        assert_eq!(
            midi.sent,
            vec![vec![0xB0, 106, 0], vec![0xB0, 107, 0], vec![0xB0, 108, 127],]
        );

        midi.sent.clear();
        target
            .set_parameter(
                &mut midi,
                &ParameterSetting::OscillatorWaveform {
                    oscillator: OscillatorId::Two,
                    waveform: OscillatorWaveform::Saw,
                },
            )
            .unwrap();
        assert_eq!(
            midi.sent,
            vec![vec![0xB0, 106, 127], vec![0xB0, 107, 0], vec![0xB0, 108, 0],]
        );
    }

    #[test]
    fn oscillator_one_waveform_is_rejected() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        let err = target
            .set_parameter(
                &mut midi,
                &ParameterSetting::OscillatorWaveform {
                    oscillator: OscillatorId::One,
                    waveform: OscillatorWaveform::Saw,
                },
            )
            .unwrap_err();
        assert!(matches!(err, TargetError::UnsupportedParameter(_)));
        assert!(midi.sent.is_empty());
    }

    #[test]
    fn panic_sends_all_notes_off_and_all_sound_off() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        target.panic(&mut midi).unwrap();
        assert_eq!(midi.sent, vec![vec![0xB0, 123, 0], vec![0xB0, 64, 0]]);
        assert_eq!(midi.flushed, 1);
    }

    #[test]
    fn prepare_session_sends_per_note_offs_once() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        target.prepare_session(&mut midi).unwrap();
        assert_eq!(midi.sent[0], vec![0xB0, 123, 0]);
        assert_eq!(midi.sent[1], vec![0xB0, 64, 0]);
        assert_eq!(midi.sent.len(), 2 + 128);
        assert_eq!(midi.sent[2], vec![0x80, 0, 0]);
        assert_eq!(midi.sent[2 + 127], vec![0x80, 127, 0]);
        assert_eq!(midi.flushed, 2);
    }

    #[test]
    fn notes_and_transcript_decorator() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
        target
            .note_on(
                &mut midi,
                MidiNote::try_new(69).unwrap(),
                MidiVelocity::try_new(100).unwrap(),
            )
            .unwrap();
        target
            .note_off(&mut midi, MidiNote::try_new(69).unwrap())
            .unwrap();
        assert_eq!(midi.entries().len(), 2);
        assert_eq!(midi.entries()[0].bytes, vec![0x90, 69, 100]);
        assert_eq!(midi.entries()[1].bytes, vec![0x80, 69, 0]);
        assert!(!midi.fingerprint().is_empty());
    }

    #[test]
    fn pulse_width_fifty_percent_is_manual_noop() {
        let mut target = Prophet5V1::new(MidiChannel::try_new(1).unwrap());
        let mut midi = FakeMidiTransport::default();
        target
            .set_parameter(
                &mut midi,
                &ParameterSetting::OscillatorPulseWidth {
                    oscillator: OscillatorId::Two,
                    normalized: UnitInterval::try_new(0.5).unwrap(),
                },
            )
            .unwrap();
        assert!(midi.sent.is_empty());

        let err = target
            .set_parameter(
                &mut midi,
                &ParameterSetting::OscillatorPulseWidth {
                    oscillator: OscillatorId::Two,
                    normalized: UnitInterval::try_new(0.25).unwrap(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, TargetError::UnsupportedParameter(_)));

        let original = mapping_fingerprint();
        assert_eq!(original, fingerprint_mapping_table(MAPPING_ROWS));
        assert_eq!(
            original,
            "9816d98209944039c6414c0a48c37ccad474d445fe03d3333a6af414c0012681"
        );
        let mut altered = MAPPING_ROWS.to_vec();
        altered[0].2 = 1;
        assert_ne!(original, fingerprint_mapping_table(&altered));
    }
}
