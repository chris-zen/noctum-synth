pub mod audio;
pub mod cli;
pub mod doctor;
pub mod domain;
pub mod events;
pub mod extraction;
pub mod midi;
pub mod project;
pub mod protocols;
pub mod runner;
pub mod targets;
pub mod terminal;
pub mod validation;

pub use doctor::{DoctorConfig, DoctorError, DoctorRecord, require_doctor_success, run_doctor};
pub use domain::{
    BipolarUnit, CaptureCase, CaseKind, CaseTags, DomainError, DurationSecs, EnvelopeSetting,
    FrequencyHz, MidiChannel, MidiNote, MidiVelocity, NoteStimulus, OscillatorId,
    OscillatorWaveform, ParameterSetting, PitchErrorCents, SampleRateHz, ScientificRole,
    UnitInterval,
};
pub use events::{CaptureEvent, CasePhase, NullReporter, Outcome, OutcomeStatus, Reporter};
pub use extraction::{
    CaptureExtractor, EXTRACTOR_ID, EXTRACTOR_REVISION, ExtractionError, ExtractionSummary,
    ExtractorDescriptor, OscillatorStaticExtractorV1, PitchExtraction, estimate_frequency,
    extract_pitch,
};
pub use midi::{
    FakeMidiTransport, MidiError, MidiTransport, TranscriptTransport, list_midi_output_names,
};
pub use project::{
    CaptureProject, CaseStatus, NewProjectRequest, ProjectDocument, ProjectError, ProjectState,
    StatusReport, VerifyReport,
};
pub use protocols::{
    CaptureProtocol, OSCILLATOR_STATIC_V1_ID, OSCILLATOR_STATIC_V1_REVISION, OscillatorStaticV1,
    ProtocolConfig, ProtocolDescriptor, ProtocolError, TargetCapabilities,
    scientific_role_for_note,
};
pub use targets::{
    AudioRequirements, OperatorConfirmer, OperatorSetupError, OperatorSetupStep, SettlePolicy,
    SkipOperatorConfirmer, StdinOperatorConfirmer, SynthTarget, TargetDescriptor, TargetError,
    arturia_prophet5_v1, confirm_target_setup, resolve_target,
};
pub use terminal::{ColorChoice, MemoryTerm, ReporterConfig, TerminalReporter};
