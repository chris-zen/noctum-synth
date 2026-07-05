use eframe::egui;
use egui_knob::{Knob, KnobStyle};
use synth_core::ParamId;

use crate::engine::SynthEngineControl;

pub const KNOB_SIZE: f32 = 32.0;
pub const MASTER_KNOB_SIZE: f32 = 22.0;
pub const MASTER_FONT_SIZE: f32 = 12.0;
const KNOB_FONT_SIZE: f32 = 11.0;
const KNOB_SWEEP_START: f32 = 1.0 / 12.0;
const KNOB_SWEEP_RANGE: f32 = 10.0 / 12.0;
const KNOB_LABEL_OVERLAP: f32 = -6.0;

fn knob_edit_id(param: ParamId) -> egui::Id {
    egui::Id::new(format!("knob_txt_{param:?}"))
}

fn knob_value_edit(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    refresh_text: bool,
    format: impl Fn(f32) -> String,
    font_id: egui::FontId,
    text_color: egui::Color32,
) -> bool {
    let mut edit_text = ui
        .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
        .unwrap_or_default();

    let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));

    if !edit_has_focus && refresh_text {
        edit_text = format(*value);
    }

    if edit_text.is_empty() {
        edit_text = format(*value);
    }

    let edit_response = ui.add(
        egui::TextEdit::singleline(&mut edit_text)
            .id(edit_id)
            .font(font_id)
            .horizontal_align(egui::Align::Center)
            .frame(egui::Frame::NONE)
            .text_color(text_color),
    );

    let mut changed = false;
    if edit_response.lost_focus() && !edit_text.trim().is_empty() {
        if let Ok(new_val) = edit_text.trim().parse::<f32>() {
            let clamped = new_val.clamp(min, max);
            if (*value - clamped).abs() > f32::EPSILON {
                *value = clamped;
                changed = true;
            }
            edit_text = format(*value);
        } else {
            edit_text = format(*value);
        }
    }

    ui.memory_mut(|mem| mem.data.insert_temp(edit_id, edit_text));
    changed
}

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
    let format_fn = format_knob_value(min, max);

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(value, min, max, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);

    let edited = knob_value_edit(
        ui,
        edit_id,
        value,
        min,
        max,
        response.changed() || *value != previous,
        &format_fn,
        font_id,
        text_color,
    );

    if edited || response.changed() || *value != previous {
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

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut normalized, 0.0, 1.0, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_normalized)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );

    *value_hz = (min_log + normalized.clamp(0.0, 1.0) * log_range).exp();

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);

    let edited = knob_value_edit(
        ui,
        edit_id,
        value_hz,
        min_hz,
        max_hz,
        response.changed() || (*value_hz - previous_hz).abs() > f32::EPSILON,
        format_hz,
        font_id,
        text_color,
    );

    if edited || response.changed() || (*value_hz - previous_hz).abs() > f32::EPSILON {
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

pub fn master_volume(
    ui: &mut egui::Ui,
    value: &mut f32,
    control: &SynthEngineControl,
) {
    let previous = *value;
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let font_id = egui::FontId::proportional(MASTER_FONT_SIZE);

    ui.spacing_mut().item_spacing.y = 0.0;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        ui.label(
            egui::RichText::new("Master")
                .font(font_id.clone())
                .color(text_color),
        );

        ui.add_sized(
            [MASTER_KNOB_SIZE, MASTER_KNOB_SIZE],
            Knob::new(value, 0.0, 1.0, KnobStyle::Wiper)
                .with_size(MASTER_KNOB_SIZE)
                .with_stroke_width(1.5)
                .with_colors(knob_color, accent, text_color)
                .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
                .with_double_click_reset(1.0)
                .with_background_arc(false)
                .with_show_filled_segments(false),
        );

        let edit_id = egui::Id::new("knob_txt_master_volume");

        let mut edit_text = ui
            .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
            .unwrap_or_default();

        let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));
        let changed = *value != previous;

        if !edit_has_focus && changed {
            edit_text = format!("{:.2}", *value);
        }

        if edit_text.is_empty() {
            edit_text = format!("{:.2}", *value);
        }

        let edit_response = ui.add_sized(
            [38.0, MASTER_KNOB_SIZE],
            egui::TextEdit::singleline(&mut edit_text)
                .id(edit_id)
                .font(font_id)
                .horizontal_align(egui::Align::Center)
                .frame(egui::Frame::NONE)
                .text_color(text_color),
        );

        let apply = edit_response.lost_focus() && !edit_text.trim().is_empty();
        if apply {
            if let Ok(new_val) = edit_text.trim().parse::<f32>() {
                let clamped = new_val.clamp(0.0, 1.0);
                if (*value - clamped).abs() > f32::EPSILON {
                    *value = clamped;
                    control.set_param(ParamId::MasterVolume, *value);
                }
                edit_text = format!("{:.2}", *value);
            } else {
                edit_text = format!("{:.2}", *value);
            }
        }

        ui.memory_mut(|mem| mem.data.insert_temp(edit_id, edit_text));

        if changed {
            control.set_param(ParamId::MasterVolume, *value);
        }
    });
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
