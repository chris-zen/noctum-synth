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
    extraction::{CaptureExtractor, OscillatorStaticExtractorV1},
    midi::{FakeMidiTransport, TranscriptTransport},
    project::{CaptureProject, NewProjectRequest},
    runner::{RunConfig, run_capture},
    targets::fake_render::FakeRenderTarget,
};

#[test]
fn extract_writes_npz_for_completed_fake_project() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("extract-smoke");
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
        extract_cases(),
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
            session_id: "extract-s1".to_string(),
            max_cases: None,
            sleep_enabled: false,
        },
    )
    .unwrap();
    assert_eq!(summary.completed, 3);
    assert!(project.verify().unwrap().ok);

    let output = project.root().join("derived");
    let result = OscillatorStaticExtractorV1
        .extract(&project, &output)
        .unwrap();
    assert_eq!(result.note_count, 2);
    assert_eq!(result.waveform_count, 2);
    assert!(output.join("saw-cycles-v1.npz").is_file());
    assert!(output.join("saw-summary-v1.json").is_file());
    assert!(output.join("triangle-cycles-v1.npz").is_file());
    assert!(output.join("triangle-summary-v1.json").is_file());
    assert!(!output.join("pulse50-cycles-v1.npz").exists());

    let saw_summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output.join("saw-summary-v1.json")).unwrap())
            .unwrap();
    assert_eq!(saw_summary["extractor_revision"], 1);
    assert_eq!(saw_summary["pitches"].as_array().unwrap().len(), 1);
    assert!(saw_summary["npz_sha256"].as_str().unwrap().len() == 64);
}

#[test]
fn extract_refuses_incomplete_project() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("incomplete");
    let project = CaptureProject::create_with_cases(
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
        extract_cases(),
    )
    .unwrap();

    let err = OscillatorStaticExtractorV1
        .extract(&project, &project.root().join("derived"))
        .unwrap_err();
    assert!(err.to_string().contains("incomplete") || err.to_string().contains("Pending"));
}

fn extract_cases() -> Vec<CaptureCase> {
    let settle = DurationSecs::try_new(0.0).unwrap();
    let discard = DurationSecs::try_new(0.0).unwrap();
    let capture = DurationSecs::try_new(1.0).unwrap();
    let post = DurationSecs::try_new(0.0).unwrap();
    let pitch = PitchErrorCents::try_new(50.0).unwrap();
    let note = MidiNote::try_new(60).unwrap();
    let mut cases = vec![CaptureCase {
        id: "extract/silence".to_string(),
        kind: CaseKind::Silence,
        settings: Vec::new(),
        stimulus: None,
        settle,
        attack_discard: discard,
        capture: DurationSecs::try_new(0.05).unwrap(),
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
    }];
    for (slug, waveform) in [
        ("saw", OscillatorWaveform::Saw),
        ("triangle", OscillatorWaveform::Triangle),
    ] {
        cases.push(CaptureCase {
            id: format!("extract/{slug}/midi-060"),
            kind: CaseKind::Stimulated,
            settings: vec![ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::Two,
                waveform,
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
                waveform: Some(waveform),
                note: Some(note),
                pulse_width: None,
                oscillator: Some(OscillatorId::Two),
                protocol_revision: "test".to_string(),
                target_revision: "test".to_string(),
            },
        });
    }
    cases
}
