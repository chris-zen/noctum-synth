use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

use crate::{
    audio::{
        AudioInput,
        cpal_input::{CpalAudioInput, CpalInputConfig},
    },
    doctor::{
        DoctorConfig, check_audio_format, probe_plans, require_doctor_success, run_doctor,
        write_doctor_record,
    },
    domain::{CaptureCase, OscillatorId, OscillatorWaveform, ParameterSetting},
    events::{Outcome, OutcomeStatus, Reporter},
    extraction::CaptureExtractor,
    midi::{
        FakeMidiTransport, TranscriptTransport, list_midi_output_names,
        midir_output::MidirOutputTransport,
    },
    project::{CaptureProject, CaseStatus, NewProjectRequest, ProjectDocument},
    runner::{RunConfig, RunSummary, install_ctrlc_flag, run_capture_with_reporter},
    targets::{
        StdinOperatorConfirmer, SynthTarget,
        prophet5_v1::{self, Prophet5V1},
    },
    terminal::{ColorChoice, Palette, ReporterConfig, TerminalReporter, format_duration},
};

const EXIT_ERROR: u8 = 1;
const EXIT_INTERRUPTED: u8 = 130;
const PLAN_PREVIEW_CASES: usize = 10;

#[derive(Debug, Parser)]
#[command(
    name = "synth-capture",
    version = env!("CARGO_PKG_VERSION"),
    about = "Capture oscillator references from external synths"
)]
pub struct Cli {
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub color: ColorChoice,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Devices {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    New {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        protocol: String,
        #[arg(long)]
        midi_port: String,
        #[arg(long)]
        audio_device: String,
        #[arg(long, default_value_t = 0)]
        input_channel: u32,
        #[arg(long, default_value_t = 96_000)]
        sample_rate: u32,
        #[arg(long)]
        plugin_version: String,
    },
    Doctor {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Run {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Status {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Verify {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Retry {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value_t = false)]
        failed: bool,
        #[arg(long, default_value_t = false)]
        all: bool,
        #[arg(long, default_value_t = false)]
        complete: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        case: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Extract {
        #[arg(long)]
        project: PathBuf,
    },
}

pub fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Command::Devices { json } => devices_command(json, cli.color),
        Command::New {
            project,
            target,
            protocol,
            midi_port,
            audio_device,
            input_channel,
            sample_rate,
            plugin_version,
        } => new_command(NewProjectRequest {
            root: project,
            target_id: target,
            protocol_id: protocol,
            midi_port,
            audio_device,
            input_channel,
            sample_rate_hz: sample_rate,
            plugin_version,
        }),
        Command::Doctor {
            project,
            dry_run,
            json,
        } => doctor_command(project, dry_run, json, cli.color),
        Command::Run {
            project,
            dry_run,
            json,
        } => run_command(project, dry_run, json, cli.color),
        Command::Status { project, json } => status_command(project, json),
        Command::Verify { project, json } => verify_command(project, json),
        Command::Retry {
            project,
            failed,
            all,
            complete,
            session,
            case,
            json,
        } => retry_command(project, failed, all, complete, session, case, json),
        Command::Extract { project } => extract_command(&project),
    }
}

fn devices_command(json: bool, color: ColorChoice) -> ExitCode {
    let report = collect_devices();
    if json {
        return match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(err) => print_error(&format!("{err}")),
        };
    }
    let config = ReporterConfig::new(color, false, 96_000);
    let palette = Palette::new(config.color_enabled());
    eprintln!("MIDI outputs:");
    if let Some(error) = &report.midi_error {
        eprintln!("  {} {error}", palette.error("ERROR"));
    }
    if report.midi_outputs.is_empty() && report.midi_error.is_none() {
        eprintln!("  (none)");
    } else {
        for name in &report.midi_outputs {
            eprintln!("  {} {}", palette.ok("available"), palette.item(name));
        }
    }
    eprintln!("Audio inputs:");
    if report.audio_inputs.is_empty() {
        eprintln!("  (none)");
    } else {
        for device in &report.audio_inputs {
            let marker = if device.float32 {
                palette.ok("float32")
            } else {
                palette.warn("no-float32")
            };
            eprintln!(
                "  {marker} {}  rates={:?}",
                palette.item(&device.name),
                device.sample_rates
            );
        }
    }
    ExitCode::SUCCESS
}

fn new_command(request: NewProjectRequest) -> ExitCode {
    match CaptureProject::create(request) {
        Ok(created) => {
            eprintln!(
                "created project {} ({} cases, fingerprint {})",
                created.root().display(),
                created.document().cases.len(),
                created.document().scientific_fingerprint
            );
            ExitCode::SUCCESS
        }
        Err(err) => print_error(&format!("{err}")),
    }
}

fn doctor_command(
    project_path: PathBuf,
    dry_run: bool,
    json: bool,
    color: ColorChoice,
) -> ExitCode {
    let project = match CaptureProject::open(&project_path) {
        Ok(project) => project,
        Err(err) => return print_error(&format!("{err}")),
    };
    let started = Instant::now();
    let config = ReporterConfig::new(
        color,
        json,
        project.document().protocol_config.sample_rate.get(),
    );
    let mut reporter = TerminalReporter::stderr(&config);
    let result = doctor_body(&project, dry_run, json, &mut reporter);
    finish_command(&mut reporter, started, result)
}

fn doctor_body(
    project: &CaptureProject,
    dry_run: bool,
    json: bool,
    reporter: &mut TerminalReporter,
) -> Result<CommandSuccess, CommandFailure> {
    let document = project.document();
    let mut target = build_live_target(document)?;
    if dry_run {
        print_doctor_plan(&mut target, reporter)?;
        return Ok(CommandSuccess::new(format!(
            "dry run listed {} doctor probes",
            probe_plans().len()
        )));
    }

    let stop = install_ctrlc_flag().map_err(|err| CommandFailure::Error(err.to_string()))?;
    let requirements = target.audio_requirements();
    let transport = MidirOutputTransport::open_exact(&document.midi_port)
        .map_err(|err| CommandFailure::Error(err.to_string()))?;
    let mut midi = TranscriptTransport::new(transport);
    let mut audio = CpalAudioInput::open_with_stop(
        CpalInputConfig {
            device_name: document.audio_device.clone(),
            sample_rate: document.protocol_config.sample_rate.get(),
            input_channel: document.input_channel,
            require_float32: requirements.require_native_float32,
        },
        stop.handle(),
    )
    .map_err(|err| CommandFailure::Error(err.to_string()))?;

    let mut confirmer = StdinOperatorConfirmer;
    let record = run_doctor(
        project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        &DoctorConfig::default(),
        &mut confirmer,
        reporter,
    )
    .map_err(|err| match err {
        crate::doctor::DoctorError::Stopped => {
            CommandFailure::Interrupted("operator setup aborted".to_string(), Vec::new())
        }
        other => CommandFailure::Error(other.to_string()),
    })?;
    write_doctor_record(project, &record).map_err(|err| CommandFailure::Error(err.to_string()))?;
    if json {
        print_json(&record)?;
    }
    Ok(
        CommandSuccess::new(format!("doctor passed {} probes", record.probes.len())).with_detail(
            format!(
                "record written to {}",
                project.doctor_record_path().display()
            ),
        ),
    )
}

fn run_command(project_path: PathBuf, dry_run: bool, json: bool, color: ColorChoice) -> ExitCode {
    let mut project = match CaptureProject::open(&project_path) {
        Ok(project) => project,
        Err(err) => return print_error(&format!("{err}")),
    };
    let started = Instant::now();
    let config = ReporterConfig::new(
        color,
        json,
        project.document().protocol_config.sample_rate.get(),
    );
    let mut reporter = TerminalReporter::stderr(&config);
    let result = run_body(&mut project, dry_run, json, &mut reporter);
    finish_command(&mut reporter, started, result)
}

fn run_body(
    project: &mut CaptureProject,
    dry_run: bool,
    json: bool,
    reporter: &mut TerminalReporter,
) -> Result<CommandSuccess, CommandFailure> {
    let mut target = build_live_target(project.document())?;
    if dry_run {
        let pending = print_run_plan(project, &mut target, reporter)?;
        let record = require_doctor_success(project)
            .map_err(|err| CommandFailure::Error(err.to_string()))?;
        return Ok(
            CommandSuccess::new(format!("dry run listed {pending} case(s) to capture"))
                .with_detail(format!(
                    "doctor record created at {} is compatible",
                    record.created_at_unix_ms
                )),
        );
    }
    require_doctor_success(project).map_err(|err| CommandFailure::Error(err.to_string()))?;

    let stop = install_ctrlc_flag().map_err(|err| CommandFailure::Error(err.to_string()))?;
    let requirements = target.audio_requirements();
    let document = project.document().clone();
    let transport = MidirOutputTransport::open_exact(&document.midi_port)
        .map_err(|err| CommandFailure::Error(err.to_string()))?;
    let mut midi = TranscriptTransport::new(transport);
    let mut audio = CpalAudioInput::open_with_stop(
        CpalInputConfig {
            device_name: document.audio_device.clone(),
            sample_rate: document.protocol_config.sample_rate.get(),
            input_channel: document.input_channel,
            require_float32: requirements.require_native_float32,
        },
        stop.handle(),
    )
    .map_err(|err| CommandFailure::Error(err.to_string()))?;
    check_audio_format(
        &audio.format(),
        &requirements,
        document.protocol_config.sample_rate,
        document.input_channel,
    )
    .map_err(|err| CommandFailure::Error(err.to_string()))?;

    let mut confirmer = StdinOperatorConfirmer;
    let summary = run_capture_with_reporter(
        project,
        &mut target,
        &mut midi,
        &mut audio,
        &stop,
        RunConfig::default(),
        &mut confirmer,
        reporter,
    )
    .map_err(|err| match err {
        crate::runner::RunnerError::Stopped => {
            CommandFailure::Interrupted("operator setup aborted".to_string(), Vec::new())
        }
        other => CommandFailure::Error(other.to_string()),
    })?;

    if json {
        print_json(&RunReport::new(project, &summary))?;
    }
    let status = project.status_report();
    let headline = format!(
        "{} complete, {} skipped, {} failed ({}/{} cases done)",
        summary.completed, summary.skipped, summary.failed, status.complete, status.total_cases
    );
    let mut details = vec![format!("project {}", project.root().display())];
    if status.complete < status.total_cases {
        details.push(format!(
            "resume with: cargo run --release -p synth-capture -- run --project {}",
            project.root().display()
        ));
    }
    if summary.interrupted {
        return Err(CommandFailure::Interrupted(headline, details));
    }
    Ok(CommandSuccess { headline, details })
}

fn status_command(project_path: PathBuf, json: bool) -> ExitCode {
    let project = match CaptureProject::open(&project_path) {
        Ok(project) => project,
        Err(err) => return print_error(&format!("{err}")),
    };
    let report = project.status_report();
    if json {
        return match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(err) => print_error(&format!("{err}")),
        };
    }
    eprintln!(
        "project {}  complete {}/{}  pending {}  failed {}  interrupted {}",
        report.project_id,
        report.complete,
        report.total_cases,
        report.pending,
        report.failed,
        report.interrupted
    );
    eprintln!(
        "captured {}  estimated remaining {}",
        format_duration(Duration::from_millis(report.captured_ms)),
        format_duration(Duration::from_millis(report.estimated_remaining_ms))
    );
    if let Some(current) = report.current_case_id {
        eprintln!("current {current}");
    }
    if let Some(last) = report.last_completed_case_id {
        eprintln!("last completed {last}");
    }
    ExitCode::SUCCESS
}

fn verify_command(project_path: PathBuf, json: bool) -> ExitCode {
    let project = match CaptureProject::open(&project_path) {
        Ok(project) => project,
        Err(err) => return print_error(&format!("{err}")),
    };
    let report = match project.verify() {
        Ok(report) => report,
        Err(err) => return print_error(&format!("{err}")),
    };
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(err) => return print_error(&format!("{err}")),
        }
    } else if report.ok {
        eprintln!("OK verify passed");
    } else {
        eprintln!("ERROR verify failed with {} issue(s)", report.issues.len());
        for issue in &report.issues {
            match &issue.case_id {
                Some(case_id) => eprintln!("  [{}] {}: {}", issue.kind, case_id, issue.message),
                None => eprintln!("  [{}] {}", issue.kind, issue.message),
            }
        }
    }
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_ERROR)
    }
}

fn retry_command(
    project_path: PathBuf,
    failed: bool,
    all: bool,
    complete: bool,
    session: Option<String>,
    case: Option<String>,
    json: bool,
) -> ExitCode {
    let mut project = match CaptureProject::open(&project_path) {
        Ok(project) => project,
        Err(err) => return print_error(&format!("{err}")),
    };
    let selectors = u8::from(failed)
        + u8::from(all)
        + u8::from(complete)
        + u8::from(session.is_some())
        + u8::from(case.is_some());
    if selectors != 1 {
        return print_error(
            "select cases with exactly one of --failed, --all, --complete, --session <id>, or --case <id>",
        );
    }
    let selected = if failed {
        project.case_ids_with_status(CaseStatus::Failed)
    } else if all {
        project.case_ids_with_captured_progress()
    } else if complete {
        project.case_ids_with_status(CaseStatus::Complete)
    } else if let Some(session_id) = session.as_deref() {
        project.case_ids_with_session(session_id)
    } else if let Some(case_id) = case {
        vec![case_id]
    } else {
        unreachable!("selector count checked above");
    };
    if selected.is_empty() {
        eprintln!("no matching cases to reset");
        return ExitCode::SUCCESS;
    }
    let stamp = match project.archive_and_reset_cases(&selected) {
        Ok(stamp) => stamp,
        Err(err) => return print_error(&format!("{err}")),
    };
    if json {
        return match serde_json::to_string_pretty(&RetryReport {
            reset_cases: selected,
            superseded_stamp: stamp,
        }) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(err) => print_error(&format!("{err}")),
        };
    }
    eprintln!(
        "reset {} case(s); archived under superseded/{stamp}",
        selected.len()
    );
    for case_id in &selected {
        eprintln!("  {case_id}");
    }
    ExitCode::SUCCESS
}

fn extract_command(project_path: &Path) -> ExitCode {
    match extract_project(project_path) {
        Ok(summary) => {
            eprintln!(
                "extract complete: {} waveform group(s), {} note(s) -> {}",
                summary.waveform_count,
                summary.note_count,
                summary.output_dir.display()
            );
            for path in &summary.files {
                eprintln!("  {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(err) => print_error(&err.to_string()),
    }
}

fn extract_project(
    project_path: &Path,
) -> Result<crate::extraction::ExtractionSummary, crate::extraction::ExtractionError> {
    let project = CaptureProject::open(project_path)
        .map_err(|err| crate::extraction::ExtractionError::Io(err.to_string()))?;
    let output = project.root().join("derived");
    let extractor = crate::extraction::OscillatorStaticExtractorV1;
    extractor.extract(&project, &output)
}

fn print_operator_setup_plan(target: &Prophet5V1, reporter: &mut TerminalReporter) {
    let steps = target.operator_setup_steps();
    if steps.is_empty() {
        return;
    }
    reporter.line(&format!(
        "operator setup (once per session, {} step(s)):",
        steps.len()
    ));
    for (index, step) in steps.iter().enumerate() {
        reporter.line(&format!("  {}. {}", index + 1, step.title));
        for line in step.instructions.lines() {
            reporter.line(&format!("     {line}"));
        }
    }
}

fn print_doctor_plan(
    target: &mut Prophet5V1,
    reporter: &mut TerminalReporter,
) -> Result<(), CommandFailure> {
    reporter.line("doctor dry run: no ports opened, no audio written");
    print_operator_setup_plan(target, reporter);
    for line in render_reset_sequence(target)? {
        reporter.line(&line);
    }
    for plan in probe_plans() {
        match plan.waveform {
            None => reporter.line(&format!("probe {}: silence, no note", plan.label)),
            Some(waveform) => {
                let ops = render_waveform_ops(target, waveform)?;
                reporter.line(&format!(
                    "probe {}: MIDI {} for {} operation(s)",
                    plan.label,
                    plan.note.map(|note| note.get()).unwrap_or_default(),
                    ops.len()
                ));
                for line in ops {
                    reporter.line(&format!("  {line}"));
                }
            }
        }
    }
    Ok(())
}

fn print_run_plan(
    project: &CaptureProject,
    target: &mut Prophet5V1,
    reporter: &mut TerminalReporter,
) -> Result<usize, CommandFailure> {
    reporter.line("run dry run: no ports opened, no audio written");
    print_operator_setup_plan(target, reporter);
    for line in render_reset_sequence(target)? {
        reporter.line(&line);
    }
    let mut pending = Vec::new();
    for case in &project.document().cases {
        let status = project
            .state()
            .cases
            .get(&case.id)
            .map(|entry| entry.status)
            .unwrap_or(CaseStatus::Pending);
        if status == CaseStatus::Complete {
            continue;
        }
        pending.push(case);
    }
    reporter.line(&format!(
        "{} of {} case(s) still need capture",
        pending.len(),
        project.document().cases.len()
    ));
    for case in pending.iter().take(PLAN_PREVIEW_CASES) {
        reporter.line(&format!("  {}", case.id));
    }
    if pending.len() > PLAN_PREVIEW_CASES {
        reporter.line(&format!(
            "  ... {} more",
            pending.len() - PLAN_PREVIEW_CASES
        ));
    }
    if let Some(first) = pending.first() {
        reporter.line(&format!("operations for {}", first.id));
        for line in render_case_ops(target, first)? {
            reporter.line(&format!("  {line}"));
        }
    }
    Ok(pending.len())
}

fn render_reset_sequence(target: &mut Prophet5V1) -> Result<Vec<String>, CommandFailure> {
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    target
        .reset(&mut midi)
        .map_err(|err| CommandFailure::Error(err.to_string()))?;
    let mut lines = vec![format!("reset sends {} message(s)", midi.entries().len())];
    for entry in midi.entries() {
        lines.push(format!("  {}", format_bytes(&entry.bytes)));
    }
    Ok(lines)
}

fn render_waveform_ops(
    target: &mut Prophet5V1,
    waveform: OscillatorWaveform,
) -> Result<Vec<String>, CommandFailure> {
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    target
        .set_parameter(
            &mut midi,
            &ParameterSetting::OscillatorWaveform {
                oscillator: OscillatorId::Two,
                waveform,
            },
        )
        .map_err(|err| CommandFailure::Error(err.to_string()))?;
    Ok(midi
        .entries()
        .iter()
        .map(|entry| format_bytes(&entry.bytes))
        .collect())
}

fn render_case_ops(
    target: &mut Prophet5V1,
    case: &CaptureCase,
) -> Result<Vec<String>, CommandFailure> {
    let mut midi = TranscriptTransport::new(FakeMidiTransport::default());
    for setting in &case.settings {
        target
            .set_parameter(&mut midi, setting)
            .map_err(|err| CommandFailure::Error(err.to_string()))?;
    }
    if let Some(stimulus) = &case.stimulus {
        target
            .note_on(&mut midi, stimulus.note, stimulus.velocity)
            .map_err(|err| CommandFailure::Error(err.to_string()))?;
        target
            .note_off(&mut midi, stimulus.note)
            .map_err(|err| CommandFailure::Error(err.to_string()))?;
    }
    Ok(midi
        .entries()
        .iter()
        .map(|entry| format_bytes(&entry.bytes))
        .collect())
}

fn build_live_target(document: &ProjectDocument) -> Result<Prophet5V1, CommandFailure> {
    if document.target.id != prophet5_v1::TARGET_ID {
        return Err(CommandFailure::Error(format!(
            "no live adapter for target `{}`",
            document.target.id
        )));
    }
    let live = prophet5_v1::descriptor();
    if live != document.target {
        return Err(CommandFailure::Error(
            "target adapter revision or MIDI mapping changed since the project was created"
                .to_string(),
        ));
    }
    Ok(Prophet5V1::new(document.protocol_config.midi_channel))
}

fn finish_command(
    reporter: &mut TerminalReporter,
    started: Instant,
    result: Result<CommandSuccess, CommandFailure>,
) -> ExitCode {
    let elapsed = started.elapsed();
    match result {
        Ok(success) => {
            let mut outcome = Outcome::new(OutcomeStatus::Success, success.headline, elapsed);
            outcome.details = success.details;
            reporter.finish(&outcome);
            ExitCode::SUCCESS
        }
        Err(CommandFailure::Interrupted(headline, details)) => {
            let mut outcome = Outcome::new(OutcomeStatus::Interrupted, headline, elapsed);
            outcome.details = details;
            reporter.finish(&outcome);
            ExitCode::from(EXIT_INTERRUPTED)
        }
        Err(CommandFailure::Error(message)) => {
            reporter.finish(&Outcome::new(OutcomeStatus::Failed, message, elapsed));
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CommandFailure> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| CommandFailure::Error(err.to_string()))?;
    println!("{text}");
    Ok(())
}

fn print_error(message: &str) -> ExitCode {
    eprintln!("ERROR: {message}");
    ExitCode::from(EXIT_ERROR)
}

fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_devices() -> DevicesReport {
    let (midi_outputs, midi_error) = match list_midi_output_names() {
        Ok(names) => (names, None),
        Err(err) => (Vec::new(), Some(err.to_string())),
    };
    DevicesReport {
        midi_outputs,
        midi_error,
        audio_inputs: list_audio_inputs(),
    }
}

fn list_audio_inputs() -> Vec<AudioInputDevice> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut inputs = Vec::new();
    for device in devices {
        let Ok(description) = device.description() else {
            continue;
        };
        let name = description.name().to_string();
        let mut sample_rates = Vec::new();
        let mut float32 = false;
        if let Ok(configs) = device.supported_input_configs() {
            for range in configs {
                sample_rates.push(range.min_sample_rate());
                sample_rates.push(range.max_sample_rate());
                if matches!(range.sample_format(), cpal::SampleFormat::F32) {
                    float32 = true;
                }
            }
        }
        sample_rates.sort_unstable();
        sample_rates.dedup();
        inputs.push(AudioInputDevice {
            name,
            sample_rates,
            float32,
        });
    }
    inputs.sort_by(|left, right| left.name.cmp(&right.name));
    inputs
}

struct CommandSuccess {
    headline: String,
    details: Vec<String>,
}

impl CommandSuccess {
    fn new(headline: impl Into<String>) -> Self {
        Self {
            headline: headline.into(),
            details: Vec::new(),
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }
}

enum CommandFailure {
    Error(String),
    Interrupted(String, Vec<String>),
}

#[derive(Serialize)]
struct DevicesReport {
    midi_outputs: Vec<String>,
    midi_error: Option<String>,
    audio_inputs: Vec<AudioInputDevice>,
}

#[derive(Serialize)]
struct AudioInputDevice {
    name: String,
    sample_rates: Vec<u32>,
    float32: bool,
}

#[derive(Serialize)]
struct RunReport {
    project_id: String,
    root: String,
    completed: usize,
    skipped: usize,
    failed: usize,
    interrupted: bool,
    complete: usize,
    total_cases: usize,
}

impl RunReport {
    fn new(project: &CaptureProject, summary: &RunSummary) -> Self {
        let status = project.status_report();
        Self {
            project_id: status.project_id,
            root: status.root,
            completed: summary.completed,
            skipped: summary.skipped,
            failed: summary.failed,
            interrupted: summary.interrupted,
            complete: status.complete,
            total_cases: status.total_cases,
        }
    }
}

#[derive(Serialize)]
struct RetryReport {
    reset_cases: Vec<String>,
    superseded_stamp: String,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::{
        cli::{Cli, Command},
        terminal::ColorChoice,
    };

    #[test]
    fn parses_global_color_and_run_flags() {
        let cli = Cli::try_parse_from([
            "synth-capture",
            "run",
            "--project",
            "/tmp/project",
            "--dry-run",
            "--color",
            "never",
        ])
        .unwrap();
        assert_eq!(cli.color, ColorChoice::Never);
        assert!(matches!(cli.command, Command::Run { dry_run: true, .. }));
    }

    #[test]
    fn parses_doctor_and_retry() {
        let cli = Cli::try_parse_from([
            "synth-capture",
            "doctor",
            "--project",
            "/tmp/project",
            "--json",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Doctor { json: true, .. }));

        let cli = Cli::try_parse_from([
            "synth-capture",
            "retry",
            "--project",
            "/tmp/project",
            "--case",
            "oscillator-static-v1/silence",
        ])
        .unwrap();
        match cli.command {
            Command::Retry {
                case,
                failed,
                all,
                complete,
                session,
                ..
            } => {
                assert!(!failed);
                assert!(!all);
                assert!(!complete);
                assert!(session.is_none());
                assert_eq!(case.as_deref(), Some("oscillator-static-v1/silence"));
            }
            other => panic!("unexpected command {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "synth-capture",
            "retry",
            "--project",
            "/tmp/project",
            "--all",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Retry {
                all: true,
                failed: false,
                complete: false,
                case: None,
                session: None,
                ..
            }
        ));

        let cli = Cli::try_parse_from([
            "synth-capture",
            "retry",
            "--project",
            "/tmp/project",
            "--session",
            "session-1",
        ])
        .unwrap();
        match cli.command {
            Command::Retry { session, .. } => {
                assert_eq!(session.as_deref(), Some("session-1"));
            }
            other => panic!("unexpected command {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_color_value() {
        assert!(
            Cli::try_parse_from([
                "synth-capture",
                "status",
                "--project",
                "/tmp",
                "--color",
                "rgb"
            ])
            .is_err()
        );
    }
}
