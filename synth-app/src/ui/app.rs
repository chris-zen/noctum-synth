use eframe::egui;
use midir::MidiInputConnection;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::engine::{AudioMetrics, SynthEngineBridge};
use crate::midi;
use crate::ui::analysis::{self, AnalysisState, config::AnalysisConfig};
use crate::ui::params_view::{PatchManager, UiState};
use crate::ui::settings_view::AudioBaseline;
use crate::ui::viewport::{DeferredViewport, RootViewport};
use crate::ui::{params_view, settings_view};
use synth_core::Patch;

pub(crate) const APP_TITLE: &str = "Analog Synth";
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(PartialEq, Eq, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    #[default]
    Parameters,
    Settings,
}

pub struct App {
    pub engine: SynthEngineBridge,
    pub active_tab: Tab,
    pub theme_dark: bool,
    pub analysis: Arc<Mutex<AnalysisState>>,
    pub analysis_viewport: DeferredViewport,
    pub main_viewport: RootViewport,
    pub ui_state: UiState,
    pub midi_conn: Option<MidiInputConnection<()>>,
    pub patch_mgr: PatchManager,
    config: Config,
    audio_baseline: AudioBaseline,
    last_autosave: Instant,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: SynthEngineBridge,
        midi_port: Option<String>,
        config: Config,
    ) -> Self {
        let theme_dark = config.settings.dark_theme;

        cc.egui_ctx.set_visuals(if theme_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        let audio_baseline = AudioBaseline::from_settings(&config.settings);

        let port_name = midi_port.or_else(|| config.settings.midi_port.clone());
        let midi_conn = midi::start_midi(port_name.as_deref(), engine.control.clone());

        let mut analysis = AnalysisState::default();
        config.analysis.apply_to(&mut analysis);

        let patch_mgr = PatchManager::new();
        let mut ui_state = UiState::default();
        if let Some(patch) = patch_mgr.load_autosave() {
            engine.control.load_patch(&patch);
            ui_state.apply_from_patch(&patch);
        }

        Self {
            engine,
            active_tab: config.active_tab,
            theme_dark,
            analysis: Arc::new(Mutex::new(analysis)),
            analysis_viewport: DeferredViewport::from_config(
                "analysis",
                "Analysis",
                [600.0, 500.0],
                config.analysis_open,
                config.analysis_viewport,
            ),
            main_viewport: RootViewport::from_config(config.main_viewport),
            ui_state,
            patch_mgr,
            midi_conn,
            config,
            audio_baseline,
            last_autosave: Instant::now(),
        }
    }

    fn persist(&mut self) {
        self.config.active_tab = self.active_tab;
        self.config.main_viewport = self.main_viewport.geometry();
        self.config.analysis_open = self.analysis_viewport.open;
        self.config.analysis_viewport = self.analysis_viewport.geometry();
        self.config.analysis = AnalysisConfig::from_state(&self.analysis.lock());
        self.config.save();
        self.patch_mgr.save_autosave(&Patch::from(&self.ui_state));
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.persist();
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.main_viewport.drive(ctx);

        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            self.persist();
            self.last_autosave = Instant::now();
        }

        let engine_view = self.engine.view.clone();
        let analysis = self.analysis.clone();

        self.analysis_viewport.show_deferred(ctx, move |ui| {
            let audio_blocks = engine_view.drain_audio_blocks();
            let mut state = analysis.lock();
            analysis::show(ui, audio_blocks, &mut state);
        });

        ctx.request_repaint();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("tab_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Parameters, "Parameters");
                ui.selectable_value(&mut self.active_tab, Tab::Settings, "Settings");
            });
        });

        let active = self.engine.view.active_voices();
        let total = self.engine.view.total_voices();
        let metrics = self.engine.view.metrics();

        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Voices: {active}/{total}"));
                if let Some(metrics) = metrics {
                    ui.separator();
                    ui.label(metrics_text(&metrics));
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| match self.active_tab {
            Tab::Parameters => {
                params_view::show(
                    ui,
                    &mut self.ui_state,
                    &self.engine.control,
                    &mut self.analysis_viewport.open,
                    &mut self.patch_mgr,
                );
            }
            Tab::Settings => {
                settings_view::show(
                    ui,
                    &mut self.config.settings,
                    &self.audio_baseline,
                    &self.engine.control,
                    &mut self.midi_conn,
                );
                if self.config.settings.dark_theme != self.theme_dark {
                    self.theme_dark = self.config.settings.dark_theme;
                    ui.ctx().set_visuals(if self.config.settings.dark_theme {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                }
            }
        });
    }
}

fn metrics_text(metrics: &AudioMetrics) -> String {
    format!(
        "cb {:.2}/{:.2}ms  render {:.2}/{:.2}ms  deadline {:.2}ms  over {}/{} of {}",
        metrics.callback_avg_ms,
        metrics.callback_max_ms,
        metrics.render_avg_ms,
        metrics.render_max_ms,
        metrics.deadline_ms,
        metrics.overruns,
        metrics.render_overruns,
        metrics.callbacks,
    )
}
