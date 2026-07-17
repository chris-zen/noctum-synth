pub mod config;
pub mod filter_design;
pub mod osc_design;
pub mod real_time;
pub mod spectrum;

use eframe::egui;
use std::collections::VecDeque;

use synth_core::FilterType;

use crate::engine::AudioBlock;
use crate::engine::SynthEngineControl;
use crate::ui::analysis::filter_design::FilterDesignState;
use crate::ui::analysis::osc_design::OscillatorViewState;
use crate::ui::analysis::real_time::RealTimeState;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AnalysisTab {
    #[default]
    RealTime,
    OscDesign,
    FilterDesign,
}

pub struct AnalysisState {
    pub active_tab: AnalysisTab,
    pub real_time: RealTimeState,
    pub osc_design: OscillatorViewState,
    pub filter_design: FilterDesignState,
}

impl Default for AnalysisState {
    fn default() -> Self {
        Self {
            active_tab: AnalysisTab::RealTime,
            real_time: RealTimeState::default(),
            osc_design: OscillatorViewState::default(),
            filter_design: FilterDesignState::default(),
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    audio_blocks: VecDeque<AudioBlock>,
    state: &mut AnalysisState,
    control: &SynthEngineControl,
    filter_type: &mut FilterType,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.active_tab, AnalysisTab::RealTime, "Real Time");
        ui.selectable_value(&mut state.active_tab, AnalysisTab::OscDesign, "Osc Design");
        ui.selectable_value(
            &mut state.active_tab,
            AnalysisTab::FilterDesign,
            "Filter Design",
        );
    });
    ui.separator();

    match state.active_tab {
        AnalysisTab::RealTime => real_time::show(ui, audio_blocks, &mut state.real_time),
        AnalysisTab::OscDesign => osc_design::show(ui, &mut state.osc_design),
        AnalysisTab::FilterDesign => {
            filter_design::show(ui, &mut state.filter_design, control, filter_type)
        }
    }
}
