use eframe::egui;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

use synth_core::dsp::FilterType;
use synth_core::{LayerPatch, ParamId};

use crate::audio::AudioManager;
use crate::config::Config;
use crate::engine::{AudioMetrics, SynthEngineBridge};
use crate::midi::MidiInputManager;
use crate::ui::analysis::{self, AnalysisState, config::AnalysisConfig};
use crate::ui::params_view::{PatchManager, UiState};
use crate::ui::settings_view::{AudioBaseline, MidiInputEntry};
use crate::ui::viewport::{DeferredViewport, RootViewport};
use crate::ui::{params_view, settings_view};

pub(crate) const APP_TITLE: &str = "Noctum";
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
    pub midi_inputs: MidiInputManager,
    pub patch_mgr: PatchManager,
    filter_type: Arc<Mutex<FilterType>>,
    config: Config,
    audio_manager: AudioManager,
    audio_baseline: AudioBaseline,
    applied_audio_generation: u64,
    last_autosave: Instant,
    muted: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: SynthEngineBridge,
        audio_manager: AudioManager,
        midi_port: Option<String>,
        mut config: Config,
    ) -> Self {
        let theme_dark = config.settings.dark_theme;

        cc.egui_ctx.set_visuals(if theme_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        let audio_baseline = AudioBaseline::from_settings(&config.settings);
        let applied_audio_generation = audio_manager.applied().generation;
        let filter_type = Arc::new(Mutex::new(config.filter_type));

        let mut midi_inputs =
            MidiInputManager::new(engine.control.clone(), engine.control.midi_output());
        if let Some(port) = midi_port {
            if !config
                .settings
                .midi_inputs
                .iter()
                .any(|entry| entry.port == port)
            {
                config.settings.midi_inputs.push(MidiInputEntry::new(port));
            }
        }
        config.settings.migrate_legacy_midi_port();
        midi_inputs.sync(
            &config.settings.midi_inputs,
            config.settings.midi_clock_source.as_deref(),
            config.settings.midi_clock_mode,
        );
        engine
            .control
            .set_midi_clock_mode(config.settings.midi_clock_mode);
        engine
            .control
            .set_midi_output_clock_mode(config.settings.midi_output_clock_mode);
        engine
            .control
            .set_midi_output_port(config.settings.midi_output_port.as_deref());
        engine.control.set_input_enabled(config.input_enabled);

        let mut analysis = AnalysisState::default();
        config.analysis.apply_to(&mut analysis);
        let applied = audio_manager.applied();
        if applied.sample_rate > 0 {
            analysis.real_time.sample_rate = applied.sample_rate as f32;
        }

        let mut patch_mgr = PatchManager::new();
        let mut ui_state = UiState::default();
        if let Some((patch, loaded_name, baseline, save_name)) = patch_mgr.load_autosave() {
            ui_state.apply_from_patch(&patch);
            patch_mgr.restore_autosave_metadata(loaded_name, baseline, save_name, &patch);
        }
        let muted = config.muted;
        engine
            .control
            .load_patch_respecting_mute(&LayerPatch::from(&ui_state), muted);

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
            filter_type,
            midi_inputs,
            config,
            audio_manager,
            audio_baseline,
            applied_audio_generation,
            last_autosave: Instant::now(),
            muted,
        }
    }

    fn sync_applied_audio(&mut self) {
        let applied = self.audio_manager.applied();
        if applied.generation == self.applied_audio_generation {
            return;
        }
        self.applied_audio_generation = applied.generation;
        if applied.sample_rate > 0 {
            self.analysis.lock().real_time.sample_rate = applied.sample_rate as f32;
        }
        if applied.error.is_none() {
            self.config.settings.sample_rate = applied.sample_rate_setting;
            self.audio_baseline = AudioBaseline::from_settings(&self.config.settings);
            let patch = LayerPatch::from(&self.ui_state);
            self.engine.control.all_notes_off();
            self.engine
                .control
                .load_patch_respecting_mute(&patch, self.muted);
            self.engine
                .control
                .set_filter_oversampling(self.config.settings.filter_oversampling);
            self.engine
                .control
                .set_filter_type(*self.filter_type.lock());
            self.engine
                .control
                .set_midi_clock_mode(self.config.settings.midi_clock_mode);
        }
    }

    fn persist(&mut self) {
        self.config.active_tab = self.active_tab;
        self.config.main_viewport = self.main_viewport.geometry();
        self.config.analysis_open = self.analysis_viewport.open;
        self.config.analysis_viewport = self.analysis_viewport.geometry();
        self.config.analysis = AnalysisConfig::from_state(&self.analysis.lock());
        self.config.filter_type = *self.filter_type.lock();
        self.config.muted = self.muted;
        self.config.input_enabled = self.engine.control.input_enabled();
        self.config.save();
        self.patch_mgr
            .save_autosave(&LayerPatch::from(&self.ui_state));
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.persist();
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_applied_audio();
        self.engine.view.drain_feedback();
        self.main_viewport.drive(ctx);

        self.engine.view.drain_midi_ui_updates(|update| {
            self.ui_state.apply_midi_update(update);
        });

        let mut imports = Vec::new();
        self.engine
            .view
            .drain_midi_program_imports(|program| imports.push(program));
        let mut saved_any = false;
        for program in imports {
            match self.patch_mgr.save_midi_program(&program) {
                Ok(path) => {
                    saved_any = true;
                    eprintln!(
                        "Imported {:?} program to {}",
                        program.source(),
                        path.display()
                    );
                }
                Err(err) => eprintln!(
                    "Failed to import bank {} program {}: {err}",
                    program.bank(),
                    program.program()
                ),
            }
        }
        if saved_any {
            self.patch_mgr.refresh();
        }

        if self.muted {
            self.engine
                .control
                .set_param_audio_only(ParamId::MasterVolume, 0.0);
        }

        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            self.persist();
            self.last_autosave = Instant::now();
        }

        self.midi_inputs.tick();
        self.engine.control.midi_output().tick();

        let engine_view = self.engine.view.clone();
        let analysis = self.analysis.clone();
        let filter_type = self.filter_type.clone();
        let analysis_control = self.engine.control.clone();

        self.analysis_viewport.show_deferred(ctx, move |ui| {
            let audio_blocks = engine_view.drain_audio_blocks();
            let mut state = analysis.lock();
            let mut selected_filter = filter_type.lock();
            analysis::show(
                ui,
                audio_blocks,
                &mut state,
                &analysis_control,
                &mut selected_filter,
            );
        });

        ctx.request_repaint();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("tab_bar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Parameters, "Parameters");
                ui.selectable_value(&mut self.active_tab, Tab::Settings, "Settings");
            });
            ui.add_space(4.0);
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
                let mut filter_type = self.filter_type.lock();
                params_view::show(
                    ui,
                    &mut self.ui_state,
                    &self.engine.control,
                    &mut self.analysis_viewport.open,
                    &mut self.patch_mgr,
                    &mut filter_type,
                    self.config.settings.midi_output_port.as_deref(),
                    &mut self.muted,
                );
            }
            Tab::Settings => {
                let current_patch = LayerPatch::from(&self.ui_state);
                let applied = self.audio_manager.applied();
                let filter_type = *self.filter_type.lock();
                settings_view::show(
                    ui,
                    &mut self.config.settings,
                    &self.audio_baseline,
                    &applied,
                    &self.audio_manager,
                    filter_type,
                    &self.engine.control,
                    &mut self.midi_inputs,
                    &current_patch,
                    self.muted,
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
