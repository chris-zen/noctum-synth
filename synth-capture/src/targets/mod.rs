use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    domain::{DurationSecs, MidiChannel, MidiNote, MidiVelocity, ParameterSetting},
    midi::MidiTransport,
    project::sha256_bytes,
    protocols::TargetCapabilities,
};

pub mod arturia_prophet5_v1;
pub mod fake_render;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    pub id: String,
    pub revision: String,
    pub adapter_revision: String,
    pub mapping_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettlePolicy {
    pub reset_settle: DurationSecs,
    pub parameter_settle: DurationSecs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRequirements {
    pub required_sample_rate_hz: Option<u32>,
    pub require_native_float32: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorSetupStep {
    pub id: String,
    pub title: String,
    pub instructions: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OperatorSetupError {
    #[error("operator setup aborted")]
    Aborted,
    #[error("operator setup failed: {0}")]
    Message(String),
}

pub trait OperatorConfirmer {
    fn confirm_setup(&mut self, steps: &[OperatorSetupStep]) -> Result<(), OperatorSetupError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SkipOperatorConfirmer;

impl OperatorConfirmer for SkipOperatorConfirmer {
    fn confirm_setup(&mut self, _steps: &[OperatorSetupStep]) -> Result<(), OperatorSetupError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdinOperatorConfirmer;

impl OperatorConfirmer for StdinOperatorConfirmer {
    fn confirm_setup(&mut self, steps: &[OperatorSetupStep]) -> Result<(), OperatorSetupError> {
        if steps.is_empty() {
            return Ok(());
        }
        eprintln!("Operator setup required (once per session):");
        for (index, step) in steps.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, step.title);
            for line in step.instructions.lines() {
                eprintln!("     {line}");
            }
        }
        eprint!("Press Enter when done (or type abort to cancel): ");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|err| OperatorSetupError::Message(err.to_string()))?;
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("abort")
            || trimmed.eq_ignore_ascii_case("q")
            || trimmed.eq_ignore_ascii_case("quit")
        {
            return Err(OperatorSetupError::Aborted);
        }
        Ok(())
    }
}

pub fn confirm_target_setup<T: SynthTarget>(
    target: &T,
    confirmer: &mut dyn OperatorConfirmer,
) -> Result<(), OperatorSetupError> {
    let steps = target.operator_setup_steps();
    if steps.is_empty() {
        return Ok(());
    }
    confirmer.confirm_setup(&steps)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TargetError {
    #[error("unsupported parameter: {0}")]
    UnsupportedParameter(&'static str),
    #[error("midi error: {0}")]
    Midi(#[from] crate::midi::MidiError),
    #[error("domain error: {0}")]
    Domain(#[from] crate::domain::DomainError),
    #[error("target error: {0}")]
    Message(String),
}

pub trait SynthTarget {
    fn descriptor(&self) -> TargetDescriptor;
    fn capabilities(&self) -> TargetCapabilities;
    fn audio_requirements(&self) -> AudioRequirements;
    fn operator_setup_steps(&self) -> Vec<OperatorSetupStep> {
        Vec::new()
    }
    fn reset(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError>;
    fn set_parameter(
        &mut self,
        midi: &mut dyn MidiTransport,
        setting: &ParameterSetting,
    ) -> Result<(), TargetError>;
    fn note_on(
        &mut self,
        midi: &mut dyn MidiTransport,
        note: MidiNote,
        velocity: MidiVelocity,
    ) -> Result<(), TargetError>;
    fn note_off(&mut self, midi: &mut dyn MidiTransport, note: MidiNote)
    -> Result<(), TargetError>;
    fn panic(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError>;
    fn prepare_session(&mut self, midi: &mut dyn MidiTransport) -> Result<(), TargetError> {
        self.panic(midi)
    }
    fn settle_policy(&self) -> SettlePolicy;
}

pub fn resolve_target(target_id: &str) -> Option<TargetDescriptor> {
    match target_id {
        arturia_prophet5_v1::TARGET_ID => Some(arturia_prophet5_v1::descriptor()),
        fake_render::TARGET_ID => Some(fake_render::descriptor()),
        _ => None,
    }
}

pub fn fingerprint_mapping_table(rows: &[(u8, &str, u8)]) -> String {
    let mut lines = Vec::with_capacity(rows.len());
    for (cc, semantic, neutral) in rows {
        lines.push(format!("{cc}\t{semantic}\t{neutral}"));
    }
    lines.sort();
    sha256_bytes(lines.join("\n").as_bytes())
}

pub(crate) fn cc_status(channel: MidiChannel) -> u8 {
    0xB0 | (channel.get() - 1)
}

pub(crate) fn note_on_status(channel: MidiChannel) -> u8 {
    0x90 | (channel.get() - 1)
}

pub(crate) fn note_off_status(channel: MidiChannel) -> u8 {
    0x80 | (channel.get() - 1)
}

pub(crate) fn send_cc(
    midi: &mut dyn MidiTransport,
    channel: MidiChannel,
    controller: u8,
    value: u8,
) -> Result<(), TargetError> {
    midi.send(&[cc_status(channel), controller, value])?;
    Ok(())
}

pub(crate) fn unit_to_cc(value: crate::domain::UnitInterval) -> u8 {
    (value.get() * 127.0).round().clamp(0.0, 127.0) as u8
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{
        audio::fake::RenderEngine,
        domain::MidiChannel,
        targets::{
            OperatorConfirmer, OperatorSetupError, OperatorSetupStep, SkipOperatorConfirmer,
            SynthTarget, arturia_prophet5_v1::ArturiaProphet5V1, confirm_target_setup,
            fake_render::FakeRenderTarget,
        },
    };

    struct RecordingConfirmer {
        seen: Vec<String>,
        abort: bool,
    }

    impl OperatorConfirmer for RecordingConfirmer {
        fn confirm_setup(&mut self, steps: &[OperatorSetupStep]) -> Result<(), OperatorSetupError> {
            self.seen = steps.iter().map(|step| step.id.clone()).collect();
            if self.abort {
                Err(OperatorSetupError::Aborted)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn confirm_target_setup_skips_when_no_steps() {
        let target = FakeRenderTarget::new(
            MidiChannel::try_new(1).unwrap(),
            Arc::new(Mutex::new(RenderEngine::new(96_000.0))),
        );
        let mut confirmer = RecordingConfirmer {
            seen: Vec::new(),
            abort: false,
        };
        confirm_target_setup(&target, &mut confirmer).unwrap();
        assert!(confirmer.seen.is_empty());
        let mut skip = SkipOperatorConfirmer;
        confirm_target_setup(&target, &mut skip).unwrap();
    }

    #[test]
    fn confirm_target_setup_asks_once_for_target_steps() {
        let target = ArturiaProphet5V1::new(MidiChannel::try_new(1).unwrap());
        assert!(!target.operator_setup_steps().is_empty());
        let mut confirmer = RecordingConfirmer {
            seen: Vec::new(),
            abort: false,
        };
        confirm_target_setup(&target, &mut confirmer).unwrap();
        assert_eq!(
            confirmer.seen,
            vec![
                "osc2_fine_tune_zero".to_string(),
                "osc2_pulse_width_50".to_string(),
                "filter_env_amount_center".to_string(),
            ]
        );
        let mut aborting = RecordingConfirmer {
            seen: Vec::new(),
            abort: true,
        };
        assert_eq!(
            confirm_target_setup(&target, &mut aborting),
            Err(OperatorSetupError::Aborted)
        );
    }
}
