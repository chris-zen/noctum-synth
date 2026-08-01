use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{project::CaptureProject, protocols::ProtocolDescriptor};

pub mod oscillator_static_v1;
pub mod wav_reader;

pub use oscillator_static_v1::{
    EXTRACTOR_ID, EXTRACTOR_REVISION, HARMONICS, MAX_CYCLES, OscillatorStaticExtractorV1,
    PHASE_BINS, PitchExtraction, estimate_frequency, extract_pitch,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractorDescriptor {
    pub id: String,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractionSummary {
    pub project_id: String,
    pub output_dir: PathBuf,
    pub waveform_count: usize,
    pub note_count: usize,
    pub files: Vec<PathBuf>,
    pub extractor_fingerprint: String,
    pub scientific_fingerprint: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("unsupported protocol `{0}`")]
    UnsupportedProtocol(String),
    #[error("project incomplete: {0}")]
    Incomplete(String),
    #[error("WAV checksum mismatch for `{case_id}`: expected {expected}, found {found}")]
    ChecksumMismatch {
        case_id: String,
        expected: String,
        found: String,
    },
    #[error("case `{case_id}`: {message}")]
    Case { case_id: String, message: String },
    #[error("wav error: {0}")]
    Wav(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("{0}")]
    Message(String),
}

pub trait CaptureExtractor {
    fn descriptor(&self) -> ExtractorDescriptor;
    fn supports(&self, protocol: &ProtocolDescriptor) -> bool;
    fn extract(
        &self,
        project: &CaptureProject,
        output: &Path,
    ) -> Result<ExtractionSummary, ExtractionError>;
}
