use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    domain::CaptureCase,
    protocols::{ProtocolConfig, ProtocolDescriptor},
    targets::{TargetDescriptor, resolve_target},
};

pub const PROJECT_SCHEMA_ID: &str = "synth-capture-project-v1";

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("project not found at {0}")]
    NotFound(PathBuf),
    #[error("unknown target `{0}`")]
    UnknownTarget(String),
    #[error("unknown protocol `{0}`")]
    UnknownProtocol(String),
    #[error("protocol error: {0}")]
    Protocol(#[from] crate::protocols::ProtocolError),
    #[error("case `{0}` not found")]
    CaseNotFound(String),
    #[error("refusing to overwrite completed audio for case `{0}`")]
    CompletedAudioOverwrite(String),
    #[error("scientific fingerprint mismatch (expected {expected}, found {found})")]
    FingerprintMismatch { expected: String, found: String },
    #[error("invalid project: {0}")]
    Invalid(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Pending,
    Recording,
    Validating,
    Complete,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseStateEntry {
    pub status: CaseStatus,
    pub attempts: u32,
    pub session_id: Option<String>,
    pub updated_at_unix_ms: u64,
    pub reason: Option<String>,
    pub audio_relpath: Option<String>,
    pub metadata_relpath: Option<String>,
    pub exact_frames: Option<u64>,
    pub wav_sha256: Option<String>,
    pub transcript_fingerprint: Option<String>,
    pub case_fingerprint: String,
    #[serde(default)]
    pub signal_metrics: Option<crate::validation::SignalMetrics>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectState {
    pub cases: BTreeMap<String, CaseStateEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDocument {
    pub schema_id: String,
    pub project_id: String,
    pub target: TargetDescriptor,
    pub protocol: ProtocolDescriptor,
    pub protocol_config: ProtocolConfig,
    pub cases: Vec<CaptureCase>,
    pub midi_port: String,
    pub audio_device: String,
    pub input_channel: u32,
    pub plugin_version: String,
    pub host_os: String,
    pub created_at_unix_ms: u64,
    pub scientific_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct CaptureProject {
    root: PathBuf,
    document: ProjectDocument,
    state: ProjectState,
}

#[derive(Clone, Debug)]
pub struct NewProjectRequest {
    pub root: PathBuf,
    pub target_id: String,
    pub protocol_id: String,
    pub midi_port: String,
    pub audio_device: String,
    pub input_channel: u32,
    pub sample_rate_hz: u32,
    pub plugin_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    pub project_id: String,
    pub root: String,
    pub scientific_fingerprint: String,
    pub total_cases: usize,
    pub pending: usize,
    pub recording: usize,
    pub validating: usize,
    pub complete: usize,
    pub failed: usize,
    pub interrupted: usize,
    pub current_case_id: Option<String>,
    pub last_completed_case_id: Option<String>,
    pub captured_ms: u64,
    pub estimated_remaining_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyIssue {
    pub case_id: Option<String>,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub ok: bool,
    pub issues: Vec<VerifyIssue>,
}

#[derive(Serialize)]
struct ScientificFingerprintMaterial<'a> {
    schema_id: &'a str,
    target: &'a TargetDescriptor,
    protocol: &'a ProtocolDescriptor,
    capture_order_seed: &'a str,
    target_revision: &'a str,
    sample_rate_hz: u32,
    midi_channel: u8,
    velocity: u8,
    settle_frames: u64,
    attack_discard_frames: u64,
    stimulated_capture_frames: u64,
    post_note_frames: u64,
    silence_frames: u64,
    permitted_pitch_error_cents_milli: u64,
    pulse_width_milli: u32,
    input_channel: u32,
    cases: Vec<CaseFingerprintMaterial>,
}

#[derive(Serialize)]
struct CaseFingerprintMaterial {
    id: String,
    kind: crate::domain::CaseKind,
    role: crate::domain::ScientificRole,
    note: Option<u8>,
    waveform: Option<crate::domain::OscillatorWaveform>,
    pulse_width_milli: Option<u32>,
    oscillator: Option<crate::domain::OscillatorId>,
    settings: Vec<SettingFingerprintMaterial>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SettingFingerprintMaterial {
    OscillatorWaveform {
        oscillator: crate::domain::OscillatorId,
        waveform: crate::domain::OscillatorWaveform,
    },
    OscillatorPulseWidth {
        oscillator: crate::domain::OscillatorId,
        normalized_milli: u32,
    },
    OscillatorLevel {
        oscillator: crate::domain::OscillatorId,
        normalized_milli: u32,
    },
    OscillatorTuneSemitones {
        oscillator: crate::domain::OscillatorId,
        semitones: i16,
    },
    OscillatorKeyboardTracking {
        oscillator: crate::domain::OscillatorId,
        enabled: bool,
    },
    OscillatorLowFrequencyMode {
        oscillator: crate::domain::OscillatorId,
        enabled: bool,
    },
    NoiseLevel {
        normalized_milli: u32,
    },
    FilterCutoffNormalized {
        normalized_milli: u32,
    },
    FilterResonance {
        normalized_milli: u32,
    },
    FilterEnvelopeAmount {
        bipolar_milli: i32,
    },
    AmplifierEnvelope {
        attack_milli: u32,
        decay_milli: u32,
        sustain_milli: u32,
        release_milli: u32,
    },
    FilterEnvelope {
        attack_milli: u32,
        decay_milli: u32,
        sustain_milli: u32,
        release_milli: u32,
    },
    UnisonEnabled {
        enabled: bool,
    },
    OscillatorSyncEnabled {
        enabled: bool,
    },
    VoiceDispersion {
        normalized_milli: u32,
    },
    MasterLevel {
        normalized_milli: u32,
    },
}

fn unit_milli(value: crate::domain::UnitInterval) -> u32 {
    (value.get() * 1000.0).round() as u32
}

fn bipolar_milli(value: crate::domain::BipolarUnit) -> i32 {
    (value.get() * 1000.0).round() as i32
}

fn setting_fingerprint_material(
    setting: &crate::domain::ParameterSetting,
) -> SettingFingerprintMaterial {
    use crate::domain::ParameterSetting;
    match setting {
        ParameterSetting::OscillatorWaveform {
            oscillator,
            waveform,
        } => SettingFingerprintMaterial::OscillatorWaveform {
            oscillator: *oscillator,
            waveform: *waveform,
        },
        ParameterSetting::OscillatorPulseWidth {
            oscillator,
            normalized,
        } => SettingFingerprintMaterial::OscillatorPulseWidth {
            oscillator: *oscillator,
            normalized_milli: unit_milli(*normalized),
        },
        ParameterSetting::OscillatorLevel {
            oscillator,
            normalized,
        } => SettingFingerprintMaterial::OscillatorLevel {
            oscillator: *oscillator,
            normalized_milli: unit_milli(*normalized),
        },
        ParameterSetting::OscillatorTuneSemitones {
            oscillator,
            semitones,
        } => SettingFingerprintMaterial::OscillatorTuneSemitones {
            oscillator: *oscillator,
            semitones: *semitones,
        },
        ParameterSetting::OscillatorKeyboardTracking {
            oscillator,
            enabled,
        } => SettingFingerprintMaterial::OscillatorKeyboardTracking {
            oscillator: *oscillator,
            enabled: *enabled,
        },
        ParameterSetting::OscillatorLowFrequencyMode {
            oscillator,
            enabled,
        } => SettingFingerprintMaterial::OscillatorLowFrequencyMode {
            oscillator: *oscillator,
            enabled: *enabled,
        },
        ParameterSetting::NoiseLevel(normalized) => SettingFingerprintMaterial::NoiseLevel {
            normalized_milli: unit_milli(*normalized),
        },
        ParameterSetting::FilterCutoffNormalized(normalized) => {
            SettingFingerprintMaterial::FilterCutoffNormalized {
                normalized_milli: unit_milli(*normalized),
            }
        }
        ParameterSetting::FilterResonance(normalized) => {
            SettingFingerprintMaterial::FilterResonance {
                normalized_milli: unit_milli(*normalized),
            }
        }
        ParameterSetting::FilterEnvelopeAmount(amount) => {
            SettingFingerprintMaterial::FilterEnvelopeAmount {
                bipolar_milli: bipolar_milli(*amount),
            }
        }
        ParameterSetting::AmplifierEnvelope(envelope) => {
            SettingFingerprintMaterial::AmplifierEnvelope {
                attack_milli: unit_milli(envelope.attack),
                decay_milli: unit_milli(envelope.decay),
                sustain_milli: unit_milli(envelope.sustain),
                release_milli: unit_milli(envelope.release),
            }
        }
        ParameterSetting::FilterEnvelope(envelope) => SettingFingerprintMaterial::FilterEnvelope {
            attack_milli: unit_milli(envelope.attack),
            decay_milli: unit_milli(envelope.decay),
            sustain_milli: unit_milli(envelope.sustain),
            release_milli: unit_milli(envelope.release),
        },
        ParameterSetting::UnisonEnabled(enabled) => {
            SettingFingerprintMaterial::UnisonEnabled { enabled: *enabled }
        }
        ParameterSetting::OscillatorSyncEnabled(enabled) => {
            SettingFingerprintMaterial::OscillatorSyncEnabled { enabled: *enabled }
        }
        ParameterSetting::VoiceDispersion(normalized) => {
            SettingFingerprintMaterial::VoiceDispersion {
                normalized_milli: unit_milli(*normalized),
            }
        }
        ParameterSetting::MasterLevel(normalized) => SettingFingerprintMaterial::MasterLevel {
            normalized_milli: unit_milli(*normalized),
        },
    }
}

fn scientific_fingerprint(document: &ProjectDocument) -> Result<String, ProjectError> {
    let config = &document.protocol_config;
    let sample_rate = config.sample_rate;
    let material = ScientificFingerprintMaterial {
        schema_id: &document.schema_id,
        target: &document.target,
        protocol: &document.protocol,
        capture_order_seed: &config.capture_order_seed,
        target_revision: &config.target_revision,
        sample_rate_hz: sample_rate.get(),
        midi_channel: config.midi_channel.get(),
        velocity: config.velocity.get(),
        settle_frames: config.settle.frames(sample_rate),
        attack_discard_frames: config.attack_discard.frames(sample_rate),
        stimulated_capture_frames: config.stimulated_capture.frames(sample_rate),
        post_note_frames: config.post_note.frames(sample_rate),
        silence_frames: config.silence_duration.frames(sample_rate),
        permitted_pitch_error_cents_milli: (config.permitted_pitch_error_cents.get() * 1000.0)
            .round() as u64,
        pulse_width_milli: (config.pulse_width.get() * 1000.0).round() as u32,
        input_channel: document.input_channel,
        cases: document
            .cases
            .iter()
            .map(|case| CaseFingerprintMaterial {
                id: case.id.clone(),
                kind: case.kind,
                role: case.role,
                note: case.tags.note.map(|note| note.get()),
                waveform: case.tags.waveform,
                pulse_width_milli: case
                    .tags
                    .pulse_width
                    .map(|width| (width.get() * 1000.0).round() as u32),
                oscillator: case.tags.oscillator,
                settings: case
                    .settings
                    .iter()
                    .map(setting_fingerprint_material)
                    .collect(),
            })
            .collect(),
    };
    Ok(sha256_bytes(&serde_json::to_vec(&material)?))
}

impl CaptureProject {
    pub fn create(request: NewProjectRequest) -> Result<Self, ProjectError> {
        let root = request.root;
        if root.join("project.json").exists() {
            return Err(ProjectError::AlreadyExists(root));
        }
        if root.exists() {
            let mut entries = fs::read_dir(&root)?;
            if entries.next().is_some() {
                return Err(ProjectError::Invalid(format!(
                    "project path {} exists and is not empty",
                    root.display()
                )));
            }
        }

        let target = resolve_target(&request.target_id)
            .ok_or_else(|| ProjectError::UnknownTarget(request.target_id.clone()))?;

        if request.protocol_id != "oscillator-static-v1" {
            return Err(ProjectError::UnknownProtocol(request.protocol_id));
        }
        if target.id == crate::targets::arturia_prophet5_v1::TARGET_ID
            && request.sample_rate_hz != 96_000
        {
            return Err(ProjectError::Invalid(
                "arturia-prophet5-v1 requires sample rate 96000".to_string(),
            ));
        }

        let protocol = crate::protocols::OscillatorStaticV1;
        let mut protocol_config =
            crate::protocols::OscillatorStaticV1::default_config(target.revision.clone())?;
        protocol_config.sample_rate = crate::domain::SampleRateHz::try_new(request.sample_rate_hz)
            .map_err(|err| ProjectError::Invalid(err.to_string()))?;

        let cases = crate::protocols::CaptureProtocol::build_cases(&protocol, &protocol_config)?;
        let protocol_descriptor = crate::protocols::CaptureProtocol::descriptor(&protocol);

        let mut document = ProjectDocument {
            schema_id: PROJECT_SCHEMA_ID.to_string(),
            project_id: root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project")
                .to_string(),
            target: target.clone(),
            protocol: protocol_descriptor,
            protocol_config,
            cases: cases.clone(),
            midi_port: request.midi_port,
            audio_device: request.audio_device,
            input_channel: request.input_channel,
            plugin_version: request.plugin_version,
            host_os: std::env::consts::OS.to_string(),
            created_at_unix_ms: unix_now_ms(),
            scientific_fingerprint: String::new(),
        };
        document.scientific_fingerprint = scientific_fingerprint(&document)?;

        let mut state_cases = BTreeMap::new();
        for case in &document.cases {
            state_cases.insert(
                case.id.clone(),
                CaseStateEntry {
                    status: CaseStatus::Pending,
                    attempts: 0,
                    session_id: None,
                    updated_at_unix_ms: document.created_at_unix_ms,
                    reason: None,
                    audio_relpath: None,
                    metadata_relpath: None,
                    exact_frames: None,
                    wav_sha256: None,
                    transcript_fingerprint: None,
                    case_fingerprint: case_fingerprint(case)?,
                    signal_metrics: None,
                },
            );
        }

        let staging = root.with_file_name(format!(
            ".{}.creating",
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project")
        ));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }

        let write_result = (|| -> Result<Self, ProjectError> {
            fs::create_dir_all(staging.join("sessions"))?;
            fs::create_dir_all(staging.join("audio"))?;
            fs::create_dir_all(staging.join("cases"))?;
            fs::create_dir_all(staging.join("incomplete"))?;
            fs::create_dir_all(staging.join("superseded"))?;
            fs::create_dir_all(staging.join("logs"))?;
            fs::create_dir_all(staging.join("derived"))?;

            let project = Self {
                root: staging.clone(),
                document,
                state: ProjectState { cases: state_cases },
            };
            atomic_write_json(&project.project_json_path(), &project.document)?;
            project.save_state()?;
            append_event(
                &project.events_path(),
                &serde_json::json!({
                    "event": "project_created",
                    "project_id": project.document.project_id,
                    "scientific_fingerprint": project.document.scientific_fingerprint,
                    "cases": project.document.cases.len(),
                }),
            )?;
            write_mapping_instructions(&project)?;
            Ok(project)
        })();

        match write_result {
            Ok(mut project) => {
                if let Some(parent) = root.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&staging, &root)?;
                project.root = root;
                Ok(project)
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&staging);
                Err(err)
            }
        }
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ProjectError> {
        let root = root.into();
        let project_path = root.join("project.json");
        if !project_path.exists() {
            return Err(ProjectError::NotFound(root));
        }
        let document: ProjectDocument = serde_json::from_str(&fs::read_to_string(project_path)?)?;
        let expected = scientific_fingerprint(&document)?;
        if expected != document.scientific_fingerprint {
            return Err(ProjectError::FingerprintMismatch {
                expected,
                found: document.scientific_fingerprint,
            });
        }
        let state: ProjectState =
            serde_json::from_str(&fs::read_to_string(root.join("state.json"))?)?;
        Ok(Self {
            root,
            document,
            state,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }

    pub fn state(&self) -> &ProjectState {
        &self.state
    }

    pub fn save_state(&self) -> Result<(), ProjectError> {
        atomic_write_json(&self.state_json_path(), &self.state)
    }

    pub fn status_report(&self) -> StatusReport {
        let mut pending = 0;
        let mut recording = 0;
        let mut validating = 0;
        let mut complete = 0;
        let mut failed = 0;
        let mut interrupted = 0;
        let mut current_case_id = None;
        let mut last_completed_case_id = None;
        let mut last_completed_ts = 0;
        let mut captured_ms = 0u64;
        let mut estimated_remaining_ms = 0u64;

        for case in &self.document.cases {
            let entry = self.state.cases.get(&case.id);
            let status = entry
                .map(|entry| entry.status)
                .unwrap_or(CaseStatus::Pending);
            if status == CaseStatus::Complete {
                captured_ms = captured_ms.saturating_add(case_capture_ms(case));
            } else {
                estimated_remaining_ms =
                    estimated_remaining_ms.saturating_add(case_wall_clock_ms(case));
            }
            match status {
                CaseStatus::Pending => pending += 1,
                CaseStatus::Recording => {
                    recording += 1;
                    current_case_id = Some(case.id.clone());
                }
                CaseStatus::Validating => {
                    validating += 1;
                    current_case_id = Some(case.id.clone());
                }
                CaseStatus::Complete => {
                    complete += 1;
                    if let Some(entry) = entry
                        && entry.updated_at_unix_ms >= last_completed_ts
                    {
                        last_completed_ts = entry.updated_at_unix_ms;
                        last_completed_case_id = Some(case.id.clone());
                    }
                }
                CaseStatus::Failed => failed += 1,
                CaseStatus::Interrupted => interrupted += 1,
            }
        }

        StatusReport {
            project_id: self.document.project_id.clone(),
            root: self.root.display().to_string(),
            scientific_fingerprint: self.document.scientific_fingerprint.clone(),
            total_cases: self.document.cases.len(),
            pending,
            recording,
            validating,
            complete,
            failed,
            interrupted,
            current_case_id,
            last_completed_case_id,
            captured_ms,
            estimated_remaining_ms,
        }
    }

    pub fn case_ids_with_status(&self, status: CaseStatus) -> Vec<String> {
        self.document
            .cases
            .iter()
            .filter(|case| {
                self.state
                    .cases
                    .get(&case.id)
                    .is_some_and(|entry| entry.status == status)
            })
            .map(|case| case.id.clone())
            .collect()
    }

    pub fn case_ids_with_session(&self, session_id: &str) -> Vec<String> {
        self.document
            .cases
            .iter()
            .filter(|case| {
                self.state.cases.get(&case.id).is_some_and(|entry| {
                    entry
                        .session_id
                        .as_deref()
                        .is_some_and(|value| value == session_id)
                })
            })
            .map(|case| case.id.clone())
            .collect()
    }

    pub fn case_ids_with_captured_progress(&self) -> Vec<String> {
        self.document
            .cases
            .iter()
            .filter(|case| {
                self.state.cases.get(&case.id).is_some_and(|entry| {
                    !matches!(entry.status, CaseStatus::Pending)
                        || entry.audio_relpath.is_some()
                        || self.final_audio_path(&case.id).exists()
                        || self.case_metadata_path(&case.id).exists()
                        || self.partial_audio_path(&case.id).exists()
                })
            })
            .map(|case| case.id.clone())
            .collect()
    }

    pub fn prepare_resume(&mut self) -> Result<Vec<String>, ProjectError> {
        let mut interrupted = Vec::new();
        for case in &self.document.cases {
            let Some(entry) = self.state.cases.get(&case.id) else {
                continue;
            };
            if matches!(entry.status, CaseStatus::Recording | CaseStatus::Validating) {
                interrupted.push(case.id.clone());
            }
        }

        let now = unix_now_ms();
        for case_id in &interrupted {
            self.archive_partial_locked(case_id)?;
            let entry = self
                .state
                .cases
                .get_mut(case_id)
                .ok_or_else(|| ProjectError::CaseNotFound(case_id.clone()))?;
            entry.status = CaseStatus::Interrupted;
            entry.reason = Some("stale in-flight state converted on resume".to_string());
            entry.updated_at_unix_ms = now;
        }
        self.save_state()?;
        Ok(interrupted)
    }

    pub fn archive_and_reset_case(&mut self, case_id: &str) -> Result<(), ProjectError> {
        let id = case_id.to_string();
        self.archive_and_reset_cases(std::slice::from_ref(&id))?;
        Ok(())
    }

    pub fn archive_and_reset_cases(&mut self, case_ids: &[String]) -> Result<String, ProjectError> {
        if case_ids.is_empty() {
            return Ok(String::new());
        }
        for case_id in case_ids {
            if !self.state.cases.contains_key(case_id) {
                return Err(ProjectError::CaseNotFound(case_id.clone()));
            }
        }

        let stamp = unix_now_ms().to_string();
        let dest_root = self.root.join("superseded").join(&stamp);
        fs::create_dir_all(&dest_root)?;

        for case_id in case_ids {
            let audio = self.final_audio_path(case_id);
            let meta = self.case_metadata_path(case_id);
            if audio.exists() {
                let dest = dest_root.join(format!("{}.wav", sanitize_case_filename(case_id)));
                fs::rename(&audio, dest)?;
            }
            if meta.exists() {
                let dest = dest_root.join(format!("{}.json", sanitize_case_filename(case_id)));
                fs::rename(&meta, dest)?;
            }
            self.archive_partial_locked(case_id)?;

            let case = self
                .document
                .cases
                .iter()
                .find(|case| case.id == *case_id)
                .ok_or_else(|| ProjectError::CaseNotFound(case_id.clone()))?;
            let fingerprint = case_fingerprint(case)?;
            let entry = self
                .state
                .cases
                .get_mut(case_id)
                .ok_or_else(|| ProjectError::CaseNotFound(case_id.clone()))?;
            *entry = CaseStateEntry {
                status: CaseStatus::Pending,
                attempts: 0,
                session_id: None,
                updated_at_unix_ms: unix_now_ms(),
                reason: Some(format!("archived under superseded/{stamp}")),
                audio_relpath: None,
                metadata_relpath: None,
                exact_frames: None,
                wav_sha256: None,
                transcript_fingerprint: None,
                case_fingerprint: fingerprint,
                signal_metrics: None,
            };
        }
        self.save_state()?;
        Ok(stamp)
    }

    pub fn commit_case_complete(
        &mut self,
        case_id: &str,
        session_id: &str,
        exact_frames: u64,
        wav_sha256: String,
        transcript_fingerprint: Option<String>,
        metrics: crate::validation::SignalMetrics,
    ) -> Result<(), ProjectError> {
        let audio_relpath = format!("audio/{case_id}.wav");
        let metadata_relpath = format!("cases/{case_id}.json");
        let entry = self
            .state
            .cases
            .get_mut(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        entry.status = CaseStatus::Complete;
        entry.session_id = Some(session_id.to_string());
        entry.updated_at_unix_ms = unix_now_ms();
        entry.reason = None;
        entry.audio_relpath = Some(audio_relpath);
        entry.metadata_relpath = Some(metadata_relpath);
        entry.exact_frames = Some(exact_frames);
        entry.wav_sha256 = Some(wav_sha256);
        entry.transcript_fingerprint = transcript_fingerprint;
        entry.signal_metrics = Some(metrics);
        self.save_state()
    }

    pub fn interrupt_in_flight_case(
        &mut self,
        case_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), ProjectError> {
        self.archive_partial_locked(case_id)?;
        let entry = self
            .state
            .cases
            .get_mut(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        entry.status = CaseStatus::Interrupted;
        entry.reason = Some(reason.into());
        entry.updated_at_unix_ms = unix_now_ms();
        self.save_state()
    }

    pub fn mark_failed(
        &mut self,
        case_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), ProjectError> {
        self.archive_partial_locked(case_id)?;
        let entry = self
            .state
            .cases
            .get_mut(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        entry.status = CaseStatus::Failed;
        entry.reason = Some(reason.into());
        entry.updated_at_unix_ms = unix_now_ms();
        self.save_state()
    }

    pub fn complete_take_is_resumable(&self, case_id: &str) -> Result<bool, ProjectError> {
        let case = self
            .document
            .cases
            .iter()
            .find(|case| case.id == case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        let entry = self
            .state
            .cases
            .get(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        if entry.status != CaseStatus::Complete {
            return Ok(false);
        }
        if entry.case_fingerprint != case_fingerprint(case)? {
            return Ok(false);
        }
        let Some(expected_hash) = &entry.wav_sha256 else {
            return Ok(false);
        };
        if entry.exact_frames.is_none() {
            return Ok(false);
        }
        let audio_path = self.final_audio_path(case_id);
        if !audio_path.exists() || &sha256_file(&audio_path)? != expected_hash {
            return Ok(false);
        }
        if !self.case_metadata_path(case_id).exists() {
            return Ok(false);
        }
        Ok(true)
    }

    pub fn verified_complete_count(&self) -> Result<usize, ProjectError> {
        let mut count = 0usize;
        for case in &self.document.cases {
            if self.complete_take_is_resumable(&case.id)? {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn create_with_cases(
        request: NewProjectRequest,
        cases: Vec<crate::domain::CaptureCase>,
    ) -> Result<Self, ProjectError> {
        let mut project = Self::create(request)?;
        let mut document = project.document.clone();
        document.cases = cases;
        document.scientific_fingerprint = scientific_fingerprint(&document)?;
        let mut state_cases = BTreeMap::new();
        for case in &document.cases {
            state_cases.insert(
                case.id.clone(),
                CaseStateEntry {
                    status: CaseStatus::Pending,
                    attempts: 0,
                    session_id: None,
                    updated_at_unix_ms: document.created_at_unix_ms,
                    reason: None,
                    audio_relpath: None,
                    metadata_relpath: None,
                    exact_frames: None,
                    wav_sha256: None,
                    transcript_fingerprint: None,
                    case_fingerprint: case_fingerprint(case)?,
                    signal_metrics: None,
                },
            );
        }
        project.document = document;
        project.state = ProjectState { cases: state_cases };
        atomic_write_json(&project.project_json_path(), &project.document)?;
        project.save_state()?;
        Ok(project)
    }

    pub fn assert_can_write_audio(&self, case_id: &str) -> Result<(), ProjectError> {
        let entry = self
            .state
            .cases
            .get(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        if entry.status == CaseStatus::Complete && self.final_audio_path(case_id).exists() {
            return Err(ProjectError::CompletedAudioOverwrite(case_id.to_string()));
        }
        Ok(())
    }

    pub fn mark_status(
        &mut self,
        case_id: &str,
        status: CaseStatus,
        reason: Option<String>,
    ) -> Result<(), ProjectError> {
        let entry = self
            .state
            .cases
            .get_mut(case_id)
            .ok_or_else(|| ProjectError::CaseNotFound(case_id.to_string()))?;
        entry.status = status;
        entry.reason = reason;
        entry.updated_at_unix_ms = unix_now_ms();
        if matches!(status, CaseStatus::Recording) {
            entry.attempts = entry.attempts.saturating_add(1);
        }
        self.save_state()
    }

    pub fn verify(&self) -> Result<VerifyReport, ProjectError> {
        let mut issues = Vec::new();
        let expected = scientific_fingerprint(&self.document)?;
        if expected != self.document.scientific_fingerprint {
            issues.push(VerifyIssue {
                case_id: None,
                kind: "fingerprint".to_string(),
                message: format!(
                    "scientific fingerprint mismatch: expected {expected}, found {}",
                    self.document.scientific_fingerprint
                ),
            });
        }

        for case in &self.document.cases {
            let Some(entry) = self.state.cases.get(&case.id) else {
                issues.push(VerifyIssue {
                    case_id: Some(case.id.clone()),
                    kind: "missing_state".to_string(),
                    message: "case missing from state.json".to_string(),
                });
                continue;
            };

            let expected_case_fp = case_fingerprint(case)?;
            if entry.case_fingerprint != expected_case_fp {
                issues.push(VerifyIssue {
                    case_id: Some(case.id.clone()),
                    kind: "case_fingerprint".to_string(),
                    message: "case fingerprint does not match project case definition".to_string(),
                });
            }

            let audio_path = self.final_audio_path(&case.id);
            let meta_path = self.case_metadata_path(&case.id);
            match entry.status {
                CaseStatus::Complete => {
                    if !audio_path.exists() {
                        issues.push(VerifyIssue {
                            case_id: Some(case.id.clone()),
                            kind: "missing_audio".to_string(),
                            message: "complete case is missing final WAV".to_string(),
                        });
                    } else if let Some(expected_hash) = &entry.wav_sha256 {
                        let actual = sha256_file(&audio_path)?;
                        if &actual != expected_hash {
                            issues.push(VerifyIssue {
                                case_id: Some(case.id.clone()),
                                kind: "corrupt_audio".to_string(),
                                message: format!(
                                    "WAV sha256 mismatch: expected {expected_hash}, found {actual}"
                                ),
                            });
                        }
                    }
                    if !meta_path.exists() {
                        issues.push(VerifyIssue {
                            case_id: Some(case.id.clone()),
                            kind: "missing_metadata".to_string(),
                            message: "complete case is missing metadata JSON".to_string(),
                        });
                    }
                }
                CaseStatus::Pending
                | CaseStatus::Failed
                | CaseStatus::Interrupted
                | CaseStatus::Recording
                | CaseStatus::Validating => {
                    if audio_path.exists() && entry.status != CaseStatus::Complete {
                        issues.push(VerifyIssue {
                            case_id: Some(case.id.clone()),
                            kind: "orphan_audio".to_string(),
                            message: format!("final WAV exists while status is {:?}", entry.status),
                        });
                    }
                }
            }
        }

        for path in walk_files(&self.root.join("audio"))? {
            if path.extension().and_then(|ext| ext.to_str()) != Some("wav") {
                continue;
            }
            let rel = path
                .strip_prefix(self.root.join("audio"))
                .unwrap_or(&path)
                .with_extension("");
            let case_id = rel.to_string_lossy().replace('\\', "/");
            if !self.state.cases.contains_key(&case_id) {
                issues.push(VerifyIssue {
                    case_id: Some(case_id),
                    kind: "orphan_audio".to_string(),
                    message: "WAV has no matching case in project".to_string(),
                });
            }
        }

        Ok(VerifyReport {
            ok: issues.is_empty(),
            issues,
        })
    }

    pub fn final_audio_path(&self, case_id: &str) -> PathBuf {
        self.root.join("audio").join(format!("{case_id}.wav"))
    }

    pub fn case_metadata_path(&self, case_id: &str) -> PathBuf {
        self.root.join("cases").join(format!("{case_id}.json"))
    }

    pub fn partial_audio_path(&self, case_id: &str) -> PathBuf {
        self.root
            .join("incomplete")
            .join(format!("{}.partial.wav", sanitize_case_filename(case_id)))
    }

    pub fn doctor_record_path(&self) -> PathBuf {
        self.root.join("doctor.json")
    }

    fn project_json_path(&self) -> PathBuf {
        self.root.join("project.json")
    }

    fn state_json_path(&self) -> PathBuf {
        self.root.join("state.json")
    }

    fn events_path(&self) -> PathBuf {
        self.root.join("logs").join("events.jsonl")
    }

    fn archive_partial_locked(&self, case_id: &str) -> Result<(), ProjectError> {
        let nested_partial = self
            .root
            .join("audio")
            .join(format!("{case_id}.partial.wav"));
        let dest = self.partial_audio_path(case_id);
        for path in [nested_partial, dest.clone()] {
            if path.exists() && path != dest {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(path, &dest)?;
            }
        }
        Ok(())
    }
}

fn write_mapping_instructions(project: &CaptureProject) -> Result<(), ProjectError> {
    let path = project.root.join("ARTURIA_MIDI_LEARN.txt");
    let body = format!(
        "Arturia Prophet-5 V absolute MIDI Learn\n\
         target: {}\n\
         adapter_revision: {}\n\
         mapping_fingerprint: {}\n\
         \n\
         Configure Prophet-5 V for absolute CC response and learn each control\n\
         listed by the adapter documentation. Do not change CC assignments after\n\
         project creation; a revised table requires a new project.\n\
         \n\
         Oscillator 2 Fine Tune, Pulse Width, and Filter Envelope Amount are not\n\
         MIDI-mapped (7-bit CC cannot center them). Set Fine Tune to 0.000, Pulse\n\
         Width to exactly 50%, and Filter Env Amount to exactly 5.0 when\n\
         doctor/run prompts at session start.\n",
        project.document.target.id,
        project.document.target.adapter_revision,
        project.document.target.mapping_fingerprint
    );
    atomic_write_bytes(&path, body.as_bytes())
}

fn case_capture_ms(case: &CaptureCase) -> u64 {
    (case.capture.get() * 1000.0).round() as u64
}

fn case_wall_clock_ms(case: &CaptureCase) -> u64 {
    ((case.settle.get() + case.attack_discard.get() + case.capture.get() + case.post_note.get())
        * 1000.0)
        .round() as u64
}

fn case_fingerprint(case: &CaptureCase) -> Result<String, ProjectError> {
    #[derive(Serialize)]
    struct Material<'a> {
        id: &'a str,
        kind: crate::domain::CaseKind,
        role: crate::domain::ScientificRole,
        note: Option<u8>,
        velocity: Option<u8>,
        waveform: Option<crate::domain::OscillatorWaveform>,
        pulse_width_milli: Option<u32>,
        oscillator: Option<crate::domain::OscillatorId>,
        settings: Vec<SettingFingerprintMaterial>,
        settle_ms: u64,
        attack_discard_ms: u64,
        capture_ms: u64,
        post_note_ms: u64,
        permitted_pitch_error_cents_milli: u64,
        protocol_revision: &'a str,
        target_revision: &'a str,
    }

    let material = Material {
        id: &case.id,
        kind: case.kind,
        role: case.role,
        note: case.tags.note.map(|note| note.get()),
        velocity: case
            .stimulus
            .as_ref()
            .map(|stimulus| stimulus.velocity.get()),
        waveform: case.tags.waveform,
        pulse_width_milli: case
            .tags
            .pulse_width
            .map(|width| (width.get() * 1000.0).round() as u32),
        oscillator: case.tags.oscillator,
        settings: case
            .settings
            .iter()
            .map(setting_fingerprint_material)
            .collect(),
        settle_ms: (case.settle.get() * 1000.0).round() as u64,
        attack_discard_ms: (case.attack_discard.get() * 1000.0).round() as u64,
        capture_ms: (case.capture.get() * 1000.0).round() as u64,
        post_note_ms: (case.post_note.get() * 1000.0).round() as u64,
        permitted_pitch_error_cents_milli: (case.permitted_pitch_error_cents.get() * 1000.0).round()
            as u64,
        protocol_revision: &case.tags.protocol_revision,
        target_revision: &case.tags.target_revision,
    };
    Ok(sha256_bytes(&serde_json::to_vec(&material)?))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

pub fn sha256_file(path: &Path) -> Result<String, ProjectError> {
    let bytes = fs::read(path)?;
    Ok(sha256_bytes(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sanitize_case_filename(case_id: &str) -> String {
    case_id.replace('/', "__")
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProjectError> {
    let pretty = serde_json::to_vec_pretty(value)?;
    atomic_write_bytes(path, &pretty)
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => format!("{name}.tmp"),
        None => "write.tmp".to_string(),
    };
    let tmp = path.with_file_name(tmp_name);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn append_event(path: &Path, event: &serde_json::Value) -> Result<(), ProjectError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::project::{
        CaptureProject, CaseStatus, NewProjectRequest, ProjectError, atomic_write_bytes,
    };

    #[test]
    fn create_open_and_status_round_trip() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("arturia-prophet5-v1");
        let project = CaptureProject::create(NewProjectRequest {
            root: root.clone(),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "Noctum Capture".to_string(),
            audio_device: "BlackHole 2ch".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "3.0.0".to_string(),
        })
        .unwrap();

        assert_eq!(project.document().cases.len(), 226);
        assert!(!project.document().scientific_fingerprint.is_empty());
        let status = project.status_report();
        assert_eq!(status.total_cases, 226);
        assert_eq!(status.pending, 226);

        let reopened = CaptureProject::open(&root).unwrap();
        assert_eq!(
            reopened.document().scientific_fingerprint,
            project.document().scientific_fingerprint
        );
    }

    #[test]
    fn fingerprint_changes_with_scientific_settings() {
        let dir = tempdir().unwrap();
        let a = CaptureProject::create(NewProjectRequest {
            root: dir.path().join("a"),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "port-a".to_string(),
            audio_device: "dev-a".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();
        let b = CaptureProject::create(NewProjectRequest {
            root: dir.path().join("b"),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "port-b".to_string(),
            audio_device: "dev-b".to_string(),
            input_channel: 1,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();
        assert_ne!(
            a.document().scientific_fingerprint,
            b.document().scientific_fingerprint
        );
        assert_eq!(
            a.document().scientific_fingerprint,
            CaptureProject::create(NewProjectRequest {
                root: dir.path().join("c"),
                target_id: "arturia-prophet5-v1".to_string(),
                protocol_id: "oscillator-static-v1".to_string(),
                midi_port: "other-port".to_string(),
                audio_device: "other-dev".to_string(),
                input_channel: 0,
                sample_rate_hz: 96_000,
                plugin_version: "9".to_string(),
            })
            .unwrap()
            .document()
            .scientific_fingerprint
        );
    }

    #[test]
    fn atomic_state_recovery_after_crashed_temp_write() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        let mut project = CaptureProject::create(NewProjectRequest {
            root: root.clone(),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();

        let case_id = project.document().cases[0].id.clone();
        project
            .mark_status(&case_id, CaseStatus::Recording, None)
            .unwrap();

        let tmp = root.join("state.json.tmp");
        atomic_write_bytes(&tmp, b"{not-json").unwrap();
        assert!(tmp.exists());

        let reopened = CaptureProject::open(&root).unwrap();
        assert_eq!(
            reopened.state().cases[&case_id].status,
            CaseStatus::Recording
        );
    }

    #[test]
    fn resume_converts_in_flight_and_retry_archives() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        let mut project = CaptureProject::create(NewProjectRequest {
            root: root.clone(),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();

        let case_id = project.document().cases[1].id.clone();
        project
            .mark_status(&case_id, CaseStatus::Validating, None)
            .unwrap();
        let partial = root.join("audio").join(format!("{case_id}.partial.wav"));
        if let Some(parent) = partial.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&partial, b"partial").unwrap();

        let interrupted = project.prepare_resume().unwrap();
        assert_eq!(interrupted, vec![case_id.clone()]);
        assert_eq!(
            project.state().cases[&case_id].status,
            CaseStatus::Interrupted
        );
        assert!(project.partial_audio_path(&case_id).exists());

        let final_audio = project.final_audio_path(&case_id);
        if let Some(parent) = final_audio.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&final_audio, b"complete-audio").unwrap();
        project
            .mark_status(&case_id, CaseStatus::Complete, None)
            .unwrap();
        assert!(matches!(
            project.assert_can_write_audio(&case_id),
            Err(ProjectError::CompletedAudioOverwrite(_))
        ));

        project.archive_and_reset_case(&case_id).unwrap();
        assert_eq!(project.state().cases[&case_id].status, CaseStatus::Pending);
        assert!(!final_audio.exists());
        assert!(project.assert_can_write_audio(&case_id).is_ok());
    }

    #[test]
    fn archive_and_reset_cases_batches_under_one_stamp() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        let mut project = CaptureProject::create(NewProjectRequest {
            root: root.clone(),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();

        let case_a = project.document().cases[1].id.clone();
        let case_b = project.document().cases[2].id.clone();
        for case_id in [&case_a, &case_b] {
            let audio = project.final_audio_path(case_id);
            if let Some(parent) = audio.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&audio, b"wav").unwrap();
            project
                .commit_case_complete(
                    case_id,
                    "session-batch",
                    10,
                    "deadbeef".to_string(),
                    None,
                    crate::validation::SignalMetrics {
                        rms: 0.1,
                        peak: 0.2,
                        dc: 0.0,
                        estimated_frequency_hz: Some(440.0),
                        clipping: false,
                        overflow: false,
                        dc_warning: false,
                    },
                )
                .unwrap();
        }

        assert_eq!(project.case_ids_with_session("session-batch").len(), 2);
        let stamp = project
            .archive_and_reset_cases(&project.case_ids_with_captured_progress())
            .unwrap();
        assert!(!stamp.is_empty());
        let archived = root.join("superseded").join(&stamp);
        assert!(archived.exists());
        assert_eq!(fs::read_dir(&archived).unwrap().count(), 2);
        assert_eq!(project.state().cases[&case_a].status, CaseStatus::Pending);
        assert_eq!(project.state().cases[&case_b].status, CaseStatus::Pending);
        assert!(project.case_ids_with_captured_progress().is_empty());
    }

    #[test]
    fn create_rejects_non_96k_for_arturia() {
        let dir = tempdir().unwrap();
        let err = CaptureProject::create(NewProjectRequest {
            root: dir.path().join("proj"),
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 48_000,
            plugin_version: "1".to_string(),
        })
        .unwrap_err();
        assert!(matches!(err, ProjectError::Invalid(_)));
    }

    #[test]
    fn verify_passes_on_fresh_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        let project = CaptureProject::create(NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();
        let report = project.verify().unwrap();
        assert!(report.ok, "{:?}", report.issues);
    }

    #[test]
    fn verify_detects_missing_complete_audio() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        let mut project = CaptureProject::create(NewProjectRequest {
            root,
            target_id: "arturia-prophet5-v1".to_string(),
            protocol_id: "oscillator-static-v1".to_string(),
            midi_port: "midi".to_string(),
            audio_device: "audio".to_string(),
            input_channel: 0,
            sample_rate_hz: 96_000,
            plugin_version: "1".to_string(),
        })
        .unwrap();
        let case_id = project.document().cases[0].id.clone();
        project
            .mark_status(&case_id, CaseStatus::Complete, None)
            .unwrap();
        let report = project.verify().unwrap();
        assert!(!report.ok);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == "missing_audio")
        );
    }
}
