use eframe::egui;
use egui_knob::{Knob, KnobStyle};
use synth_core::ParamId;

use crate::engine::SynthEngineControl;

pub const KNOB_SIZE: f32 = 32.0;
pub const MASTER_KNOB_SIZE: f32 = 22.0;
pub const MASTER_FONT_SIZE: f32 = 12.0;
const MASTER_KNOB_STROKE: f32 = 1.5;
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

    let edit_id = knob_edit_id(param);
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

    if edited || response.changed() {
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
                control.set_param(ParamId::MasterVolume, *value);
            }
        }

        let edit_id = egui::Id::new("knob_txt_master_volume");

        let mut edit_text = ui
            .memory_mut(|mem| mem.data.get_temp::<String>(edit_id))
            .unwrap_or_default();

        let edit_has_focus = ui.memory(|mem| mem.has_focus(edit_id));
        if !edit_has_focus {
            edit_text = format!("{:.2}", *value);
        }

        if edit_text.is_empty() {
            edit_text = format!("{:.2}", *value);
        }

        let edit_response = ui.add(
            egui::TextEdit::singleline(&mut edit_text)
                .id(edit_id)
                .font(font_id)
                .desired_width(36.0)
                .margin(egui::Margin::ZERO)
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
                    if echo_midi {
                        control.set_param(ParamId::MasterVolume, *value);
                    }
                }
                edit_text = format!("{:.2}", *value);
            } else {
                edit_text = format!("{:.2}", *value);
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
