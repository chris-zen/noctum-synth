use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use synth_capture::{
    audio::{
        AudioError, AudioFormat, AudioHealth, AudioInput, StopFlag,
        fake::{FakeAudioInput, RenderEngine},
    },
    doctor::{
        DoctorConfig, DoctorError, DoctorRecord, read_doctor_record, require_doctor_success,
        run_doctor, write_doctor_record,
    },
    domain::{
        CaptureCase, CaseKind, CaseTags, DurationSecs, MidiChannel, MidiNote, MidiVelocity,
        NoteStimulus, OscillatorId, OscillatorWaveform, ParameterSetting, PitchErrorCents,
        ScientificRole,
    },
    events::NullReporter,
    midi::{FakeMidiTransport, MidiTransport, TranscriptTransport},
    project::{CaptureProject, NewProjectRequest},
    runner::{RunConfig, run_capture},
    targets::{
        AudioRequirements, SettlePolicy, SkipOperatorConfirmer, SynthTarget, TargetDescriptor,
        TargetError, fake_render, fake_render::FakeRenderTarget,
    },
};

const SAMPLE_RATE: u32 = 96_000;

#[test]
fn doctor_passes_and_gates_run_with_fake_devices() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("ok"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(Arc::clone(&engine));

    assert!(matches!(
        require_doctor_success(&project),
        Err(DoctorError::MissingRecord(_))
    ));

    let record = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap();
    assert!(record.ok);
    assert_eq!(record.probes.len(), 10);
    assert_eq!(record.distinctness.len(), 9);
    assert_eq!(record.coherence.len(), 6);
    assert_eq!(record.audio_format.sample_rate_hz, SAMPLE_RATE);
    assert_eq!(record.target, fake_render::descriptor());
    let saw = record
        .probes
        .iter()
        .find(|probe| probe.waveform == Some(OscillatorWaveform::Saw))
        .unwrap();
    assert!(saw.pitch_error_cents.unwrap().abs() <= 50.0);
    assert!(saw.metrics.rms > 0.0);

    write_doctor_record(&project, &record).unwrap();
    assert!(project.doctor_record_path().exists());
    let reloaded = read_doctor_record(&project).unwrap();
    assert_eq!(reloaded, record);
    require_doctor_success(&project).unwrap();

    let mut project = project;
    let summary = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        RunConfig {
            session_id: "doctor-gated".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert_eq!(summary.completed, 2);
}

#[test]
fn stale_doctor_record_is_rejected() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("stale"), short_cases());
    write_doctor_record(&project, &passing_record(&project)).unwrap();
    require_doctor_success(&project).unwrap();

    let mut record = passing_record(&project);
    record.scientific_fingerprint = "0".repeat(64);
    write_doctor_record(&project, &record).unwrap();
    assert!(matches!(
        require_doctor_success(&project),
        Err(DoctorError::Incompatible(_))
    ));

    let mut record = passing_record(&project);
    record.ok = false;
    write_doctor_record(&project, &record).unwrap();
    assert!(matches!(
        require_doctor_success(&project),
        Err(DoctorError::Incompatible(_))
    ));

    let mut record = passing_record(&project);
    record.target.mapping_fingerprint = "changed".to_string();
    write_doctor_record(&project, &record).unwrap();
    assert!(matches!(
        require_doctor_success(&project),
        Err(DoctorError::Incompatible(_))
    ));

    let mut record = passing_record(&project);
    record.audio_format.sample_rate_hz = 48_000;
    write_doctor_record(&project, &record).unwrap();
    assert!(matches!(
        require_doctor_success(&project),
        Err(DoctorError::Incompatible(_))
    ));
}

#[test]
fn doctor_fails_on_silent_probe() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("silent"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), engine);
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = ConstantInput::new(0.0);

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    match err {
        DoctorError::Probe { probe, reason } => {
            assert_eq!(probe, "saw-48");
            assert!(reason.contains("RMS too low"), "{reason}");
        }
        other => panic!("unexpected error {other}"),
    }
    assert!(!project.doctor_record_path().exists());
}

#[test]
fn doctor_fails_on_clipping_probe() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("clip"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), engine);
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = ConstantInput::new(1.0);

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    match err {
        DoctorError::Probe { reason, .. } => assert!(reason.contains("clipping"), "{reason}"),
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn doctor_fails_on_noisy_silence_probe() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("noisy"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), engine);
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = ConstantInput::new(0.05);

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    match err {
        DoctorError::Probe { probe, reason } => {
            assert_eq!(probe, "silence");
            assert!(reason.contains("silence RMS too high"), "{reason}");
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn doctor_fails_when_probes_are_identical() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("identical"), short_cases());
    let engine = engine();
    let mut target =
        StuckWaveformTarget::new(FakeRenderTarget::new(channel(), Arc::clone(&engine)));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(engine);

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    assert!(matches!(err, DoctorError::NotDistinct { .. }), "{err}");
}

#[test]
fn doctor_fails_on_wrong_sample_rate() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("rate"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::with_format(
        engine,
        AudioFormat {
            sample_rate_hz: 48_000,
            channels: 2,
            input_channel: 0,
            native_float32: true,
        },
    );

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    assert!(matches!(err, DoctorError::AudioFormat(_)), "{err}");
    assert!(midi.entries().is_empty());
}

#[test]
fn doctor_fails_on_integer_input_format() {
    let dir = tempdir().unwrap();
    let project = project_with_cases(dir.path().join("integer"), short_cases());
    let engine = engine();
    let mut target = FakeRenderTarget::new(channel(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::with_format(
        engine,
        AudioFormat {
            sample_rate_hz: SAMPLE_RATE,
            channels: 2,
            input_channel: 0,
            native_float32: false,
        },
    );

    let err = run_doctor(
        &project,
        &mut target,
        &mut midi,
        &mut audio,
        &StopFlag::new(),
        &config(),
        &mut SkipOperatorConfirmer,
        &mut NullReporter,
    )
    .unwrap_err();
    assert!(matches!(err, DoctorError::AudioFormat(_)), "{err}");
}

struct ConstantInput {
    value: f32,
    format: AudioFormat,
}

impl ConstantInput {
    fn new(value: f32) -> Self {
        Self {
            value,
            format: AudioFormat {
                sample_rate_hz: SAMPLE_RATE,
                channels: 1,
                input_channel: 0,
                native_float32: true,
            },
        }
    }
}

impl AudioInput for ConstantInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn drain_frames(&mut self, frame_count: usize, dest: &mut Vec<f32>) -> Result<(), AudioError> {
        dest.clear();
        dest.resize(frame_count, self.value);
        Ok(())
    }

    fn health(&self) -> AudioHealth {
        AudioHealth::default()
    }

    fn reset_health(&mut self) {}
}

struct StuckWaveformTarget {
    inner: FakeRenderTarget,
}

impl StuckWaveformTarget {
    fn new(inner: FakeRenderTarget) -> Self {
        Self { inner }
    }
}

impl SynthTarget for StuckWaveformTarget {
    fn descriptor(&self) -> TargetDescriptor {
        self.inner.descriptor()
    }

    fn capabilities(&self) -> synth_capture::protocols::TargetCapabilities {
        self.inner.capabilities()
    }

    fn audio_requirements(&self) -> AudioRequirements {
        self.inner.audio_requirements()
    }

    fn reset(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        self.inner.reset(midi)
    }

    fn set_parameter(
        &mut self,
        midi: &mut dyn MidiTransport,
        setting: &ParameterSetting,
    ) -> Result<(), TargetError> {
        let forced = match setting {
            ParameterSetting::OscillatorWaveform { oscillator, .. } => {
                ParameterSetting::OscillatorWaveform {
                    oscillator: *oscillator,
                    waveform: OscillatorWaveform::Saw,
                }
            }
            other => other.clone(),
        };
        self.inner.set_parameter(midi, &forced)
    }

    fn note_on(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
        velocity: MidiVelocity,
    ) -> Result<(), TargetError> {
        self.inner.note_on(midi, note, velocity)
    }

    fn note_off(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
    ) -> Result<(), TargetError> {
        self.inner.note_off(midi, note)
    }

    fn panic(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        self.inner.panic(midi)
    }

    fn settle_policy(&self) -> SettlePolicy {
        self.inner.settle_policy()
    }
}

fn config() -> DoctorConfig {
    DoctorConfig {
        probe_duration: DurationSecs::try_new(0.25).unwrap(),
        sleep_enabled: false,
    }
}

fn channel() -> MidiChannel {
    MidiChannel::try_new(1).unwrap()
}

fn engine() -> Arc<Mutex<RenderEngine>> {
    Arc::new(Mutex::new(RenderEngine::new(SAMPLE_RATE as f32)))
}

fn project_with_cases(root: std::path::PathBuf, cases: Vec<CaptureCase>) -> CaptureProject {
    CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: fake_render::TARGET_ID.to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: SAMPLE_RATE,
            plugin_version: "test".to_string(),
        },
        cases,
    )
    .unwrap()
}

fn passing_record(project: &CaptureProject) -> DoctorRecord {
    DoctorRecord {
        schema_id: "synth-capture-doctor-v2".to_string(),
        ok: true,
        created_at_unix_ms: 1,
        project_id: project.document().project_id.clone(),
        scientific_fingerprint: project.document().scientific_fingerprint.clone(),
        target: project.document().target.clone(),
        protocol: project.document().protocol.clone(),
        midi_port: project.document().midi_port.clone(),
        audio_device: project.document().audio_device.clone(),
        audio_format: AudioFormat {
            sample_rate_hz: SAMPLE_RATE,
            channels: 2,
            input_channel: 0,
            native_float32: true,
        },
        probe_frames: 24_000,
        target_settle_secs: 0.0,
        probes: vec![],
        distinctness: vec![],
        coherence: vec![],
    }
}

fn short_cases() -> Vec<CaptureCase> {
    let zero = DurationSecs::try_new(0.0).unwrap();
    let pitch = PitchErrorCents::try_new(50.0).unwrap();
    let note = MidiNote::try_new(69).unwrap();
    vec![
        CaptureCase {
            id: "test/silence".to_string(),
            kind: CaseKind::Silence,
            settings: Vec::new(),
            stimulus: None,
            settle: zero,
            attack_discard: zero,
            capture: DurationSecs::try_new(0.02).unwrap(),
            post_note: zero,
            expected_fundamental_hz: None,
            permitted_pitch_error_cents: pitch,
            role: ScientificRole::NoiseFloor,
            tags: CaseTags {
                waveform: None,
                note: None,
                pulse_width: None,
                oscillator: None,
                protocol_revision: "test".to_string(),
                target_revision: "test".to_string(),
            },
        },
        CaptureCase {
            id: "test/saw-a4".to_string(),
            kind: CaseKind::Stimulated,
            settings: vec![ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::Two,
                waveform: OscillatorWaveform::Saw,
            }],
            stimulus: Some(NoteStimulus {
                note,
                velocity: MidiVelocity::try_new(100).unwrap(),
            }),
            settle: zero,
            attack_discard: zero,
            capture: DurationSecs::try_new(0.05).unwrap(),
            post_note: zero,
            expected_fundamental_hz: Some(note.frequency_hz()),
            permitted_pitch_error_cents: pitch,
            role: ScientificRole::Training,
            tags: CaseTags {
                waveform: Some(OscillatorWaveform::Saw),
                note: Some(note),
                pulse_width: None,
                oscillator: Some(OscillatorId::Two),
                protocol_revision: "test".to_string(),
                target_revision: "test".to_string(),
            },
        },
    ]
}
