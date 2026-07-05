use eframe::egui;
use egui_knob::{Knob, KnobStyle, LabelPosition};
use synth_core::ParamId;

use crate::engine::SynthEngineControl;

const KNOB_SIZE: f32 = 48.0;
const KNOB_FONT_SIZE: f32 = 12.0;
const KNOB_SWEEP_START: f32 = 1.0 / 12.0;
const KNOB_SWEEP_RANGE: f32 = 10.0 / 12.0;

pub fn param_knob_f32(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    let min = *range.start();
    let max = *range.end();
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let previous = *value;

    let response = ui.add(
        Knob::new(value, min, max, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_font_size(KNOB_FONT_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_label(label, LabelPosition::Bottom)
            .with_label_offset(3.0)
            .with_label_format(format_knob_value(min, max))
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );

    if response.changed() || *value != previous {
        control.set_param(param, *value);
    }
}

pub fn param_knob_bipolar(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    reset_value: f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    param_knob_f32(ui, label, value, -1.0..=1.0, reset_value, param, control);
}

pub fn param_knob_log_hz(
    ui: &mut egui::Ui,
    label: &str,
    value_hz: &mut f32,
    min_hz: f32,
    max_hz: f32,
    reset_hz: f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    let min_log = min_hz.ln();
    let max_log = max_hz.ln();
    let log_range = max_log - min_log;
    let mut normalized = ((*value_hz).clamp(min_hz, max_hz).ln() - min_log) / log_range;
    let previous_hz = *value_hz;
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let reset_normalized = (reset_hz.clamp(min_hz, max_hz).ln() - min_log) / log_range;

    let response = ui.add(
        Knob::new(&mut normalized, 0.0, 1.0, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_font_size(KNOB_FONT_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_label(label, LabelPosition::Bottom)
            .with_label_offset(3.0)
            .with_label_format(move |value| format_hz((min_log + value * log_range).exp()))
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_normalized)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );

    *value_hz = (min_log + normalized.clamp(0.0, 1.0) * log_range).exp();

    if response.changed() || (*value_hz - previous_hz).abs() > f32::EPSILON {
        control.set_param(param, *value_hz);
    }
}

pub fn param_toggle(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut bool,
    param: ParamId,
    control: &SynthEngineControl,
) {
    if ui.toggle_value(value, label).changed() {
        control.set_param(param, if *value { 1.0 } else { 0.0 });
    }
}

fn format_hz(value: f32) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

fn format_knob_value(min: f32, max: f32) -> impl Fn(f32) -> String + 'static {
    move |value| {
        if min < 0.0 && max <= 1.0 {
            format!("{value:+.2}")
        } else if max > 20.0 {
            format!("{value:.0}")
        } else if max > 2.0 {
            format!("{value:.2}")
        } else {
            format!("{value:.2}")
        }
    }
}
