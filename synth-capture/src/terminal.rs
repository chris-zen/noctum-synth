use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use clap::ValueEnum;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};

use crate::events::{CaptureEvent, CasePhase, Outcome, OutcomeStatus, Reporter};

const REFRESH_HZ: u8 = 10;
const MIN_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const PROGRESS_CHARS: &str = "##-";
const OVERALL_TEMPLATE: &str = "{prefix} [{bar:32}] {pos}/{len} {percent:>3}% elapsed {elapsed_precise} ETA {eta_precise} {msg}";
const CURRENT_TEMPLATE: &str = "{prefix} [{bar:18}] {msg}";
const GREEN_BOLD: &str = "\x1b[1;32m";
const CYAN: &str = "\x1b[36m";
const YELLOW_BOLD: &str = "\x1b[1;33m";
const RED_BOLD: &str = "\x1b[1;31m";
const MAGENTA: &str = "\x1b[35m";
const DIM_WHITE: &str = "\x1b[2;37m";
const RESET: &str = "\x1b[0m";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Animated,
    Plain,
}

#[derive(Clone, Debug)]
pub struct ReporterConfig {
    pub color: ColorChoice,
    pub json: bool,
    pub interactive: bool,
    pub no_color_env: bool,
    pub sample_rate_hz: u32,
}

impl ReporterConfig {
    pub fn new(color: ColorChoice, json: bool, sample_rate_hz: u32) -> Self {
        Self {
            color,
            json,
            interactive: stderr_is_terminal(),
            no_color_env: no_color_env_set(),
            sample_rate_hz,
        }
    }

    pub fn render_mode(&self) -> RenderMode {
        if self.json || !self.interactive {
            RenderMode::Plain
        } else {
            RenderMode::Animated
        }
    }

    pub fn color_enabled(&self) -> bool {
        resolve_color(self.color, self.interactive, self.no_color_env, self.json)
    }
}

pub fn resolve_color(
    choice: ColorChoice,
    interactive: bool,
    no_color_env: bool,
    json: bool,
) -> bool {
    match choice {
        ColorChoice::Never => false,
        ColorChoice::Always => !json,
        ColorChoice::Auto => interactive && !no_color_env && !json,
    }
}

pub fn stderr_is_terminal() -> bool {
    use std::io::IsTerminal;
    io::stderr().is_terminal()
}

pub fn no_color_env_set() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

pub struct TerminalReporter {
    mode: RenderMode,
    palette: Palette,
    sink: LineSink,
    sample_rate_hz: u32,
    multi: Option<MultiProgress>,
    overall: Option<ProgressBar>,
    current: Option<ProgressBar>,
    current_label: String,
    current_phase: CasePhase,
    current_len: u64,
    last_update: Option<Instant>,
    finished: bool,
}

impl TerminalReporter {
    pub fn stderr(config: &ReporterConfig) -> Self {
        Self::with_sink(config, LineSink::Stderr)
    }

    pub fn memory(config: &ReporterConfig, term: MemoryTerm) -> Self {
        Self::with_sink(config, LineSink::Memory(term))
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn mode(&self) -> RenderMode {
        self.mode
    }

    pub fn overall_position(&self) -> Option<u64> {
        self.overall.as_ref().map(|bar| bar.position())
    }

    pub fn current_position(&self) -> Option<u64> {
        self.current.as_ref().map(|bar| bar.position())
    }

    pub fn active_bars(&self) -> usize {
        usize::from(self.overall.is_some()) + usize::from(self.current.is_some())
    }

    pub fn line(&self, text: &str) {
        match &self.multi {
            Some(multi) => {
                let _ = multi.println(text);
            }
            None => self.sink.write_line(text),
        }
    }

    fn with_sink(config: &ReporterConfig, sink: LineSink) -> Self {
        let mode = config.render_mode();
        let multi = match mode {
            RenderMode::Animated => {
                let multi = MultiProgress::with_draw_target(sink.draw_target());
                Some(multi)
            }
            RenderMode::Plain => None,
        };
        Self {
            mode,
            palette: Palette::new(config.color_enabled()),
            sink,
            sample_rate_hz: config.sample_rate_hz.max(1),
            multi,
            overall: None,
            current: None,
            current_label: String::new(),
            current_phase: CasePhase::Reset,
            current_len: 0,
            last_update: None,
            finished: false,
        }
    }

    fn session_started(&mut self, project_id: &str, total_cases: usize, complete_cases: usize) {
        self.line(&format!(
            "{} capture {} ({} cases, {} already complete)",
            self.palette.info("INFO"),
            self.palette.item(project_id),
            total_cases,
            complete_cases
        ));
        let Some(multi) = &self.multi else {
            return;
        };
        let bar = multi.add(ProgressBar::new(total_cases as u64));
        bar.set_style(bar_style(OVERALL_TEMPLATE));
        bar.set_prefix(self.palette.info("Capture"));
        bar.set_position(complete_cases as u64);
        self.overall = Some(bar);
        if self.mode == RenderMode::Animated {
            self.sink.write_str(HIDE_CURSOR);
        }
    }

    fn case_started(&mut self, label: &str, capture_frames: u64) {
        self.current_label = label.to_string();
        self.current_phase = CasePhase::Reset;
        self.current_len = capture_frames;
        self.last_update = None;
        let Some(multi) = &self.multi else {
            return;
        };
        if let Some(previous) = self.current.take() {
            previous.finish_and_clear();
            multi.remove(&previous);
        }
        let bar = multi.add(ProgressBar::new(capture_frames));
        bar.set_style(bar_style(CURRENT_TEMPLATE));
        bar.set_prefix(self.palette.info("Current"));
        bar.set_message(self.current_message(0));
        self.current = Some(bar);
    }

    fn case_phase_changed(&mut self, phase: CasePhase) {
        self.current_phase = phase;
        match &self.current {
            Some(bar) => bar.set_message(self.current_message(bar.position())),
            None => self.line(&format!(
                "{} {}  {}",
                self.palette.info("INFO"),
                self.palette.item(&self.current_label),
                phase.label()
            )),
        }
    }

    fn case_progress(&mut self, frames: u64) {
        if self.current.is_none() {
            return;
        }
        let final_frame = frames >= self.current_len;
        if !final_frame && !self.update_due() {
            return;
        }
        let message = self.current_message(frames);
        if let Some(bar) = &self.current {
            bar.set_position(frames);
            bar.set_message(message);
        }
    }

    fn case_completed(&mut self, case_id: &str) {
        if let Some(bar) = &self.overall {
            bar.inc(1);
            bar.set_message(String::new());
        }
        self.clear_current();
        if self.multi.is_none() {
            self.line(&format!(
                "{} complete {}",
                self.palette.ok("OK"),
                self.palette.item(case_id)
            ));
        }
    }

    fn case_skipped(&mut self, case_id: &str) {
        let text = format!("skipped {case_id}");
        match &self.overall {
            Some(bar) => bar.set_message(self.palette.skipped(&text)),
            None => self.line(&format!(
                "{} {}",
                self.palette.skipped("SKIP"),
                self.palette.skipped(case_id)
            )),
        }
    }

    fn case_failed(&mut self, case_id: &str, reason: &str) {
        self.clear_current();
        self.line(&format!(
            "{} {} {}",
            self.palette.error("ERROR"),
            self.palette.item(case_id),
            reason
        ));
    }

    fn case_interrupted(&mut self, case_id: &str, reason: &str) {
        self.clear_current();
        self.line(&format!(
            "{} {} {}",
            self.palette.warn("WARN"),
            self.palette.item(case_id),
            reason
        ));
    }

    fn current_message(&self, frames: u64) -> String {
        let rate = f64::from(self.sample_rate_hz);
        format!(
            "{}  {}  {:.1}/{:.1} s",
            self.palette.item(&self.current_label),
            self.current_phase.label(),
            frames as f64 / rate,
            self.current_len as f64 / rate
        )
    }

    fn update_due(&mut self) -> bool {
        let now = Instant::now();
        match self.last_update {
            Some(previous) if now.duration_since(previous) < MIN_UPDATE_INTERVAL => false,
            _ => {
                self.last_update = Some(now);
                true
            }
        }
    }

    fn clear_current(&mut self) {
        if let Some(bar) = self.current.take() {
            bar.finish_and_clear();
            if let Some(multi) = &self.multi {
                multi.remove(&bar);
            }
        }
        self.last_update = None;
    }

    fn clear_all(&mut self) {
        self.clear_current();
        if let Some(bar) = self.overall.take() {
            bar.finish_and_clear();
            if let Some(multi) = &self.multi {
                multi.remove(&bar);
            }
        }
        if let Some(multi) = &self.multi {
            let _ = multi.clear();
        }
        if self.mode == RenderMode::Animated {
            self.sink.write_str(SHOW_CURSOR);
        }
    }
}

impl Reporter for TerminalReporter {
    fn event(&mut self, event: &CaptureEvent) {
        match event {
            CaptureEvent::SessionStarted {
                project_id,
                total_cases,
                complete_cases,
            } => self.session_started(project_id, *total_cases, *complete_cases),
            CaptureEvent::CaseStarted {
                label,
                capture_frames,
                ..
            } => self.case_started(label, *capture_frames),
            CaptureEvent::CasePhaseChanged { phase, .. } => self.case_phase_changed(*phase),
            CaptureEvent::CaseProgress { frames, .. } => self.case_progress(*frames),
            CaptureEvent::CaseCompleted { case_id } => self.case_completed(case_id),
            CaptureEvent::CaseSkipped { case_id } => self.case_skipped(case_id),
            CaptureEvent::CaseFailed { case_id, reason } => self.case_failed(case_id, reason),
            CaptureEvent::CaseInterrupted { case_id, reason } => {
                self.case_interrupted(case_id, reason)
            }
            CaptureEvent::DoctorStarted { probe_count } => self.line(&format!(
                "{} doctor running {probe_count} probes",
                self.palette.info("INFO")
            )),
            CaptureEvent::DoctorProbeStarted { label } => self.line(&format!(
                "{} probe {}",
                self.palette.info("INFO"),
                self.palette.item(label)
            )),
            CaptureEvent::DoctorProbePassed { label, detail } => self.line(&format!(
                "{} {} {detail}",
                self.palette.ok("OK"),
                self.palette.item(label)
            )),
            CaptureEvent::DoctorProbeFailed { label, reason } => self.line(&format!(
                "{} {} {reason}",
                self.palette.error("ERROR"),
                self.palette.item(label)
            )),
            CaptureEvent::DoctorFinished { ok } => {
                if *ok {
                    self.line(&format!("{} doctor probes passed", self.palette.ok("OK")));
                } else {
                    self.line(&format!("{} doctor failed", self.palette.error("ERROR")));
                }
            }
            CaptureEvent::Info { message } => {
                self.line(&format!("{} {message}", self.palette.info("INFO")))
            }
            CaptureEvent::Warning { message } => {
                self.line(&format!("{} {message}", self.palette.warn("WARN")))
            }
        }
    }

    fn finish(&mut self, outcome: &Outcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.clear_all();
        let tag = match outcome.status {
            OutcomeStatus::Success => self.palette.ok("OK"),
            OutcomeStatus::Interrupted => self.palette.warn("WARN"),
            OutcomeStatus::Failed => self.palette.error("ERROR"),
        };
        self.sink.write_line(&format!(
            "{tag} {} (elapsed {})",
            outcome.headline,
            format_duration(outcome.elapsed)
        ));
        for detail in &outcome.details {
            self.sink.write_line(&format!("     {detail}"));
        }
        self.sink.flush();
    }
}

impl Drop for TerminalReporter {
    fn drop(&mut self) {
        if !self.finished {
            self.clear_all();
            self.sink.flush();
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    enabled: bool,
}

impl Palette {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn ok(&self, text: &str) -> String {
        self.paint(GREEN_BOLD, text)
    }

    pub fn info(&self, text: &str) -> String {
        self.paint(CYAN, text)
    }

    pub fn warn(&self, text: &str) -> String {
        self.paint(YELLOW_BOLD, text)
    }

    pub fn error(&self, text: &str) -> String {
        self.paint(RED_BOLD, text)
    }

    pub fn item(&self, text: &str) -> String {
        self.paint(MAGENTA, text)
    }

    pub fn skipped(&self, text: &str) -> String {
        self.paint(DIM_WHITE, text)
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }
}

#[derive(Clone, Debug)]
enum LineSink {
    Stderr,
    Memory(MemoryTerm),
}

impl LineSink {
    fn draw_target(&self) -> ProgressDrawTarget {
        match self {
            Self::Stderr => ProgressDrawTarget::stderr_with_hz(REFRESH_HZ),
            Self::Memory(term) => {
                ProgressDrawTarget::term_like_with_hz(Box::new(term.clone()), REFRESH_HZ)
            }
        }
    }

    fn write_line(&self, text: &str) {
        match self {
            Self::Stderr => {
                let mut handle = io::stderr().lock();
                let _ = writeln!(handle, "{text}");
            }
            Self::Memory(term) => {
                let _ = TermLike::write_line(term, text);
            }
        }
    }

    fn write_str(&self, text: &str) {
        match self {
            Self::Stderr => {
                let mut handle = io::stderr().lock();
                let _ = write!(handle, "{text}");
                let _ = handle.flush();
            }
            Self::Memory(term) => {
                let _ = TermLike::write_str(term, text);
            }
        }
    }

    fn flush(&self) {
        match self {
            Self::Stderr => {
                let _ = io::stderr().flush();
            }
            Self::Memory(term) => {
                let _ = TermLike::flush(term);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryTerm {
    width: u16,
    height: u16,
    state: Arc<Mutex<MemoryTermState>>,
}

impl MemoryTerm {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            state: Arc::new(Mutex::new(MemoryTermState::default())),
        }
    }

    pub fn written(&self) -> String {
        self.lock().written.clone()
    }

    pub fn lines(&self) -> Vec<String> {
        self.lock().lines.clone()
    }

    pub fn frame(&self) -> String {
        self.lock().frame.clone()
    }

    pub fn contains_ansi(&self) -> bool {
        self.lock().written.contains('\x1b')
    }

    pub fn cursor_hidden(&self) -> bool {
        let state = self.lock();
        state.written.matches(HIDE_CURSOR).count() > state.written.matches(SHOW_CURSOR).count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryTermState> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl TermLike for MemoryTerm {
    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn move_cursor_up(&self, _n: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_down(&self, _n: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_right(&self, _n: usize) -> io::Result<()> {
        Ok(())
    }

    fn move_cursor_left(&self, _n: usize) -> io::Result<()> {
        Ok(())
    }

    fn write_line(&self, s: &str) -> io::Result<()> {
        let mut state = self.lock();
        state.written.push_str(s);
        state.written.push('\n');
        state.lines.push(s.to_string());
        state.frame.clear();
        Ok(())
    }

    fn write_str(&self, s: &str) -> io::Result<()> {
        let mut state = self.lock();
        state.written.push_str(s);
        state.frame.push_str(s);
        Ok(())
    }

    fn clear_line(&self) -> io::Result<()> {
        let mut state = self.lock();
        state.frame.clear();
        state.clear_line_calls += 1;
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct MemoryTermState {
    written: String,
    lines: Vec<String>,
    frame: String,
    clear_line_calls: usize,
}

fn bar_style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars(PROGRESS_CHARS)
}

pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        events::{CaptureEvent, CasePhase, Outcome, OutcomeStatus, Reporter},
        terminal::{
            ColorChoice, MemoryTerm, RenderMode, ReporterConfig, TerminalReporter, resolve_color,
        },
    };

    const FRAMES: u64 = 4_800;

    #[test]
    fn color_never_leaves_no_escape_bytes() {
        let term = MemoryTerm::new(120, 20);
        let mut reporter =
            TerminalReporter::memory(&config(ColorChoice::Never, false, false), term.clone());
        drive_one_case(&mut reporter);
        reporter.finish(&Outcome::new(
            OutcomeStatus::Success,
            "1 complete",
            Duration::from_secs(3),
        ));
        assert!(!term.contains_ansi(), "{:?}", term.written());
    }

    #[test]
    fn color_always_emits_escape_bytes() {
        let term = MemoryTerm::new(120, 20);
        let mut reporter =
            TerminalReporter::memory(&config(ColorChoice::Always, false, false), term.clone());
        drive_one_case(&mut reporter);
        reporter.finish(&Outcome::new(
            OutcomeStatus::Success,
            "1 complete",
            Duration::from_secs(3),
        ));
        assert!(term.contains_ansi());
    }

    #[test]
    fn no_color_env_and_json_override_auto() {
        assert!(resolve_color(ColorChoice::Auto, true, false, false));
        assert!(!resolve_color(ColorChoice::Auto, true, true, false));
        assert!(!resolve_color(ColorChoice::Auto, false, false, false));
        assert!(!resolve_color(ColorChoice::Auto, true, false, true));
        assert!(!resolve_color(ColorChoice::Always, true, false, true));
        assert!(!resolve_color(ColorChoice::Never, true, false, false));
        assert!(resolve_color(ColorChoice::Always, false, true, false));
    }

    #[test]
    fn json_mode_disables_bars_and_color() {
        let term = MemoryTerm::new(120, 20);
        let mut config = config(ColorChoice::Always, false, true);
        config.interactive = true;
        let mut reporter = TerminalReporter::memory(&config, term.clone());
        assert_eq!(reporter.mode(), RenderMode::Plain);
        assert!(!reporter.palette().enabled());
        drive_one_case(&mut reporter);
        reporter.finish(&Outcome::new(
            OutcomeStatus::Success,
            "1 complete",
            Duration::from_secs(1),
        ));
        assert_eq!(reporter.active_bars(), 0);
        assert!(!term.contains_ansi());
    }

    #[test]
    fn non_tty_mode_emits_bounded_plain_lines() {
        let term = MemoryTerm::new(120, 20);
        let mut reporter =
            TerminalReporter::memory(&config(ColorChoice::Never, false, false), term.clone());
        assert_eq!(reporter.mode(), RenderMode::Plain);
        reporter.event(&CaptureEvent::SessionStarted {
            project_id: "proj".to_string(),
            total_cases: 2,
            complete_cases: 0,
        });
        for index in 0..2 {
            let case_id = format!("case-{index}");
            reporter.event(&CaptureEvent::CaseStarted {
                case_id: case_id.clone(),
                label: "saw  MIDI 69".to_string(),
                capture_frames: FRAMES,
            });
            for phase in [
                CasePhase::Reset,
                CasePhase::Settle,
                CasePhase::Discard,
                CasePhase::Record,
                CasePhase::Validate,
                CasePhase::Commit,
            ] {
                reporter.event(&CaptureEvent::CasePhaseChanged {
                    case_id: case_id.clone(),
                    phase,
                });
            }
            for frame in 0..FRAMES {
                reporter.event(&CaptureEvent::CaseProgress {
                    case_id: case_id.clone(),
                    frames: frame,
                });
            }
            reporter.event(&CaptureEvent::CaseCompleted { case_id });
        }
        reporter.finish(&Outcome::new(
            OutcomeStatus::Success,
            "2 complete",
            Duration::from_secs(9),
        ));
        let lines = term.lines();
        assert!(lines.len() <= 1 + 2 * (6 + 1) + 1, "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("complete case-1")));
    }

    #[test]
    fn resume_starts_overall_bar_at_complete_count() {
        let term = MemoryTerm::new(120, 20);
        let mut config = config(ColorChoice::Never, true, false);
        config.interactive = true;
        let mut reporter = TerminalReporter::memory(&config, term);
        assert_eq!(reporter.mode(), RenderMode::Animated);
        reporter.event(&CaptureEvent::SessionStarted {
            project_id: "proj".to_string(),
            total_cases: 226,
            complete_cases: 78,
        });
        assert_eq!(reporter.overall_position(), Some(78));
        reporter.event(&CaptureEvent::CaseSkipped {
            case_id: "already".to_string(),
        });
        assert_eq!(reporter.overall_position(), Some(78));
        reporter.event(&CaptureEvent::CaseCompleted {
            case_id: "fresh".to_string(),
        });
        assert_eq!(reporter.overall_position(), Some(79));
    }

    #[test]
    fn frame_progress_reaches_exact_length() {
        let term = MemoryTerm::new(120, 20);
        let mut config = config(ColorChoice::Never, true, false);
        config.interactive = true;
        let mut reporter = TerminalReporter::memory(&config, term);
        reporter.event(&CaptureEvent::SessionStarted {
            project_id: "proj".to_string(),
            total_cases: 1,
            complete_cases: 0,
        });
        reporter.event(&CaptureEvent::CaseStarted {
            case_id: "case".to_string(),
            label: "pulse  MIDI 52".to_string(),
            capture_frames: FRAMES,
        });
        for frame in (0..=FRAMES).step_by(480) {
            reporter.event(&CaptureEvent::CaseProgress {
                case_id: "case".to_string(),
                frames: frame,
            });
        }
        assert_eq!(reporter.current_position(), Some(FRAMES));
        assert_eq!(reporter.active_bars(), 2);
    }

    #[test]
    fn error_cleanup_clears_bars_and_restores_cursor() {
        let term = MemoryTerm::new(120, 20);
        let mut config = config(ColorChoice::Never, true, false);
        config.interactive = true;
        let mut reporter = TerminalReporter::memory(&config, term.clone());
        reporter.event(&CaptureEvent::SessionStarted {
            project_id: "proj".to_string(),
            total_cases: 3,
            complete_cases: 0,
        });
        reporter.event(&CaptureEvent::CaseStarted {
            case_id: "case".to_string(),
            label: "saw  MIDI 69".to_string(),
            capture_frames: FRAMES,
        });
        assert!(term.cursor_hidden());
        reporter.event(&CaptureEvent::CaseInterrupted {
            case_id: "case".to_string(),
            reason: "stopped by operator".to_string(),
        });
        reporter.finish(&Outcome::new(
            OutcomeStatus::Interrupted,
            "interrupted",
            Duration::from_secs(4),
        ));
        assert_eq!(reporter.active_bars(), 0);
        assert!(!term.cursor_hidden());
        assert!(term.frame().trim().is_empty(), "{:?}", term.frame());
        assert!(term.lines().iter().any(|line| line.contains("interrupted")));
    }

    fn config(color: ColorChoice, interactive: bool, json: bool) -> ReporterConfig {
        ReporterConfig {
            color,
            json,
            interactive,
            no_color_env: false,
            sample_rate_hz: 96_000,
        }
    }

    fn drive_one_case(reporter: &mut TerminalReporter) {
        reporter.event(&CaptureEvent::SessionStarted {
            project_id: "proj".to_string(),
            total_cases: 1,
            complete_cases: 0,
        });
        reporter.event(&CaptureEvent::CaseStarted {
            case_id: "case".to_string(),
            label: "saw  MIDI 69".to_string(),
            capture_frames: FRAMES,
        });
        reporter.event(&CaptureEvent::CasePhaseChanged {
            case_id: "case".to_string(),
            phase: CasePhase::Record,
        });
        reporter.event(&CaptureEvent::CaseProgress {
            case_id: "case".to_string(),
            frames: FRAMES,
        });
        reporter.event(&CaptureEvent::CaseCompleted {
            case_id: "case".to_string(),
        });
    }
}
