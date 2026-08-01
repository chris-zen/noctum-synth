use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::{
    engine::AudioBlock,
    ui::analysis::{
        oscilloscope::{self, OscilloscopeState, OscilloscopeViewConfig},
        spectrum_analyzer::{self, FftState, FftViewConfig},
        vu_meter::{self, VU_WIDTH, VuMeterState},
    },
};

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpectrumChannel {
    Left,
    Right,
    Sum,
}

impl Default for SpectrumChannel {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSource {
    Input,
    Output,
    InputAndOutput,
}

impl Default for SignalSource {
    fn default() -> Self {
        Self::Output
    }
}

pub(crate) enum HoverStatus {
    Scope {
        time_ms: f32,
        levels: String,
    },
    Spectrum {
        freq_hz: f32,
        levels: String,
        note: String,
    },
}

const STATUS_H: f32 = 24.0;

fn draw_hover_status(ui: &mut egui::Ui, hover: Option<&HoverStatus>) {
    ui.allocate_ui(egui::vec2(ui.available_width(), STATUS_H), |ui| {
        ui.horizontal(|ui| {
            let help_button = ui
                .small_button("❔")
                .on_hover_text("Keyboard & gesture help");
            egui::Popup::menu(&help_button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(280.0);
                    ui.strong("Oscilloscope");
                    ui.label("Mode: Free / Auto / Normal / Single");
                    ui.label("Space: Run / Stop");
                    ui.label("Cmd + two-finger vertical: X zoom (around cursor)");
                    ui.label("Opt + two-finger vertical: Y range (down increases, up decreases)");
                    ui.label("Two-finger horizontal: pan left/right (inspectable buffer)");
                    ui.label("Right-click: reset timebase / view offset");
                    ui.label("Cmd + hover: set FFT window (inspectable buffer)");
                    ui.add_space(8.0);
                    ui.strong("Spectrum");
                    ui.label("Hover: frequency, level, note in status bar");
                    ui.add_space(8.0);
                    ui.strong("VU");
                    ui.label("Click meter: reset peak holds");
                });

            match hover {
                Some(HoverStatus::Scope { time_ms, levels }) => {
                    let time = if *time_ms < 1.0 {
                        format!("{time_ms:.1} ms")
                    } else {
                        format!("{time_ms:.0} ms")
                    };
                    ui.label(format!("Time: {time}   {levels}"));
                }
                Some(HoverStatus::Spectrum {
                    freq_hz,
                    levels,
                    note,
                }) => {
                    ui.label(format!(
                        "Freq: {}   {}   Note: {}",
                        spectrum_analyzer::format_hz(*freq_hz),
                        levels,
                        note
                    ));
                }
                None => {
                    ui.label("Hover: -");
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------
// RealTime root state
// ---------------------------------------------------------------------------

pub struct RealTimeState {
    pub sample_rate: f32,
    pub osc: OscilloscopeState,
    pub fft: FftState,
    pub vu: VuMeterState,
}

impl Default for RealTimeState {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
            osc: OscilloscopeState::default(),
            fft: FftState::default(),
            vu: VuMeterState::default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct RealTimeViewConfig {
    pub oscilloscope: OscilloscopeViewConfig,
    pub fft: FftViewConfig,
}

impl Default for RealTimeViewConfig {
    fn default() -> Self {
        Self::from_state(&RealTimeState::default())
    }
}

impl RealTimeViewConfig {
    pub fn from_state(state: &RealTimeState) -> Self {
        Self {
            oscilloscope: OscilloscopeViewConfig::from_state(&state.osc),
            fft: FftViewConfig::from_state(&state.fft),
        }
    }

    pub fn apply_to(&self, state: &mut RealTimeState) {
        self.oscilloscope.apply_to(&mut state.osc);
        self.fft.apply_to(&mut state.fft);
    }
}

// ---------------------------------------------------------------------------
// Main show
// ---------------------------------------------------------------------------

pub fn show(ui: &mut egui::Ui, audio_blocks: VecDeque<AudioBlock>, state: &mut RealTimeState) {
    // Free Opt/Alt for oscilloscope Y zoom. egui defaults ALT to
    // vertical_scroll_modifier, which remaps wheel deltas while Opt is held.
    ui.ctx().options_mut(|options| {
        options.input_options.vertical_scroll_modifier = egui::Modifiers::NONE;
    });

    for block in &audio_blocks {
        state.vu.feed(block, state.sample_rate);
    }

    oscilloscope::feed_audio(
        &mut state.osc,
        &mut state.fft,
        audio_blocks,
        state.sample_rate,
    );

    let available = ui.available_size();
    let gap = 12.0;
    let vu_gap = 8.0;
    let content_h = (available.y - STATUS_H).max(0.0);
    let plots_w = (available.x - VU_WIDTH - vu_gap).max(0.0);
    let section_h = ((content_h - gap) * 0.5).max(0.0);
    let osc_h = section_h;
    let fft_h = section_h;

    let mut hover = None;
    let (content_rect, _) =
        ui.allocate_exact_size(egui::vec2(available.x, content_h), egui::Sense::hover());
    let plots_rect = egui::Rect::from_min_size(content_rect.min, egui::vec2(plots_w, content_h));
    let vu_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.min.x + plots_w + vu_gap, content_rect.min.y),
        egui::vec2(VU_WIDTH, content_h),
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("rt_plots")
            .max_rect(plots_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(plots_rect));
            ui.set_max_size(plots_rect.size());
            ui.allocate_ui(egui::vec2(plots_w, osc_h), |ui| {
                ui.set_max_width(plots_w);
                ui.strong("Oscilloscope");
                ui.add_space(6.0);
                oscilloscope::draw_oscilloscope(
                    ui,
                    &mut state.osc,
                    &mut state.fft,
                    state.sample_rate,
                    &mut hover,
                );
            });
            ui.add_space(gap);
            ui.allocate_ui(egui::vec2(plots_w, fft_h), |ui| {
                ui.set_max_width(plots_w);
                ui.strong("Spectrum Analyzer");
                ui.add_space(6.0);
                spectrum_analyzer::draw_spectrum(
                    ui,
                    &mut state.fft,
                    &state.osc,
                    state.sample_rate,
                    &mut hover,
                );
            });
        },
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("rt_vu")
            .max_rect(vu_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(vu_rect));
            ui.set_max_size(vu_rect.size());
            vu_meter::draw_vu_meter(ui, &mut state.vu);
        },
    );

    draw_hover_status(ui, hover.as_ref());
}

#[cfg(test)]
mod tests {
    use super::super::oscilloscope::{TriggerSlope, find_combined_trigger};
    use super::SignalSource;

    #[test]
    fn source_defaults_to_internal_output() {
        assert!(matches!(SignalSource::default(), SignalSource::Output));
    }

    #[test]
    fn combined_trigger_uses_both_sources() {
        let input = [-0.4, -0.2, 0.1, 0.2];
        let output = [-0.4, -0.1, 0.2, 0.3];

        let trigger =
            find_combined_trigger(&input, &output, input.len(), 0.0, TriggerSlope::Rising);

        assert!(trigger.is_some());
        assert!((trigger.unwrap() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn falling_edge_trigger() {
        let buf = [0.3, 0.2, -0.1, -0.2];
        let trigger =
            super::super::oscilloscope::find_trigger(&buf, buf.len(), 0.0, TriggerSlope::Falling);
        assert!(trigger.is_some());
        assert!((trigger.unwrap() - (1.0 + 2.0 / 3.0)).abs() < 0.001);
    }
}
