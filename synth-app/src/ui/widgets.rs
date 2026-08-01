use eframe::egui;
use egui_knob::{Knob, KnobStyle};

use synth_core::{
    ParamId,
    midi::prophet::{
        FILTER_CUTOFF_RAW_MAX, FILTER_KEY_TRACK_RAW_MAX, cutoff_hz_to_raw, cutoff_raw_to_hz,
        key_track_from_raw, key_track_to_raw,
    },
};

use crate::engine::SynthEngineControl;

pub const KNOB_SIZE: f32 = 32.0;
pub const MASTER_KNOB_SIZE: f32 = 22.0;
pub const MASTER_FONT_SIZE: f32 = 12.0;
const MASTER_KNOB_STROKE: f32 = 1.5;
const KNOB_FONT_SIZE: f32 = 11.0;
const KNOB_SWEEP_START: f32 = 1.0 / 12.0;
const KNOB_SWEEP_RANGE: f32 = 10.0 / 12.0;
const KNOB_LABEL_OVERLAP: f32 = -6.0;

fn linear_gain_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.log10()
    }
}

fn db_to_linear_gain(db: f32) -> f32 {
    10.0f32.powf(db / 20.0).clamp(0.0, 1.0)
}

fn format_master_volume(linear: f32) -> String {
    let db = linear_gain_to_db(linear);
    if db.is_infinite() {
        "-inf dB".to_string()
    } else {
        format!("{db:.1} dB")
    }
}

fn parse_master_volume(input: &str) -> Option<f32> {
    let trimmed = input.trim().to_lowercase();
    if trimmed == "-inf" || trimmed == "-∞" {
        Some(0.0)
    } else if let Ok(db) = trimmed.replace("db", "").trim().parse::<f32>() {
        Some(db_to_linear_gain(db))
    } else {
        None
    }
}

fn knob_edit_id(param: ParamId) -> egui::Id {
    egui::Id::new(format!("knob_txt_{param:?}"))
}

fn discrete_knob_drag_id(param: ParamId) -> egui::Id {
    egui::Id::new(format!("knob_discrete_drag_{param:?}"))
}

fn knob_value_edit(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    display_offset: f32,
    format: impl Fn(f32) -> String,
    font_id: egui::FontId,
    text_color: egui::Color32,
) -> bool {
    let display = *value - display_offset;
    let mut edit_text = ui
        .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
        .unwrap_or_default();

    let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));

    if !edit_has_focus {
        edit_text = format(display);
    }

    if edit_text.is_empty() {
        edit_text = format(display);
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
            let clamped = (new_val + display_offset).clamp(min, max);
            if (*value - clamped).abs() > f32::EPSILON {
                *value = clamped;
                changed = true;
            }
            edit_text = format(*value - display_offset);
        } else {
            edit_text = format(*value - display_offset);
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
    param_knob_f32_offset(ui, label, value, range, reset_value, 0.0, param, control);
}

pub(crate) fn param_knob_f32_offset(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    display_offset: f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    if param_knob_f32_offset_inner(
        ui,
        label,
        value,
        range,
        reset_value,
        display_offset,
        knob_edit_id(param),
    ) {
        control.set_param(param, *value);
    }
}

fn param_knob_f32_offset_inner(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    display_offset: f32,
    edit_id: egui::Id,
) -> bool {
    let min = *range.start();
    let max = *range.end();
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let mut knob_value = *value;
    let format_fn = format_knob_value(min - display_offset, max - display_offset);

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut knob_value, min, max, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );
    if response.changed() {
        *value = knob_value;
    }

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);

    let edited = knob_value_edit(
        ui,
        edit_id,
        value,
        min,
        max,
        display_offset,
        &format_fn,
        font_id,
        text_color,
    );

    edited || response.changed()
}

pub fn param_knob_f32_custom(
    ui: &mut egui::Ui,
    label: impl Fn() -> String,
    display: impl Fn(f32) -> String,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    let changed = param_knob_f32_custom_inner(ui, &label, &display, value, range, reset_value, 0.0);
    if changed {
        control.set_param(param, *value);
    }
}

pub(crate) fn linked_param_knob_f32_custom(
    ui: &mut egui::Ui,
    label: impl Fn() -> String,
    display: impl Fn(f32) -> String,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    params: [ParamId; 2],
    control: &SynthEngineControl,
) -> bool {
    let changed = param_knob_f32_custom_inner(ui, &label, &display, value, range, reset_value, 0.0);
    if changed {
        for param in params {
            control.set_param(param, *value);
        }
    }
    changed
}

fn param_knob_f32_custom_inner(
    ui: &mut egui::Ui,
    label: &impl Fn() -> String,
    display: &impl Fn(f32) -> String,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    reset_value: f32,
    _display_offset: f32,
) -> bool {
    let min = *range.start();
    let max = *range.end();
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let mut knob_value = *value;

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut knob_value, min, max, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );
    if response.changed() {
        *value = knob_value;
    }

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label())
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    ui.label(
        egui::RichText::new(display(*value))
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    response.changed()
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

    if response.changed() {
        *value_hz = (min_log + normalized.clamp(0.0, 1.0) * log_range).exp();
    }

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);

    let edited = knob_value_edit(
        ui, edit_id, value_hz, min_hz, max_hz, 0.0, format_hz, font_id, text_color,
    );

    if edited || response.changed() {
        control.set_param(param, *value_hz);
    }
}

pub fn param_knob_filter_cutoff(
    ui: &mut egui::Ui,
    label: &str,
    value_hz: &mut f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    let max_raw = f32::from(FILTER_CUTOFF_RAW_MAX);
    let min_hz = cutoff_raw_to_hz(0);
    let max_hz = cutoff_raw_to_hz(FILTER_CUTOFF_RAW_MAX);
    let reset_raw = max_raw;
    let mut raw = f32::from(cutoff_hz_to_raw(*value_hz, FILTER_CUTOFF_RAW_MAX));
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut raw, 0.0, max_raw, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_raw)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );

    if response.changed() {
        raw = raw.round().clamp(0.0, max_raw);
        *value_hz = cutoff_raw_to_hz(raw as u16);
    }

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);
    let edited =
        knob_filter_cutoff_hz_edit(ui, edit_id, value_hz, min_hz, max_hz, font_id, text_color);
    if edited {
        *value_hz = cutoff_raw_to_hz(cutoff_hz_to_raw(*value_hz, FILTER_CUTOFF_RAW_MAX));
    }

    ui.label(
        egui::RichText::new(filter_cutoff_raw_to_note_name(f32::from(cutoff_hz_to_raw(
            *value_hz,
            FILTER_CUTOFF_RAW_MAX,
        ))))
        .font(egui::FontId::proportional(KNOB_FONT_SIZE - 1.0))
        .color(text_color.gamma_multiply(0.75)),
    );

    if edited || response.changed() {
        control.set_param(param, *value_hz);
    }
}

pub fn param_knob_filter_key_amount(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    param: ParamId,
    control: &SynthEngineControl,
) {
    let max_raw = f32::from(FILTER_KEY_TRACK_RAW_MAX);
    let mut raw = f32::from(key_track_to_raw(*value));
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut raw, 0.0, max_raw, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(0.0)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );
    if response.changed() {
        raw = raw.round().clamp(0.0, max_raw);
        *value = key_track_from_raw(raw as u16);
    }

    ui.add_space(KNOB_LABEL_OVERLAP);
    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);
    let format_raw = format_knob_value(0.0, max_raw);
    let edited = knob_value_edit(
        ui,
        edit_id,
        &mut raw,
        0.0,
        max_raw,
        0.0,
        &format_raw,
        font_id,
        text_color,
    );
    if edited {
        raw = raw.round().clamp(0.0, max_raw);
        *value = key_track_from_raw(raw as u16);
    }

    if edited || response.changed() {
        control.set_param(param, *value);
    }
}

pub fn param_knob_discrete(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut usize,
    names: &[&str],
    reset_value: usize,
    param: ParamId,
    control: &SynthEngineControl,
) {
    if names.is_empty() {
        return;
    }
    let maximum = names.len() - 1;
    *value = (*value).min(maximum);
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let drag_id = discrete_knob_drag_id(param);
    let cached_drag = ui.memory_mut(|memory| memory.data.get_temp::<(f32, usize)>(drag_id));
    let mut knob_value = match cached_drag {
        Some((drag_value, snapped_value)) if snapped_value == *value => drag_value,
        _ => *value as f32,
    };
    let previous_value = *value;

    ui.spacing_mut().item_spacing.y = 0.0;
    let response = ui.add(
        Knob::new(&mut knob_value, 0.0, maximum as f32, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value.min(maximum) as f32)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );
    let snapped_value = round_to_usize(knob_value).min(maximum);
    if snapped_value != previous_value {
        *value = snapped_value;
        control.set_param(param, *value as f32);
    }
    let retained_drag_value = if response.dragged() {
        knob_value
    } else {
        *value as f32
    };
    ui.memory_mut(|memory| {
        memory
            .data
            .insert_temp(drag_id, (retained_drag_value, *value));
    });

    ui.add_space(KNOB_LABEL_OVERLAP);
    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );
    ui.label(
        egui::RichText::new(names[*value])
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );
}

fn round_to_usize(value: f32) -> usize {
    value.round().max(0.0) as usize
}

pub fn param_toggle(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut bool,
    param: ParamId,
    control: &SynthEngineControl,
) {
    if framed_selectable(ui, *value, label).clicked() {
        *value = !*value;
        control.set_param(param, if *value { 1.0 } else { 0.0 });
    }
}

pub fn param_toggle_sized(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    label: &str,
    value: &mut bool,
    param: ParamId,
    control: &SynthEngineControl,
) {
    if framed_selectable_sized(ui, size, *value, label).clicked() {
        *value = !*value;
        control.set_param(param, if *value { 1.0 } else { 0.0 });
    }
}

pub fn framed_selectable<'a>(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    ui.add(egui::Button::selectable(selected, label).frame_when_inactive(true))
}

pub fn framed_selectable_sized<'a>(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    selected: bool,
    label: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::selectable(selected, label).frame_when_inactive(true),
    )
}

pub fn master_volume(
    ui: &mut egui::Ui,
    value: &mut f32,
    control: &SynthEngineControl,
    echo_midi: bool,
) {
    let mut knob_value = *value;
    let text_color = ui.visuals().text_color();
    let knob_color = ui.visuals().widgets.inactive.fg_stroke.color;
    let accent = ui.visuals().selection.bg_fill;
    let font_id = egui::FontId::proportional(MASTER_FONT_SIZE);

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        ui.label(
            egui::RichText::new("Master")
                .font(font_id.clone())
                .color(text_color),
        );

        let knob_visual = MASTER_KNOB_SIZE + MASTER_KNOB_STROKE * 2.0;
        let (knob_rect, _) =
            ui.allocate_exact_size(egui::vec2(knob_visual, knob_visual), egui::Sense::hover());
        let knob_response = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        knob_rect.min,
                        egui::vec2(knob_visual, knob_visual + 16.0),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
                |ui| {
                    ui.add(
                        Knob::new(&mut knob_value, 0.0, 1.0, KnobStyle::Wiper)
                            .with_size(MASTER_KNOB_SIZE)
                            .with_stroke_width(MASTER_KNOB_STROKE)
                            .with_colors(knob_color, accent, text_color)
                            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
                            .with_double_click_reset(1.0)
                            .with_background_arc(false)
                            .with_show_filled_segments(false),
                    )
                },
            )
            .inner;
        if knob_response.changed() {
            *value = knob_value;
            if echo_midi {
                control.set_master_volume(*value);
            }
        }

        let edit_id = egui::Id::new("knob_txt_master_volume");

        let mut edit_text = ui
            .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
            .unwrap_or_default();

        let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));
        if !edit_has_focus {
            edit_text = format_master_volume(*value);
        }

        if edit_text.is_empty() {
            edit_text = format_master_volume(*value);
        }

        let edit_response = ui.add(
            egui::TextEdit::singleline(&mut edit_text)
                .id(edit_id)
                .font(font_id)
                .desired_width(56.0)
                .margin(egui::Margin::ZERO)
                .horizontal_align(egui::Align::Center)
                .frame(egui::Frame::NONE)
                .text_color(text_color),
        );

        let apply = edit_response.lost_focus() && !edit_text.trim().is_empty();
        if apply {
            if let Some(new_linear) = parse_master_volume(&edit_text) {
                if (*value - new_linear).abs() > f32::EPSILON {
                    *value = new_linear;
                    if echo_midi {
                        control.set_master_volume(*value);
                    }
                }
                edit_text = format_master_volume(*value);
            } else {
                edit_text = format_master_volume(*value);
            }
        }

        ui.memory_mut(|mem| mem.data.insert_temp(edit_id, edit_text));
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

fn format_filter_cutoff_hz(value: f32) -> String {
    if value >= 1000.0 {
        format!("{:.2} kHz", value / 1000.0)
    } else if value >= 100.0 {
        format!("{value:.0} Hz")
    } else if value >= 10.0 {
        format!("{value:.1} Hz")
    } else {
        format!("{value:.2} Hz")
    }
}

fn parse_hz_text(text: &str) -> Option<f32> {
    let trimmed = text.trim().to_ascii_lowercase();
    let (number, scale) = if let Some(rest) = trimmed.strip_suffix("khz") {
        (rest.trim(), 1000.0)
    } else if let Some(rest) = trimmed.strip_suffix("hz") {
        (rest.trim(), 1.0)
    } else {
        (trimmed.as_str(), 1.0)
    };
    Some(number.parse::<f32>().ok()? * scale)
}

fn knob_filter_cutoff_hz_edit(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    value_hz: &mut f32,
    min_hz: f32,
    max_hz: f32,
    font_id: egui::FontId,
    text_color: egui::Color32,
) -> bool {
    let mut edit_text = ui
        .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
        .unwrap_or_default();

    let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));
    if !edit_has_focus {
        edit_text = format_filter_cutoff_hz(*value_hz);
    }
    if edit_text.is_empty() {
        edit_text = format_filter_cutoff_hz(*value_hz);
    }

    let edit_response = ui.add(
        egui::TextEdit::singleline(&mut edit_text)
            .id(edit_id)
            .font(font_id)
            .desired_width(64.0)
            .horizontal_align(egui::Align::Center)
            .frame(egui::Frame::NONE)
            .text_color(text_color),
    );

    let mut changed = false;
    if edit_response.lost_focus() && !edit_text.trim().is_empty() {
        if let Some(parsed) = parse_hz_text(&edit_text) {
            let clamped = parsed.clamp(min_hz, max_hz);
            if (*value_hz - clamped).abs() > f32::EPSILON {
                *value_hz = clamped;
                changed = true;
            }
        }
        edit_text = format_filter_cutoff_hz(*value_hz);
    }

    ui.memory_mut(|mem| mem.data.insert_temp(edit_id, edit_text));
    changed
}

fn format_knob_value(min: f32, max: f32) -> impl Fn(f32) -> String + 'static {
    move |value| {
        if min < 0.0 && max <= 1.0 {
            format!("{value:+.2}")
        } else if max >= 16.0 {
            format!("{value:.0}")
        } else if max > 2.0 {
            format!("{value:.2}")
        } else {
            format!("{value:.2}")
        }
    }
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn value_to_note_name(value: f32) -> String {
    let v = value.clamp(0.0, 120.0) as i32;
    let note = NOTE_NAMES[(v % 12) as usize];
    let octave = v / 12 - 2;
    format!("{note}{octave}")
}

fn filter_cutoff_raw_to_note_name(raw: f32) -> String {
    let midi = raw.round() as i32 - 36;
    let note = NOTE_NAMES[midi.rem_euclid(12) as usize];
    let octave = midi.div_euclid(12) - 1;
    format!("{note}{octave}")
}

fn parse_note_name(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let first = trimmed.chars().next()?.to_ascii_uppercase();
    let note_idx = NOTE_NAMES.iter().position(|&n| n.starts_with(first))?;

    let mut pos = 1;
    let accidental = if trimmed.len() > pos {
        match trimmed.chars().nth(pos) {
            Some('#') => {
                pos += 1;
                1
            }
            Some('b') => {
                pos += 1;
                -1
            }
            _ => 0,
        }
    } else {
        0
    };

    let note_with_accidental = (note_idx as i32 + accidental).rem_euclid(12) as usize;
    let octave_str: String = trimmed[pos..]
        .chars()
        .take_while(|c| *c == '-' || c.is_ascii_digit())
        .collect();

    let octave: i32 = octave_str.parse().ok()?;
    let value = (octave + 2) * 12 + note_with_accidental as i32;

    if (0..=120).contains(&value) {
        Some(value as f32)
    } else {
        None
    }
}

fn knob_note_edit(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    font_id: egui::FontId,
    text_color: egui::Color32,
) -> bool {
    let mut edit_text = ui
        .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
        .unwrap_or_default();

    let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));

    if !edit_has_focus {
        edit_text = value_to_note_name(*value);
    }

    if edit_text.is_empty() {
        edit_text = value_to_note_name(*value);
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
        let new_val = parse_note_name(&edit_text)
            .or_else(|| edit_text.trim().parse::<f32>().ok())
            .unwrap_or(*value)
            .clamp(min, max);

        if (*value - new_val).abs() > f32::EPSILON {
            *value = new_val;
            changed = true;
        }
        edit_text = value_to_note_name(*value);
    }

    ui.memory_mut(|mem| mem.data.insert_temp(edit_id, edit_text));
    changed
}

pub fn param_knob_note(
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
    let mut knob_value = *value;

    ui.spacing_mut().item_spacing.y = 0.0;

    let response = ui.add(
        Knob::new(&mut knob_value, min, max, KnobStyle::Wiper)
            .with_size(KNOB_SIZE)
            .with_stroke_width(2.0)
            .with_colors(knob_color, accent, text_color)
            .with_sweep_range(KNOB_SWEEP_START, KNOB_SWEEP_RANGE)
            .with_double_click_reset(reset_value)
            .with_background_arc(true)
            .with_show_filled_segments(true),
    );
    if response.changed() {
        *value = knob_value;
    }

    ui.add_space(KNOB_LABEL_OVERLAP);

    ui.label(
        egui::RichText::new(label)
            .font(egui::FontId::proportional(KNOB_FONT_SIZE))
            .color(text_color),
    );

    let edit_id = knob_edit_id(param);
    let font_id = egui::FontId::proportional(KNOB_FONT_SIZE);

    let edited = knob_note_edit(ui, edit_id, value, min, max, font_id, text_color);

    if edited || response.changed() {
        control.set_param(param, *value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::create_synth_engine_bridge;

    #[test]
    fn idle_knobs_do_not_change_parameter_bits() {
        let (_audio, bridge) = create_synth_engine_bridge(1);
        let mut envelope = 1.0635071_f32;
        let mut frequency = 0.26123878_f32;
        let mut note = 61.23457_f32;
        let mut master = 0.08661418_f32;
        let expected = [
            envelope.to_bits(),
            frequency.to_bits(),
            note.to_bits(),
            master.to_bits(),
        ];

        egui::__run_test_ui(|ui| {
            param_knob_f32(
                ui,
                "Attack",
                &mut envelope,
                0.0005..=5.0,
                0.0005,
                ParamId::FilterEgAttack,
                &bridge.control,
            );
            param_knob_log_hz(
                ui,
                "Frequency",
                &mut frequency,
                0.022,
                30.0,
                1.0,
                ParamId::Lfo1Rate,
                &bridge.control,
            );
            param_knob_note(
                ui,
                "Note",
                &mut note,
                0.0..=120.0,
                60.0,
                ParamId::Osc1Frequency,
                &bridge.control,
            );
            master_volume(ui, &mut master, &bridge.control, true);
        });

        assert_eq!(
            [
                envelope.to_bits(),
                frequency.to_bits(),
                note.to_bits(),
                master.to_bits(),
            ],
            expected
        );
    }
}
