use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use synth_capture::{
    audio::{
        StopFlag,
        fake::{FakeAudioInput, RenderEngine},
    },
    domain::{
        CaptureCase, CaseKind, CaseTags, DurationSecs, MidiChannel, MidiNote, MidiVelocity,
        NoteStimulus, OscillatorId, OscillatorWaveform, ParameterSetting, PitchErrorCents,
        ScientificRole,
    },
    events::{CaptureEvent, CasePhase, Outcome, OutcomeStatus, Reporter},
    midi::{FakeMidiTransport, TranscriptTransport},
    project::{CaptureProject, CaseStatus, NewProjectRequest},
    runner::{RunConfig, run_capture, run_capture_with_reporter},
    targets::{SkipOperatorConfirmer, fake_render::FakeRenderTarget},
    terminal::{ColorChoice, MemoryTerm, ReporterConfig, TerminalReporter},
};

#[test]
fn simulated_capture_completes_and_skips_on_resume() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sim");
    let cases = short_cases();
    let mut project = CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "test".to_string(),
        },
        cases,
    )
    .unwrap();

    let engine = Arc::new(Mutex::new(RenderEngine::new(96_000.0)));
    let mut target = FakeRenderTarget::new(MidiChannel::try_new(1).unwrap(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(Arc::clone(&engine));
    let stop = StopFlag::new();

    let summary = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s1".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.skipped, 0);
    assert_eq!(project.status_report().complete, 2);
    assert_eq!(project.verified_complete_count().unwrap(), 2);

    let saw = project
        .document()
        .cases
        .iter()
        .find(|case| case.tags.waveform == Some(OscillatorWaveform::Saw))
        .unwrap();
    assert_eq!(
        project.state().cases[&saw.id].exact_frames,
        Some(
            saw.capture
                .frames(project.document().protocol_config.sample_rate)
        )
    );
    assert!(project.final_audio_path(&saw.id).exists());
    assert!(!midi.entries().is_empty());

    let summary2 = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s2".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert_eq!(summary2.completed, 0);
    assert_eq!(summary2.skipped, 2);
}

#[test]
fn overflow_fails_case_without_overwriting_completed() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ovf");
    let cases = short_cases();
    let mut project = CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "test".to_string(),
        },
        cases,
    )
    .unwrap();

    let engine = Arc::new(Mutex::new(RenderEngine::new(96_000.0)));
    let mut target = FakeRenderTarget::new(MidiChannel::try_new(1).unwrap(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(Arc::clone(&engine));
    let stop = StopFlag::new();
    run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s1".to_string(),
            max_cases: Some(1),
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert_eq!(project.status_report().complete, 1);
    let first_id = project.document().cases[0].id.clone();
    assert!(project.assert_can_write_audio(&first_id).is_err());

    let mut audio = FakeAudioInput::with_forced_overflow(engine);
    let err = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s2".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    );
    assert!(err.is_err());
    assert_eq!(
        project.state().cases[&first_id].status,
        CaseStatus::Complete
    );
    let second_id = project.document().cases[1].id.clone();
    assert_eq!(project.state().cases[&second_id].status, CaseStatus::Failed);
}

#[test]
fn resume_rejects_complete_case_with_missing_metadata() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("meta");
    let cases = short_cases();
    let mut project = CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "test".to_string(),
        },
        cases,
    )
    .unwrap();

    let engine = Arc::new(Mutex::new(RenderEngine::new(96_000.0)));
    let mut target = FakeRenderTarget::new(MidiChannel::try_new(1).unwrap(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(Arc::clone(&engine));
    let stop = StopFlag::new();
    run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s1".to_string(),
            max_cases: Some(1),
            sleep_enabled: false,
        },
    )
    .unwrap();
    let first_id = project.document().cases[0].id.clone();
    std::fs::remove_file(project.case_metadata_path(&first_id)).unwrap();
    assert_eq!(project.status_report().complete, 1);
    assert_eq!(project.verified_complete_count().unwrap(), 0);
    let err = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s2".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("disagree") || err.to_string().contains("retry"));
}

#[test]
fn stop_flag_marks_case_interrupted() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("stop");
    let cases = short_cases();
    let mut project = CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "test".to_string(),
        },
        cases,
    )
    .unwrap();

    let engine = Arc::new(Mutex::new(RenderEngine::new(96_000.0)));
    let mut target = FakeRenderTarget::new(MidiChannel::try_new(1).unwrap(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let stop = StopFlag::new();
    let mut audio = StopAfterFirstCapture::new(FakeAudioInput::new(engine), stop.clone());

    let summary = run_capture(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s-stop".to_string(),
            max_cases: Some(1),
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert!(summary.interrupted);
    assert_eq!(summary.completed, 0);
    let first_id = &project.document().cases[0].id;
    assert_eq!(
        project.state().cases[first_id].status,
        CaseStatus::Interrupted
    );
}

#[test]
fn reporter_receives_phase_and_frame_events() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("events");
    let mut project = CaptureProject::create_with_cases(
        NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "fake".to_string(),
            audio_device: "fake".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "test".to_string(),
        },
        short_cases(),
    )
    .unwrap();

    let engine = Arc::new(Mutex::new(RenderEngine::new(96_000.0)));
    let mut target = FakeRenderTarget::new(MidiChannel::try_new(1).unwrap(), Arc::clone(&engine));
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    let mut audio = FakeAudioInput::new(Arc::clone(&engine));
    let stop = StopFlag::new();
    let mut recorder = RecordingReporter::default();

    let mut confirmer = SkipOperatorConfirmer;
    run_capture_with_reporter(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s1".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
        &mut confirmer,
        &mut recorder,
    )
    .unwrap();

    assert!(matches!(
        recorder.events.first(),
        Some(CaptureEvent::SessionStarted {
            complete_cases: 0,
            total_cases: 2,
            ..
        })
    ));
    let saw_id = "test/saw-a4";
    let phases: Vec<CasePhase> = recorder
        .events
        .iter()
        .filter_map(|event| match event {
            CaptureEvent::CasePhaseChanged { case_id, phase } if case_id == saw_id => Some(*phase),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            CasePhase::Reset,
            CasePhase::Settle,
            CasePhase::Discard,
            CasePhase::Record,
            CasePhase::Validate,
            CasePhase::Commit,
        ]
    );
    let expected_frames = project
        .document()
        .cases
        .iter()
        .find(|case| case.id == saw_id)
        .unwrap()
        .capture
        .frames(project.document().protocol_config.sample_rate);
    let last_progress = recorder
        .events
        .iter()
        .filter_map(|event| match event {
            CaptureEvent::CaseProgress { case_id, frames } if case_id == saw_id => Some(*frames),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(last_progress, expected_frames);
    assert_eq!(
        recorder
            .events
            .iter()
            .filter(|event| matches!(event, CaptureEvent::CaseCompleted { .. }))
            .count(),
        2
    );

    let term = MemoryTerm::new(120, 20);
    let mut reporter = TerminalReporter::memory(
        &ReporterConfig {
            color: ColorChoice::Never,
            json: false,
            interactive: true,
            no_color_env: false,
            sample_rate_hz: 96_000,
        },
        term.clone(),
    );
    let mut confirmer = SkipOperatorConfirmer;
    let summary = run_capture_with_reporter(
        &mut project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig {
            session_id: "s2".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
        &mut confirmer,
        &mut reporter,
    )
    .unwrap();
    assert_eq!(summary.skipped, 2);
    assert_eq!(reporter.overall_position(), Some(2));
    let without_cursor = term
        .written()
        .replace("\u{1b}[?25l", "")
        .replace("\u{1b}[?25h", "");
    assert!(
        !without_cursor.contains('\u{1b}'),
        "unexpected ANSI (excluding cursor): {without_cursor:?}"
    );
    reporter.finish(&Outcome::new(
        OutcomeStatus::Success,
        "resume verified",
        std::time::Duration::from_secs(1),
    ));
    assert_eq!(reporter.active_bars(), 0);
    assert!(!term.cursor_hidden());
    assert!(term.frame().trim().is_empty());
}

#[derive(Default)]
struct RecordingReporter {
    events: Vec<CaptureEvent>,
}

impl Reporter for RecordingReporter {
    fn event(&mut self, event: &CaptureEvent) {
        self.events.push(event.clone());
    }

    fn finish(&mut self, _outcome: &Outcome) {}
}

struct StopAfterFirstCapture {
    inner: FakeAudioInput,
    stop: StopFlag,
    saw_capture: bool,
}

impl StopAfterFirstCapture {
    fn new(inner: FakeAudioInput, stop: StopFlag) -> Self {
        Self {
            inner,
            stop,
            saw_capture: false,
        }
    }
}

impl synth_capture::audio::AudioInput for StopAfterFirstCapture {
    fn format(&self) -> synth_capture::audio::AudioFormat {
        self.inner.format()
    }

    fn drain_frames(
        &mut self,
        frame_count: usize,
        dest: &mut Vec<f32>,
    ) -> Result<(), synth_capture::audio::AudioError> {
        self.inner.drain_frames(frame_count, dest)?;
        if frame_count > 0 && !self.saw_capture {
            self.saw_capture = true;
            self.stop.request_stop();
        }
        Ok(())
    }

    fn health(&self) -> synth_capture::audio::AudioHealth {
        self.inner.health()
    }

    fn reset_health(&mut self) {
        self.inner.reset_health();
    }
}

fn short_cases() -> Vec<CaptureCase> {
    let settle = DurationSecs::try_new(0.0).unwrap();
    let discard = DurationSecs::try_new(0.0).unwrap();
    let capture = DurationSecs::try_new(0.05).unwrap();
    let post = DurationSecs::try_new(0.0).unwrap();
    let pitch = PitchErrorCents::try_new(50.0).unwrap();
    let note = MidiNote::try_new(69).unwrap();
    vec![
        CaptureCase {
            id: "test/silence".to_string(),
            kind: CaseKind::Silence,
            settings: Vec::new(),
            stimulus: None,
            settle,
            attack_discard: discard,
            capture: DurationSecs::try_new(0.02).unwrap(),
            post_note: post,
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
            settle,
            attack_discard: discard,
            capture,
            post_note: post,
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
