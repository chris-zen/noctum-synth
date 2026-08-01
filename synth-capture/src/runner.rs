use std::{
    fs,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use serde::Serialize;
use thiserror::Error;

use crate::{
    audio::{
        AudioError, AudioInput, StopFlag,
        wav::{read_float_wav, write_float_wav},
    },
    domain::{CaptureCase, CaseKind, SampleRateHz},
    events::{CaptureEvent, CasePhase, NullReporter, Reporter, case_label},
    midi::{MidiError, MidiTransport, TranscriptTransport},
    project::{CaptureProject, CaseStatus, ProjectError, atomic_write_bytes, sha256_file},
    targets::{
        OperatorConfirmer, OperatorSetupError, SkipOperatorConfirmer, SynthTarget, TargetError,
        confirm_target_setup,
    },
    validation::{ValidationError, ValidationInput, validate_take},
};

const PROGRESS_CHUNK_FRAMES: usize = 4_800;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error(transparent)]
    Midi(#[from] MidiError),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("stopped by operator")]
    Stopped,
    #[error("case `{0}` failed validation twice")]
    RepeatedValidationFailure(String),
    #[error("runner error: {0}")]
    Message(String),
    #[error(transparent)]
    OperatorSetup(#[from] OperatorSetupError),
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub session_id: String,
    pub max_cases: Option<usize>,
    pub sleep_enabled: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            session_id: format!("session-{}", unix_ms()),
            max_cases: None,
            sleep_enabled: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub interrupted: bool,
}

pub fn run_capture<T, M, A>(
    project: &mut CaptureProject,
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    config: RunConfig,
) -> Result<RunSummary, RunnerError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    let mut reporter = NullReporter;
    let mut confirmer = SkipOperatorConfirmer;
    run_capture_with_reporter(
        project,
        target,
        midi,
        audio,
        stop,
        config,
        &mut confirmer,
        &mut reporter,
    )
}

pub fn run_capture_with_reporter<T, M, A>(
    project: &mut CaptureProject,
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    config: RunConfig,
    confirmer: &mut dyn OperatorConfirmer,
    reporter: &mut dyn Reporter,
) -> Result<RunSummary, RunnerError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    project.prepare_resume()?;
    target.prepare_session(midi)?;
    confirm_target_setup(target, confirmer).map_err(|err| match err {
        OperatorSetupError::Aborted => RunnerError::Stopped,
        other => RunnerError::OperatorSetup(other),
    })?;
    let mut completed = 0usize;
    let mut skipped = 0usize;
    let failed = 0usize;
    let sample_rate = project.document().protocol_config.sample_rate;
    let cases: Vec<CaptureCase> = project.document().cases.clone();
    let limit = config.max_cases.unwrap_or(cases.len());

    reporter.event(&CaptureEvent::SessionStarted {
        project_id: project.document().project_id.clone(),
        total_cases: cases.len(),
        complete_cases: project.verified_complete_count()?,
    });

    for case in cases.into_iter().take(limit) {
        if stop.is_stopped() {
            return Ok(RunSummary {
                completed,
                skipped,
                failed,
                interrupted: true,
            });
        }

        let entry = project
            .state()
            .cases
            .get(&case.id)
            .ok_or_else(|| ProjectError::CaseNotFound(case.id.clone()))?;
        if entry.status == CaseStatus::Complete {
            if project.complete_take_is_resumable(&case.id)? {
                skipped += 1;
                reporter.event(&CaptureEvent::CaseSkipped {
                    case_id: case.id.clone(),
                });
                continue;
            }
            return Err(RunnerError::Message(format!(
                "case `{}` is marked complete but WAV/metadata/checksum disagree; use retry",
                case.id
            )));
        }

        project.assert_can_write_audio(&case.id)?;
        reporter.event(&CaptureEvent::CaseStarted {
            case_id: case.id.clone(),
            label: case_label(&case),
            capture_frames: case.capture.frames(sample_rate),
        });
        match capture_one_case(
            project,
            target,
            midi,
            audio,
            stop,
            &case,
            sample_rate,
            &config,
            reporter,
        ) {
            Ok(()) => {
                completed += 1;
                reporter.event(&CaptureEvent::CaseCompleted {
                    case_id: case.id.clone(),
                });
            }
            Err(RunnerError::Stopped) => {
                let _ = project.interrupt_in_flight_case(&case.id, "stopped by operator");
                reporter.event(&CaptureEvent::CaseInterrupted {
                    case_id: case.id.clone(),
                    reason: "stopped by operator".to_string(),
                });
                return Ok(RunSummary {
                    completed,
                    skipped,
                    failed,
                    interrupted: true,
                });
            }
            Err(RunnerError::Validation(first)) => {
                reporter.event(&CaptureEvent::Warning {
                    message: format!("{} failed validation ({first}); retrying once", case.id),
                });
                match capture_one_case(
                    project,
                    target,
                    midi,
                    audio,
                    stop,
                    &case,
                    sample_rate,
                    &config,
                    reporter,
                ) {
                    Ok(()) => {
                        completed += 1;
                        reporter.event(&CaptureEvent::CaseCompleted {
                            case_id: case.id.clone(),
                        });
                    }
                    Err(RunnerError::Stopped) => {
                        let _ = project.interrupt_in_flight_case(&case.id, "stopped by operator");
                        reporter.event(&CaptureEvent::CaseInterrupted {
                            case_id: case.id.clone(),
                            reason: "stopped by operator".to_string(),
                        });
                        return Ok(RunSummary {
                            completed,
                            skipped,
                            failed,
                            interrupted: true,
                        });
                    }
                    Err(err) => {
                        project.mark_status(&case.id, CaseStatus::Failed, Some(err.to_string()))?;
                        reporter.event(&CaptureEvent::CaseFailed {
                            case_id: case.id.clone(),
                            reason: err.to_string(),
                        });
                        return Err(RunnerError::RepeatedValidationFailure(case.id));
                    }
                }
            }
            Err(RunnerError::Audio(err @ AudioError::Overflow { .. }))
            | Err(RunnerError::Audio(err @ AudioError::Callback { .. })) => {
                let reason = err.to_string();
                let _ = project.mark_failed(&case.id, &reason);
                reporter.event(&CaptureEvent::CaseFailed {
                    case_id: case.id.clone(),
                    reason,
                });
                return Err(RunnerError::Audio(err));
            }
            Err(err) => {
                let _ = project.interrupt_in_flight_case(&case.id, err.to_string());
                reporter.event(&CaptureEvent::CaseInterrupted {
                    case_id: case.id.clone(),
                    reason: err.to_string(),
                });
                return Err(err);
            }
        }
    }

    Ok(RunSummary {
        completed,
        skipped,
        failed,
        interrupted: false,
    })
}

fn capture_one_case<T, M, A>(
    project: &mut CaptureProject,
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    case: &CaptureCase,
    sample_rate: SampleRateHz,
    config: &RunConfig,
    reporter: &mut dyn Reporter,
) -> Result<(), RunnerError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    midi.clear_entries();
    project.mark_status(&case.id, CaseStatus::Recording, None)?;
    audio.reset_health();

    let result = capture_one_case_body(
        project,
        target,
        midi,
        audio,
        stop,
        case,
        sample_rate,
        config,
        reporter,
    );
    if result.is_err() {
        let _ = target.panic(midi);
    }
    result
}

fn capture_one_case_body<T, M, A>(
    project: &mut CaptureProject,
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    case: &CaptureCase,
    sample_rate: SampleRateHz,
    config: &RunConfig,
    reporter: &mut dyn Reporter,
) -> Result<(), RunnerError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Reset,
    });
    target.panic(midi)?;
    target.reset(midi)?;
    for setting in &case.settings {
        target.set_parameter(midi, setting)?;
    }

    let settle_frames = case.settle.frames(sample_rate) as usize;
    let discard_frames = case.attack_discard.frames(sample_rate) as usize;
    let capture_frames = case.capture.frames(sample_rate) as usize;
    let post_frames = case.post_note.frames(sample_rate) as usize;

    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Settle,
    });
    if settle_frames > 0 {
        drain(audio, settle_frames, stop)?;
    } else {
        maybe_sleep(
            config.sleep_enabled,
            target.settle_policy().reset_settle.get(),
        );
    }

    if let Some(stimulus) = &case.stimulus {
        target.note_on(midi, stimulus.note, stimulus.velocity)?;
    }
    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Discard,
    });
    drain(audio, discard_frames, stop)?;

    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Record,
    });
    let samples = record_exact_frames(audio, capture_frames, &case.id, reporter)?;
    if stop.is_stopped() {
        project.interrupt_in_flight_case(&case.id, "stopped by operator")?;
        return Err(RunnerError::Stopped);
    }
    if let Some(err) = AudioError::from_health(&audio.health()) {
        return Err(RunnerError::Audio(err));
    }

    if let Some(stimulus) = &case.stimulus {
        target.note_off(midi, stimulus.note)?;
    }
    target.panic(midi)?;
    drain(audio, post_frames, stop)?;

    let partial = project.partial_audio_path(&case.id);
    write_float_wav(&partial, sample_rate.get(), &samples)?;

    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Validate,
    });
    project.mark_status(&case.id, CaseStatus::Validating, None)?;
    let (_rate, loaded) = read_float_wav(&partial)?;
    let metrics = validate_take(ValidationInput {
        samples: &loaded,
        kind: case.kind,
        expected_frames: capture_frames as u64,
        expected_fundamental_hz: case.expected_fundamental_hz,
        permitted_pitch_error_cents: case.permitted_pitch_error_cents,
        sample_rate_hz: sample_rate.get(),
        overflow: !audio.health().is_clean(),
    })?;

    reporter.event(&CaptureEvent::CasePhaseChanged {
        case_id: case.id.clone(),
        phase: CasePhase::Commit,
    });
    let transcript_fingerprint = midi.fingerprint();
    let metadata = CaseMetadata {
        case_id: case.id.clone(),
        kind: case.kind,
        exact_frames: capture_frames as u64,
        sample_rate_hz: sample_rate.get(),
        metrics: metrics.clone(),
        transcript_fingerprint: transcript_fingerprint.clone(),
    };
    let meta_path = project.case_metadata_path(&case.id);
    if let Some(parent) = meta_path.parent() {
        fs::create_dir_all(parent).map_err(ProjectError::Io)?;
    }
    atomic_write_bytes(
        &meta_path,
        &serde_json::to_vec_pretty(&metadata).map_err(ProjectError::Json)?,
    )?;

    let final_path = project.final_audio_path(&case.id);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).map_err(ProjectError::Io)?;
    }
    if final_path.exists() {
        return Err(RunnerError::Project(ProjectError::CompletedAudioOverwrite(
            case.id.clone(),
        )));
    }
    fs::rename(&partial, &final_path).map_err(ProjectError::Io)?;
    let wav_sha256 = sha256_file(&final_path)?;
    project.commit_case_complete(
        &case.id,
        &config.session_id,
        capture_frames as u64,
        wav_sha256,
        Some(transcript_fingerprint),
        metrics,
    )?;
    Ok(())
}

fn record_exact_frames<A: AudioInput>(
    audio: &mut A,
    capture_frames: usize,
    case_id: &str,
    reporter: &mut dyn Reporter,
) -> Result<Vec<f32>, RunnerError> {
    let mut samples = Vec::with_capacity(capture_frames);
    let mut chunk = Vec::with_capacity(PROGRESS_CHUNK_FRAMES);
    while samples.len() < capture_frames {
        let wanted = PROGRESS_CHUNK_FRAMES.min(capture_frames - samples.len());
        audio.drain_frames(wanted, &mut chunk)?;
        if chunk.len() != wanted {
            return Err(RunnerError::Audio(AudioError::Underrun {
                expected: wanted,
                got: chunk.len(),
            }));
        }
        samples.append(&mut chunk);
        reporter.event(&CaptureEvent::CaseProgress {
            case_id: case_id.to_string(),
            frames: samples.len() as u64,
        });
    }
    Ok(samples)
}

fn drain<A: AudioInput>(audio: &mut A, frames: usize, stop: &StopFlag) -> Result<(), RunnerError> {
    if frames == 0 {
        return Ok(());
    }
    if stop.is_stopped() {
        return Err(RunnerError::Stopped);
    }
    let mut sink = Vec::new();
    audio.drain_frames(frames, &mut sink)?;
    Ok(())
}

pub(crate) fn maybe_sleep(enabled: bool, seconds: f64) {
    if enabled && seconds > 0.0 {
        std::thread::sleep(Duration::from_secs_f64(seconds));
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Serialize)]
struct CaseMetadata {
    case_id: String,
    kind: CaseKind,
    exact_frames: u64,
    sample_rate_hz: u32,
    metrics: crate::validation::SignalMetrics,
    transcript_fingerprint: String,
}

pub fn install_ctrlc_flag() -> Result<StopFlag, RunnerError> {
    let flag = StopFlag::new();
    let handle = flag.handle();
    ctrlc::set_handler(move || {
        handle.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .map_err(|err| RunnerError::Message(err.to_string()))?;
    Ok(flag)
}

pub fn stop_flag_from_arc(flag: Arc<AtomicBool>) -> StopFlag {
    StopFlag::from_arc(flag)
}
