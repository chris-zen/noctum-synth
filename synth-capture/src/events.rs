use std::time::Duration;

use crate::domain::{CaptureCase, CaseKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasePhase {
    Reset,
    Settle,
    Discard,
    Record,
    Validate,
    Commit,
}

impl CasePhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Settle => "settle",
            Self::Discard => "discard",
            Self::Record => "record",
            Self::Validate => "validate",
            Self::Commit => "commit",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CaptureEvent {
    SessionStarted {
        project_id: String,
        total_cases: usize,
        complete_cases: usize,
    },
    CaseStarted {
        case_id: String,
        label: String,
        capture_frames: u64,
    },
    CasePhaseChanged {
        case_id: String,
        phase: CasePhase,
    },
    CaseProgress {
        case_id: String,
        frames: u64,
    },
    CaseCompleted {
        case_id: String,
    },
    CaseSkipped {
        case_id: String,
    },
    CaseFailed {
        case_id: String,
        reason: String,
    },
    CaseInterrupted {
        case_id: String,
        reason: String,
    },
    DoctorStarted {
        probe_count: usize,
    },
    DoctorProbeStarted {
        label: String,
    },
    DoctorProbePassed {
        label: String,
        detail: String,
    },
    DoctorProbeFailed {
        label: String,
        reason: String,
    },
    DoctorFinished {
        ok: bool,
    },
    Info {
        message: String,
    },
    Warning {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeStatus {
    Success,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub status: OutcomeStatus,
    pub headline: String,
    pub details: Vec<String>,
    pub elapsed: Duration,
}

impl Outcome {
    pub fn new(status: OutcomeStatus, headline: impl Into<String>, elapsed: Duration) -> Self {
        Self {
            status,
            headline: headline.into(),
            details: Vec::new(),
            elapsed,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }
}

pub trait Reporter {
    fn event(&mut self, event: &CaptureEvent);
    fn finish(&mut self, outcome: &Outcome);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NullReporter;

impl Reporter for NullReporter {
    fn event(&mut self, _event: &CaptureEvent) {}

    fn finish(&mut self, _outcome: &Outcome) {}
}

pub fn case_label(case: &CaptureCase) -> String {
    match case.kind {
        CaseKind::Silence => "silence".to_string(),
        CaseKind::Stimulated => match (case.tags.waveform, case.tags.note) {
            (Some(waveform), Some(note)) => format!("{waveform}  MIDI {}", note.get()),
            (Some(waveform), None) => waveform.to_string(),
            (None, Some(note)) => format!("MIDI {}", note.get()),
            (None, None) => case.id.clone(),
        },
    }
}
