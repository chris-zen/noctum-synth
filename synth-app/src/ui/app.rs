use eframe::egui;
use midir::MidiInputConnection;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::engine::SynthEngineBridge;
use crate::midi;
use crate::ui::analysis::{self, AnalysisState};
use crate::ui::params_view::UiState;
use crate::ui::viewport::{DeferredViewport, RootViewport};
use crate::ui::{params_view, settings_view};

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
    config: Config,
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

        let port_name = midi_port.or_else(|| config.settings.midi_port.clone());
        let midi_conn = midi::start_midi(port_name.as_deref(), engine.control.clone());

        Self {
            engine,
            active_tab: config.active_tab,
            theme_dark,
            analysis: Arc::new(Mutex::new(AnalysisState::default())),
            analysis_viewport: DeferredViewport::from_config(
                "analysis",
                "Analysis",
                [600.0, 500.0],
                config.analysis_open,
                config.analysis_viewport,
            ),
            main_viewport: RootViewport::from_config(config.main_viewport),
            ui_state: UiState::default(),
            midi_conn,
            config,
            last_autosave: Instant::now(),
        }
    }

    fn persist(&mut self) {
        self.config.active_tab = self.active_tab;
        self.config.main_viewport = self.main_viewport.geometry();
        self.config.analysis_open = self.analysis_viewport.open;
        self.config.analysis_viewport = self.analysis_viewport.geometry();
        self.config.save();
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

        egui::CentralPanel::default().show_inside(ui, |ui| match self.active_tab {
            Tab::Parameters => {
                params_view::show(
                    ui,
                    &mut self.ui_state,
                    &self.engine.control,
                    active,
                    total,
                    &mut self.analysis_viewport.open,
                );
            }
            Tab::Settings => {
                settings_view::show(
                    ui,
                    &mut self.config.settings,
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
