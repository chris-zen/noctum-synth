use serde::{Deserialize, Serialize};

use crate::ui::analysis::filter_design::FilterDesignViewConfig;
use crate::ui::analysis::osc_design::OscDesignViewConfig;
use crate::ui::analysis::real_time::RealTimeViewConfig;
use crate::ui::analysis::{AnalysisState, AnalysisTab};

#[derive(Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub active_tab: AnalysisTab,
    pub real_time: RealTimeViewConfig,
    pub osc_design: OscDesignViewConfig,
    pub filter_design: FilterDesignViewConfig,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self::from_state(&AnalysisState::default())
    }
}

impl AnalysisConfig {
    pub fn apply_to(&self, state: &mut AnalysisState) {
        state.active_tab = self.active_tab;
        self.real_time.apply_to(&mut state.real_time);
        self.osc_design.apply_to(&mut state.osc_design);
        self.filter_design.apply_to(&mut state.filter_design);
    }

    pub fn from_state(state: &AnalysisState) -> Self {
        Self {
            active_tab: state.active_tab,
            real_time: RealTimeViewConfig::from_state(&state.real_time),
            osc_design: OscDesignViewConfig::from_state(&state.osc_design),
            filter_design: FilterDesignViewConfig::from_state(&state.filter_design),
        }
    }
}
