use std::fs;

use num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    audio::{AudioError, AudioFormat, AudioInput, StopFlag},
    domain::{
        CaseKind, DurationSecs, MidiNote, MidiVelocity, OscillatorId, OscillatorWaveform,
        ParameterSetting, PitchErrorCents, SampleRateHz, UnitInterval,
    },
    events::{CaptureEvent, Reporter},
    midi::{MidiError, MidiTransport, TranscriptTransport},
    project::{CaptureProject, ProjectError, atomic_write_bytes},
    protocols::ProtocolDescriptor,
    runner::maybe_sleep,
    targets::{
        AudioRequirements, OperatorConfirmer, OperatorSetupError, SynthTarget, TargetDescriptor,
        TargetError, confirm_target_setup, resolve_target,
    },
    validation::{SignalMetrics, ValidationInput, validate_take},
};

pub const DOCTOR_SCHEMA_ID: &str = "synth-capture-doctor-v2";
pub const DOCTOR_PROBE_NOTES: [u8; 3] = [48, 64, 80];
pub const HARMONIC_COUNT: usize = 8;
pub const MIN_SPECTRAL_DISTANCE: f64 = 0.03;
pub const MIN_PITCH_COHERENCE: f64 = 0.90;

const SILENCE_LABEL: &str = "silence";

#[derive(Debug, Error)]
pub enum DoctorError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error(transparent)]
    Midi(#[from] MidiError),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error("unsupported audio format: {0}")]
    AudioFormat(String),
    #[error("probe `{probe}` failed: {reason}")]
    Probe { probe: String, reason: String },
    #[error(
        "probes `{left}` and `{right}` are not spectrally distinct (distance {distance:.4} < {threshold:.4})"
    )]
    NotDistinct {
        left: String,
        right: String,
        distance: f64,
        threshold: f64,
    },
    #[error(
        "probes `{left}` and `{right}` are not pitch-coherent (spectral cosine {similarity:.4} < {threshold:.4})"
    )]
    NotCoherent {
        left: String,
        right: String,
        similarity: f64,
        threshold: f64,
    },
    #[error("no doctor record at {0}; run `synth-capture doctor --project <path>` first")]
    MissingRecord(String),
    #[error("stored doctor record is not compatible: {0}")]
    Incompatible(String),
    #[error("stopped by operator")]
    Stopped,
    #[error(transparent)]
    OperatorSetup(#[from] OperatorSetupError),
}

#[derive(Clone, Debug)]
pub struct DoctorConfig {
    pub probe_duration: DurationSecs,
    pub sleep_enabled: bool,
}

impl Default for DoctorConfig {
    fn default() -> Self {
        Self {
            probe_duration: DurationSecs::try_new(0.5).expect("0.5 is valid"),
            sleep_enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorRecord {
    pub schema_id: String,
    pub ok: bool,
    pub created_at_unix_ms: u64,
    pub project_id: String,
    pub scientific_fingerprint: String,
    pub target: TargetDescriptor,
    pub protocol: ProtocolDescriptor,
    pub midi_port: String,
    pub audio_device: String,
    pub audio_format: AudioFormat,
    pub probe_frames: u64,
    pub target_settle_secs: f64,
    pub probes: Vec<DoctorProbe>,
    pub distinctness: Vec<DoctorDistinctness>,
    pub coherence: Vec<DoctorCoherence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorProbe {
    pub label: String,
    pub waveform: Option<OscillatorWaveform>,
    pub note: Option<u8>,
    pub metrics: SignalMetrics,
    pub pitch_error_cents: Option<f64>,
    pub harmonics: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorDistinctness {
    pub left: String,
    pub right: String,
    pub distance: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorCoherence {
    pub left: String,
    pub right: String,
    pub similarity: f64,
}

pub fn run_doctor<T, M, A>(
    project: &CaptureProject,
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    config: &DoctorConfig,
    confirmer: &mut dyn OperatorConfirmer,
    reporter: &mut dyn Reporter,
) -> Result<DoctorRecord, DoctorError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    let document = project.document();
    let descriptor = target.descriptor();
    if descriptor != document.target {
        return Err(DoctorError::Incompatible(format!(
            "target adapter `{}` revision `{}` does not match project target `{}` revision `{}`",
            descriptor.id,
            descriptor.adapter_revision,
            document.target.id,
            document.target.adapter_revision
        )));
    }
    let protocol_config = &document.protocol_config;
    let sample_rate = protocol_config.sample_rate;
    let format = audio.format();
    check_audio_format(
        &format,
        &target.audio_requirements(),
        sample_rate,
        document.input_channel,
    )?;
    target.prepare_session(midi)?;
    confirm_target_setup(target, confirmer).map_err(|err| match err {
        OperatorSetupError::Aborted => DoctorError::Stopped,
        other => DoctorError::OperatorSetup(other),
    })?;

    let plans = probe_plans();
    reporter.event(&CaptureEvent::DoctorStarted {
        probe_count: plans.len(),
    });

    let timing = ProbeTiming {
        settle_frames: protocol_config.settle.frames(sample_rate) as usize,
        discard_frames: protocol_config.attack_discard.frames(sample_rate) as usize,
        probe_frames: config.probe_duration.frames(sample_rate) as usize,
        post_frames: protocol_config.post_note.frames(sample_rate) as usize,
        settle_secs: target.settle_policy().reset_settle.get(),
        sleep_enabled: config.sleep_enabled,
        velocity: protocol_config.velocity,
        pulse_width: protocol_config.pulse_width,
    };

    let mut probes = Vec::with_capacity(plans.len());
    for plan in &plans {
        reporter.event(&CaptureEvent::DoctorProbeStarted {
            label: plan.label.to_string(),
        });
        let probe = match capture_and_evaluate_probe(
            target,
            midi,
            audio,
            stop,
            plan,
            &timing,
            sample_rate,
            protocol_config.permitted_pitch_error_cents,
        ) {
            Ok(probe) => probe,
            Err(err) => {
                reporter.event(&CaptureEvent::DoctorProbeFailed {
                    label: plan.label.to_string(),
                    reason: err.to_string(),
                });
                reporter.event(&CaptureEvent::DoctorFinished { ok: false });
                return Err(err);
            }
        };
        reporter.event(&CaptureEvent::DoctorProbePassed {
            label: plan.label.to_string(),
            detail: probe_detail(&probe),
        });
        probes.push(probe);
    }

    let distinctness = match spectral_distinctness(&probes) {
        Ok(distinctness) => distinctness,
        Err(err) => {
            reporter.event(&CaptureEvent::DoctorProbeFailed {
                label: "distinctness".to_string(),
                reason: err.to_string(),
            });
            reporter.event(&CaptureEvent::DoctorFinished { ok: false });
            return Err(err);
        }
    };
    let coherence = match pitch_coherence(&probes) {
        Ok(coherence) => coherence,
        Err(err) => {
            reporter.event(&CaptureEvent::DoctorProbeFailed {
                label: "pitch-coherence".to_string(),
                reason: err.to_string(),
            });
            reporter.event(&CaptureEvent::DoctorFinished { ok: false });
            return Err(err);
        }
    };
    reporter.event(&CaptureEvent::DoctorFinished { ok: true });

    Ok(DoctorRecord {
        schema_id: DOCTOR_SCHEMA_ID.to_string(),
        ok: true,
        created_at_unix_ms: unix_now_ms(),
        project_id: document.project_id.clone(),
        scientific_fingerprint: document.scientific_fingerprint.clone(),
        target: descriptor,
        protocol: document.protocol.clone(),
        midi_port: document.midi_port.clone(),
        audio_device: document.audio_device.clone(),
        audio_format: format,
        probe_frames: timing.probe_frames as u64,
        target_settle_secs: timing.settle_secs,
        probes,
        distinctness,
        coherence,
    })
}

pub fn write_doctor_record(
    project: &CaptureProject,
    record: &DoctorRecord,
) -> Result<(), DoctorError> {
    let bytes = serde_json::to_vec_pretty(record).map_err(ProjectError::Json)?;
    atomic_write_bytes(&project.doctor_record_path(), &bytes)?;
    Ok(())
}

pub fn read_doctor_record(project: &CaptureProject) -> Result<DoctorRecord, DoctorError> {
    let path = project.doctor_record_path();
    if !path.exists() {
        return Err(DoctorError::MissingRecord(path.display().to_string()));
    }
    let text = fs::read_to_string(&path).map_err(ProjectError::Io)?;
    let record: DoctorRecord = serde_json::from_str(&text)
        .map_err(|err| DoctorError::Incompatible(format!("unreadable doctor record: {err}")))?;
    Ok(record)
}

pub fn require_doctor_success(project: &CaptureProject) -> Result<DoctorRecord, DoctorError> {
    let record = read_doctor_record(project)?;
    let document = project.document();
    if record.schema_id != DOCTOR_SCHEMA_ID {
        return Err(DoctorError::Incompatible(format!(
            "schema `{}` is not `{DOCTOR_SCHEMA_ID}`",
            record.schema_id
        )));
    }
    if !record.ok {
        return Err(DoctorError::Incompatible(
            "last doctor run did not succeed".to_string(),
        ));
    }
    if record.scientific_fingerprint != document.scientific_fingerprint {
        return Err(DoctorError::Incompatible(
            "doctor ran against a different scientific configuration".to_string(),
        ));
    }
    if record.target != document.target {
        return Err(DoctorError::Incompatible(
            "doctor target descriptor does not match the project target".to_string(),
        ));
    }
    let live = resolve_target(&document.target.id).ok_or_else(|| {
        DoctorError::Incompatible(format!("unknown target `{}`", document.target.id))
    })?;
    if live != record.target {
        return Err(DoctorError::Incompatible(
            "target adapter revision or MIDI mapping changed since doctor ran".to_string(),
        ));
    }
    if record.protocol != document.protocol {
        return Err(DoctorError::Incompatible(
            "doctor protocol revision does not match the project".to_string(),
        ));
    }
    if record.midi_port != document.midi_port || record.audio_device != document.audio_device {
        return Err(DoctorError::Incompatible(
            "doctor used different MIDI or audio devices".to_string(),
        ));
    }
    if record.audio_format.sample_rate_hz != document.protocol_config.sample_rate.get()
        || record.audio_format.input_channel != document.input_channel
    {
        return Err(DoctorError::Incompatible(
            "doctor audio format does not match the project capture settings".to_string(),
        ));
    }
    Ok(record)
}

pub fn check_audio_format(
    format: &AudioFormat,
    requirements: &AudioRequirements,
    project_sample_rate: SampleRateHz,
    project_input_channel: u32,
) -> Result<(), DoctorError> {
    if format.sample_rate_hz != project_sample_rate.get() {
        return Err(DoctorError::AudioFormat(format!(
            "input runs at {} Hz but the project requires {} Hz",
            format.sample_rate_hz,
            project_sample_rate.get()
        )));
    }
    if let Some(required) = requirements.required_sample_rate_hz
        && format.sample_rate_hz != required
    {
        return Err(DoctorError::AudioFormat(format!(
            "target requires exactly {required} Hz but the input runs at {} Hz",
            format.sample_rate_hz
        )));
    }
    if requirements.require_native_float32 && !format.native_float32 {
        return Err(DoctorError::AudioFormat(
            "target requires a native float32 input".to_string(),
        ));
    }
    if format.input_channel != project_input_channel {
        return Err(DoctorError::AudioFormat(format!(
            "input channel {} does not match the project channel {project_input_channel}",
            format.input_channel
        )));
    }
    if u32::from(format.channels) <= format.input_channel {
        return Err(DoctorError::AudioFormat(format!(
            "input channel {} is out of range for {} channel(s)",
            format.input_channel, format.channels
        )));
    }
    Ok(())
}

pub fn probe_plans() -> Vec<ProbePlan> {
    let mut plans = vec![ProbePlan {
        label: SILENCE_LABEL,
        waveform: None,
        note: None,
    }];
    for (waveform, labels) in [
        (OscillatorWaveform::Saw, ["saw-48", "saw-64", "saw-80"]),
        (
            OscillatorWaveform::Triangle,
            ["triangle-48", "triangle-64", "triangle-80"],
        ),
        (
            OscillatorWaveform::Pulse,
            ["pulse-48", "pulse-64", "pulse-80"],
        ),
    ] {
        for (note, label) in DOCTOR_PROBE_NOTES.into_iter().zip(labels) {
            plans.push(ProbePlan {
                label,
                waveform: Some(waveform),
                note: Some(MidiNote::try_new(note).expect("doctor note is valid")),
            });
        }
    }
    plans
}

#[derive(Clone, Copy, Debug)]
pub struct ProbePlan {
    pub label: &'static str,
    pub waveform: Option<OscillatorWaveform>,
    pub note: Option<MidiNote>,
}

#[derive(Clone, Copy, Debug)]
struct ProbeTiming {
    settle_frames: usize,
    discard_frames: usize,
    probe_frames: usize,
    post_frames: usize,
    settle_secs: f64,
    sleep_enabled: bool,
    velocity: MidiVelocity,
    pulse_width: UnitInterval,
}

#[allow(clippy::too_many_arguments)]
fn capture_and_evaluate_probe<T, M, A>(
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    plan: &ProbePlan,
    timing: &ProbeTiming,
    sample_rate: SampleRateHz,
    permitted_pitch_error_cents: PitchErrorCents,
) -> Result<DoctorProbe, DoctorError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    let samples = capture_probe(target, midi, audio, stop, plan, timing)?;
    let overflow = !audio.health().is_clean();
    let expected = plan
        .note
        .map(|note| note.frequency_hz())
        .filter(|_| plan.waveform.is_some());
    let kind = if plan.waveform.is_some() {
        CaseKind::Stimulated
    } else {
        CaseKind::Silence
    };
    let metrics = validate_take(ValidationInput {
        samples: &samples,
        kind,
        expected_frames: timing.probe_frames as u64,
        expected_fundamental_hz: expected,
        permitted_pitch_error_cents,
        sample_rate_hz: sample_rate.get(),
        overflow,
    })
    .map_err(|err| DoctorError::Probe {
        probe: plan.label.to_string(),
        reason: err.to_string(),
    })?;

    let pitch_error_cents = match (expected, metrics.estimated_frequency_hz) {
        (Some(expected), Some(measured)) => Some(1200.0 * (measured / expected.get()).log2()),
        _ => None,
    };
    let analysis_frequency_hz = metrics
        .estimated_frequency_hz
        .or_else(|| expected.map(|frequency| frequency.get()));
    let harmonics = analysis_frequency_hz
        .map(|frequency| harmonic_profile(&samples, sample_rate.get(), frequency))
        .unwrap_or_default();

    Ok(DoctorProbe {
        label: plan.label.to_string(),
        waveform: plan.waveform,
        note: plan.note.map(|note| note.get()),
        metrics,
        pitch_error_cents,
        harmonics,
    })
}

fn capture_probe<T, M, A>(
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    plan: &ProbePlan,
    timing: &ProbeTiming,
) -> Result<Vec<f32>, DoctorError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    if stop.is_stopped() {
        return Err(DoctorError::Stopped);
    }
    audio.reset_health();
    let result = capture_probe_body(target, midi, audio, stop, plan, timing);
    if result.is_err() {
        let _ = target.panic(midi);
    }
    result
}

fn capture_probe_body<T, M, A>(
    target: &mut T,
    midi: &mut TranscriptTransport<M>,
    audio: &mut A,
    stop: &StopFlag,
    plan: &ProbePlan,
    timing: &ProbeTiming,
) -> Result<Vec<f32>, DoctorError>
where
    T: SynthTarget,
    M: MidiTransport,
    A: AudioInput,
{
    target.panic(midi)?;
    target.reset(midi)?;
    if let Some(waveform) = plan.waveform {
        target.set_parameter(
            midi,
            &ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::Two,
                waveform,
            },
        )?;
        if waveform == OscillatorWaveform::Pulse {
            target.set_parameter(
                midi,
                &ParameterSetting::OscillatorPulseWidth {
                    oscillator: OscillatorId::Two,
                    normalized: timing.pulse_width,
                },
            )?;
        }
    }
    settle(audio, timing)?;

    if let Some(note) = plan.note.filter(|_| plan.waveform.is_some()) {
        target.note_on(midi, note, timing.velocity)?;
    }
    discard(audio, timing.discard_frames)?;

    let mut samples = Vec::new();
    audio.drain_frames(timing.probe_frames, &mut samples)?;

    if let Some(note) = plan.note.filter(|_| plan.waveform.is_some()) {
        target.note_off(midi, note)?;
    }
    target.panic(midi)?;
    discard(audio, timing.post_frames)?;
    if stop.is_stopped() {
        return Err(DoctorError::Stopped);
    }
    Ok(samples)
}

fn settle<A: AudioInput>(audio: &mut A, timing: &ProbeTiming) -> Result<(), DoctorError> {
    if timing.settle_frames > 0 {
        discard(audio, timing.settle_frames)?;
        return Ok(());
    }
    maybe_sleep(timing.sleep_enabled, timing.settle_secs);
    Ok(())
}

fn spectral_distinctness(probes: &[DoctorProbe]) -> Result<Vec<DoctorDistinctness>, DoctorError> {
    let stimulated: Vec<&DoctorProbe> = probes
        .iter()
        .filter(|probe| probe.waveform.is_some())
        .collect();
    let mut distinctness = Vec::new();
    for (index, left) in stimulated.iter().enumerate() {
        for right in stimulated.iter().skip(index + 1) {
            if left.note != right.note || left.waveform == right.waveform {
                continue;
            }
            let distance = spectral_distance(&left.harmonics, &right.harmonics);
            distinctness.push(DoctorDistinctness {
                left: left.label.clone(),
                right: right.label.clone(),
                distance,
            });
            if distance < MIN_SPECTRAL_DISTANCE {
                return Err(DoctorError::NotDistinct {
                    left: left.label.clone(),
                    right: right.label.clone(),
                    distance,
                    threshold: MIN_SPECTRAL_DISTANCE,
                });
            }
        }
    }
    Ok(distinctness)
}

fn pitch_coherence(probes: &[DoctorProbe]) -> Result<Vec<DoctorCoherence>, DoctorError> {
    let mut coherence = Vec::new();
    for waveform in [
        OscillatorWaveform::Saw,
        OscillatorWaveform::Triangle,
        OscillatorWaveform::Pulse,
    ] {
        let matching: Vec<&DoctorProbe> = probes
            .iter()
            .filter(|probe| probe.waveform == Some(waveform))
            .collect();
        for pair in matching.windows(2) {
            let similarity = spectral_cosine(&pair[0].harmonics, &pair[1].harmonics);
            coherence.push(DoctorCoherence {
                left: pair[0].label.clone(),
                right: pair[1].label.clone(),
                similarity,
            });
            if similarity < MIN_PITCH_COHERENCE {
                return Err(DoctorError::NotCoherent {
                    left: pair[0].label.clone(),
                    right: pair[1].label.clone(),
                    similarity,
                    threshold: MIN_PITCH_COHERENCE,
                });
            }
        }
    }
    Ok(coherence)
}

fn spectral_cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    let left_norm = left
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    let right_norm = right
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    if left_norm <= 1e-20 || right_norm <= 1e-20 {
        0.0
    } else {
        (dot / (left_norm * right_norm)).clamp(0.0, 1.0)
    }
}

pub fn spectral_distance(left: &[f32], right: &[f32]) -> f64 {
    if left.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    let sum: f64 = left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| {
            let delta = f64::from(*a) - f64::from(*b);
            delta * delta
        })
        .sum();
    sum.sqrt()
}

pub fn harmonic_profile(samples: &[f32], sample_rate_hz: u32, fundamental_hz: f64) -> Vec<f32> {
    let mut size = 1usize;
    while size * 2 <= samples.len().min(32_768) {
        size *= 2;
    }
    if size < 256 || fundamental_hz <= 0.0 {
        return Vec::new();
    }
    let mut buffer: Vec<Complex<f32>> = samples
        .iter()
        .take(size)
        .enumerate()
        .map(|(index, sample)| {
            let window =
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / (size as f32 - 1.0)).cos();
            Complex {
                re: sample * window,
                im: 0.0,
            }
        })
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    planner.plan_fft_forward(size).process(&mut buffer);

    let bin_hz = f64::from(sample_rate_hz) / size as f64;
    let mut magnitudes = Vec::with_capacity(HARMONIC_COUNT);
    for harmonic in 1..=HARMONIC_COUNT {
        let target = fundamental_hz * harmonic as f64;
        let center = (target / bin_hz).round() as isize;
        if center < 1 || center + 1 >= (size as isize) / 2 {
            magnitudes.push(0.0);
            continue;
        }
        let peak = ((center - 1)..=(center + 1))
            .map(|bin| buffer[bin as usize].norm())
            .fold(0.0f32, f32::max);
        magnitudes.push(peak);
    }
    let norm = magnitudes
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm <= 0.0 {
        return magnitudes;
    }
    magnitudes
        .into_iter()
        .map(|value| (f64::from(value) / norm) as f32)
        .collect()
}

fn probe_detail(probe: &DoctorProbe) -> String {
    let frequency = probe
        .metrics
        .estimated_frequency_hz
        .map(|value| format!("{value:.2} Hz"))
        .unwrap_or_else(|| "n/a".to_string());
    let cents = probe
        .pitch_error_cents
        .map(|value| format!("{value:+.1} cents"))
        .unwrap_or_else(|| "n/a".to_string());
    format!(
        "rms {:.5} peak {:.5} dc {:+.5} freq {frequency} pitch {cents} clip {} overflow {}",
        probe.metrics.rms,
        probe.metrics.peak,
        probe.metrics.dc,
        probe.metrics.clipping,
        probe.metrics.overflow
    )
}

fn discard<A: AudioInput>(audio: &mut A, frames: usize) -> Result<(), DoctorError> {
    if frames == 0 {
        return Ok(());
    }
    let mut sink = Vec::new();
    audio.drain_frames(frames, &mut sink)?;
    Ok(())
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use crate::{
        audio::AudioFormat,
        doctor::{
            DoctorError, MIN_SPECTRAL_DISTANCE, check_audio_format, harmonic_profile,
            spectral_distance,
        },
        domain::SampleRateHz,
        targets::AudioRequirements,
    };

    const SAMPLE_RATE: u32 = 96_000;

    #[test]
    fn audio_format_must_match_project_and_target() {
        let requirements = AudioRequirements {
            required_sample_rate_hz: Some(96_000),
            require_native_float32: true,
        };
        let rate = SampleRateHz::try_new(96_000).unwrap();
        assert!(check_audio_format(&float32_format(96_000), &requirements, rate, 0).is_ok());
        assert!(matches!(
            check_audio_format(&float32_format(48_000), &requirements, rate, 0),
            Err(DoctorError::AudioFormat(_))
        ));

        let mut integer = float32_format(96_000);
        integer.native_float32 = false;
        assert!(matches!(
            check_audio_format(&integer, &requirements, rate, 0),
            Err(DoctorError::AudioFormat(_))
        ));

        let mut wrong_channel = float32_format(96_000);
        wrong_channel.input_channel = 1;
        assert!(matches!(
            check_audio_format(&wrong_channel, &requirements, rate, 0),
            Err(DoctorError::AudioFormat(_))
        ));

        let mut out_of_range = float32_format(96_000);
        out_of_range.channels = 1;
        out_of_range.input_channel = 1;
        assert!(matches!(
            check_audio_format(&out_of_range, &requirements, rate, 1),
            Err(DoctorError::AudioFormat(_))
        ));
    }

    #[test]
    fn waveform_profiles_are_spectrally_distinct() {
        let saw = harmonic_profile(&render(Wave::Saw), SAMPLE_RATE, 440.0);
        let triangle = harmonic_profile(&render(Wave::Triangle), SAMPLE_RATE, 440.0);
        let pulse = harmonic_profile(&render(Wave::Pulse), SAMPLE_RATE, 440.0);
        for (left, right) in [(&saw, &triangle), (&saw, &pulse), (&triangle, &pulse)] {
            let distance = spectral_distance(left, right);
            assert!(
                distance >= MIN_SPECTRAL_DISTANCE,
                "distance {distance} below threshold"
            );
        }
        assert!(spectral_distance(&saw, &saw) < MIN_SPECTRAL_DISTANCE);
    }

    enum Wave {
        Saw,
        Triangle,
        Pulse,
    }

    fn float32_format(sample_rate_hz: u32) -> AudioFormat {
        AudioFormat {
            sample_rate_hz,
            channels: 2,
            input_channel: 0,
            native_float32: true,
        }
    }

    fn render(wave: Wave) -> Vec<f32> {
        let mut phase = 0.0f64;
        let step = 440.0 / f64::from(SAMPLE_RATE);
        (0..SAMPLE_RATE as usize / 2)
            .map(|_| {
                let value = match wave {
                    Wave::Saw => 2.0 * phase - 1.0,
                    Wave::Triangle => {
                        if phase < 0.5 {
                            4.0 * phase - 1.0
                        } else {
                            3.0 - 4.0 * phase
                        }
                    }
                    Wave::Pulse => {
                        if phase < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                };
                phase += step;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                (value * 0.2) as f32
            })
            .collect()
    }
}
