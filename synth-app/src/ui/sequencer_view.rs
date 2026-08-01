use eframe::egui;
use std::ops::Range;

use synth_core::{
    GatedDestination, GatedSequencerMode, GatedStep, LayerId, LayerPlaybackStatus, LayerTarget,
    ModDestination, Patch, PolyLaneStep, PolyNote, PolyVelocity, SequenceClear, SequenceUpdate,
    SequencerFeedback, SequencerRecordCommand, SequencerType,
};

use crate::engine::{SequencerPlaybackStatus, SynthEngineControl};
use crate::ui::params_view::{UiState, layer_control_bar};

const TRANSPORT_BUTTON_WIDTH: f32 = 72.0;
const TYPE_SEGMENT_WIDTH: f32 = 84.0;
const LANE_NUMBER_WIDTH: f32 = 26.0;
const FIELD_LABEL_WIDTH: f32 = 52.0;
const POLY_CELL_WIDTH: f32 = 48.0;
const POLY_CELL_GAP: f32 = 2.0;
const POLY_STEP_COUNT: usize = 64;
const POLY_GRID_LEADING_WIDTH: f32 = LANE_NUMBER_WIDTH + FIELD_LABEL_WIDTH + POLY_CELL_GAP * 2.0;
const POLY_CELL_STRIDE: f32 = POLY_CELL_WIDTH + POLY_CELL_GAP;
const POLY_GRID_WIDTH: f32 = POLY_GRID_LEADING_WIDTH
    + POLY_CELL_WIDTH * POLY_STEP_COUNT as f32
    + POLY_CELL_GAP * (POLY_STEP_COUNT - 1) as f32;
const POLY_GRID_OVERSCAN: isize = 1;

#[cfg(test)]
std::thread_local! {
    static POLY_CELL_RENDER_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub struct SequencerViewState {
    recording: [bool; 2],
    cursor: [u8; 2],
    gated_position: [u8; 2],
    overflow: [bool; 2],
    poly_cell_editing: [bool; 2],
    confirm_clear_gated: bool,
    confirm_clear_poly: bool,
}

impl SequencerViewState {
    pub fn recording(&self, layer: LayerId) -> bool {
        self.recording[layer_index(layer)]
    }

    pub fn position(&self, layer: LayerId, sequencer_type: SequencerType) -> u8 {
        let index = layer_index(layer);
        match sequencer_type {
            SequencerType::Gated => self.gated_position[index],
            SequencerType::Polyphonic => self.cursor[index],
        }
    }

    pub fn apply_feedback(&mut self, feedback: SequencerFeedback) {
        match feedback {
            SequencerFeedback::RecordStatus {
                layer,
                recording,
                cursor,
            } => {
                let index = layer_index(layer);
                self.recording[index] = recording;
                self.cursor[index] = cursor;
                if recording {
                    self.overflow[index] = false;
                }
            }
            SequencerFeedback::RecordOverflow { layer, cursor } => {
                let index = layer_index(layer);
                self.cursor[index] = cursor;
                self.overflow[index] = true;
            }
            SequencerFeedback::StepChanged { layer, step, .. } => {
                let index = layer_index(layer);
                self.cursor[index] = (step + 1) % 64;
            }
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut SequencerViewState,
    patch: &mut Patch,
    edit_layer: &mut LayerId,
    parameter_state: &mut UiState,
    control: &SynthEngineControl,
    layer_playback: LayerPlaybackStatus,
    playback: [SequencerPlaybackStatus; 2],
) -> bool {
    let layer_changed = layer_control_bar(
        ui,
        parameter_state,
        patch,
        edit_layer,
        layer_playback,
        control,
    );
    if layer_changed {
        state.confirm_clear_gated = false;
        state.confirm_clear_poly = false;
    }

    ui.add_space(8.0);
    let playback = playback[layer_index(*edit_layer)];
    sequencer_control_bar(ui, state, patch, *edit_layer, control, playback);

    ui.add_space(8.0);
    let sequence_type = patch.layer(*edit_layer).sequence.sequencer_type;
    match sequence_type {
        SequencerType::Gated => gated_editor(ui, state, patch, *edit_layer, control, playback),
        SequencerType::Polyphonic => poly_editor(ui, state, patch, *edit_layer, control, playback),
    }
    layer_changed
}

pub(crate) fn sequencer_control_bar(
    ui: &mut egui::Ui,
    state: &mut SequencerViewState,
    patch: &mut Patch,
    layer: LayerId,
    control: &SynthEngineControl,
    playback: SequencerPlaybackStatus,
) {
    let index = layer_index(layer);
    let layer_mode = patch.mode;
    let available_width = ui.available_width();
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width((available_width - 20.0).max(0.0));
            ui.horizontal_wrapped(|ui| {
                let row_height = ui.spacing().interact_size.y;
                ui.allocate_ui_with_layout(
                    egui::vec2(72.0, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.strong("Sequencer");
                    },
                );
                ui.separator();
                let sequence = &mut patch.layer_mut(layer).sequence;
                let previous_type = sequence.sequencer_type;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (sequencer_type, label, corner_radius) in [
                        (
                            SequencerType::Gated,
                            "Gated",
                            egui::CornerRadius {
                                nw: 4,
                                ne: 0,
                                sw: 4,
                                se: 0,
                            },
                        ),
                        (
                            SequencerType::Polyphonic,
                            "Polyphonic",
                            egui::CornerRadius {
                                nw: 0,
                                ne: 4,
                                sw: 0,
                                se: 4,
                            },
                        ),
                    ] {
                        let selected = sequence.sequencer_type == sequencer_type;
                        let mut button = egui::Button::selectable(selected, label)
                            .min_size(egui::vec2(TYPE_SEGMENT_WIDTH, 0.0))
                            .corner_radius(corner_radius);
                        if !selected {
                            button = button.fill(egui::Color32::from_gray(80));
                        }
                        if ui.add(button).clicked() {
                            sequence.sequencer_type = sequencer_type;
                        }
                    }
                });
                if sequence.sequencer_type != previous_type {
                    control.set_sequence(
                        LayerTarget::Explicit(layer),
                        SequenceUpdate::Type(sequence.sequencer_type),
                    );
                }

                ui.separator();
                match sequence.sequencer_type {
                    SequencerType::Gated => {
                        ui.label("Mode");
                        let previous_mode = sequence.gated_mode;
                        egui::ComboBox::from_id_salt("sequencer_bar_gated_mode")
                            .selected_text(gated_mode_name(sequence.gated_mode))
                            .show_ui(ui, |ui| {
                                for mode in [
                                    GatedSequencerMode::Normal,
                                    GatedSequencerMode::NoReset,
                                    GatedSequencerMode::NoGate,
                                    GatedSequencerMode::NoGateNoReset,
                                    GatedSequencerMode::KeyStep,
                                ] {
                                    ui.selectable_value(
                                        &mut sequence.gated_mode,
                                        mode,
                                        gated_mode_name(mode),
                                    );
                                }
                            });
                        if sequence.gated_mode != previous_mode {
                            control.set_sequence(
                                LayerTarget::Explicit(layer),
                                SequenceUpdate::GatedMode(sequence.gated_mode),
                            );
                        }
                    }
                    SequencerType::Polyphonic => {
                        let transport = ui.add(
                            egui::Button::new(if playback.running { "Stop" } else { "Play" })
                                .min_size(egui::vec2(TRANSPORT_BUTTON_WIDTH, 0.0)),
                        );
                        if transport.clicked() {
                            control.set_patch_sequencers_running(
                                layer_mode,
                                layer,
                                !playback.running,
                            );
                        }

                        let recording = state.recording(layer);
                        let mut record_button = egui::Button::selectable(recording, "Record")
                            .min_size(egui::vec2(TRANSPORT_BUTTON_WIDTH, 0.0));
                        if !recording {
                            record_button = record_button.fill(egui::Color32::from_gray(80));
                        }
                        if ui.add(record_button).clicked() {
                            control
                                .set_sequencer_recording(LayerTarget::Explicit(layer), !recording);
                        }
                    }
                }

                ui.separator();
                ui.label("Position");
                let position_count = match sequence.sequencer_type {
                    SequencerType::Gated => 16,
                    SequencerType::Polyphonic => 64,
                };
                let position = state
                    .position(layer, sequence.sequencer_type)
                    .min(position_count - 1);
                if ui.button("<").clicked() {
                    let next = offset_position(position, position_count, -1);
                    set_position(state, layer, sequence.sequencer_type, next, control);
                }
                ui.monospace(format!("{:02}", position + 1));
                if ui.button(">").clicked() {
                    let next = offset_position(position, position_count, 1);
                    set_position(state, layer, sequence.sequencer_type, next, control);
                }
                if state.overflow[index] {
                    ui.colored_label(ui.visuals().warn_fg_color, "Chord limited to six notes");
                }
            });
        });
}

fn set_position(
    state: &mut SequencerViewState,
    layer: LayerId,
    sequencer_type: SequencerType,
    position: u8,
    control: &SynthEngineControl,
) {
    let index = layer_index(layer);
    match sequencer_type {
        SequencerType::Gated => state.gated_position[index] = position.min(15),
        SequencerType::Polyphonic => {
            state.cursor[index] = position.min(63);
            control.sequencer_record_command(
                LayerTarget::Explicit(layer),
                SequencerRecordCommand::SetCursor(position),
            );
        }
    }
}

fn offset_position(position: u8, position_count: u8, delta: i8) -> u8 {
    (i16::from(position) + i16::from(delta)).rem_euclid(i16::from(position_count)) as u8
}

fn gated_editor(
    ui: &mut egui::Ui,
    state: &mut SequencerViewState,
    patch: &mut Patch,
    layer: LayerId,
    control: &SynthEngineControl,
    playback: SequencerPlaybackStatus,
) {
    ui.horizontal(|ui| {
        if !state.confirm_clear_gated {
            if ui.button("Clear gated sequence...").clicked() {
                state.confirm_clear_gated = true;
            }
        } else {
            ui.colored_label(ui.visuals().warn_fg_color, "Clear all four tracks?");
            if ui.button("Confirm clear").clicked() {
                patch.layer_mut(layer).sequence.gated = synth_core::GatedSequence::default();
                control.clear_sequence(LayerTarget::Explicit(layer), SequenceClear::Gated);
                state.confirm_clear_gated = false;
            }
            if ui.button("Cancel").clicked() {
                state.confirm_clear_gated = false;
            }
        }
    });

    ui.label(
        "Values 0-125; 126 = < (Reset); 127 = - (Rest). Track 1 supplies the shared envelope gate.",
    );
    egui::ScrollArea::horizontal()
        .id_salt("gated_grid_scroll")
        .show(ui, |ui| {
            egui::Grid::new("gated_grid")
                .striped(true)
                .spacing([6.0, 5.0])
                .show(ui, |ui| {
                    ui.strong("Track");
                    ui.strong("Destination");
                    for step in 0..16 {
                        position_header(
                            ui,
                            step,
                            state.position(layer, SequencerType::Gated) == step as u8,
                            playback.active_step == Some(step as u8),
                            true,
                            POLY_CELL_WIDTH,
                        );
                    }
                    ui.end_row();

                    for track in 0..4 {
                        ui.strong(format!("{}", track + 1));
                        gated_destination(ui, patch, layer, track, control);
                        for step in 0..16 {
                            let current =
                                patch.layer(layer).sequence.gated.tracks[track].steps[step];
                            let mut raw = i32::from(current.rev2_raw());
                            let response = ui.add_sized(
                                [POLY_CELL_WIDTH, ui.spacing().interact_size.y],
                                egui::DragValue::new(&mut raw)
                                    .range(0..=127)
                                    .speed(0.2)
                                    .custom_formatter(|value, _| gated_value_label(value as u16))
                                    .custom_parser(parse_gated_value),
                            );
                            if response.changed() {
                                let value = GatedStep::from_rev2_raw(raw as u16);
                                patch.layer_mut(layer).sequence.gated.tracks[track].steps[step] =
                                    value;
                                control.set_sequence(
                                    LayerTarget::Explicit(layer),
                                    SequenceUpdate::GatedStep {
                                        track: track as u8,
                                        step: step as u8,
                                        value,
                                    },
                                );
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

fn gated_destination(
    ui: &mut egui::Ui,
    patch: &mut Patch,
    layer: LayerId,
    track: usize,
    control: &SynthEngineControl,
) {
    let current = patch.layer(layer).sequence.gated.tracks[track].destination;
    let mut selected = current;
    egui::ComboBox::from_id_salt(("gated_destination", track))
        .width(130.0)
        .selected_text(gated_destination_name(current))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected, GatedDestination::Off, "Off");
            if matches!(track, 1 | 3) {
                ui.selectable_value(&mut selected, GatedDestination::Slew, "Slew previous track");
            }
            for destination in ModDestination::ALL {
                if destination != ModDestination::Off {
                    ui.selectable_value(
                        &mut selected,
                        GatedDestination::Modulation(destination),
                        destination.name(),
                    );
                }
            }
        });
    if selected != current {
        patch.layer_mut(layer).sequence.gated.tracks[track].destination = selected;
        control.set_sequence(
            LayerTarget::Explicit(layer),
            SequenceUpdate::GatedDestination {
                track: track as u8,
                destination: selected,
            },
        );
    }
}

fn poly_editor(
    ui: &mut egui::Ui,
    state: &mut SequencerViewState,
    patch: &mut Patch,
    layer: LayerId,
    control: &SynthEngineControl,
    playback: SequencerPlaybackStatus,
) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Rest").clicked() {
            control.sequencer_record_command(
                LayerTarget::Explicit(layer),
                SequencerRecordCommand::InsertRest,
            );
        }
        if ui.button("Tie").clicked() {
            control.sequencer_record_command(
                LayerTarget::Explicit(layer),
                SequencerRecordCommand::InsertTie,
            );
        }
        if ui.button("End / Reset").clicked() {
            control.sequencer_record_command(
                LayerTarget::Explicit(layer),
                SequencerRecordCommand::InsertReset,
            );
        }
        if ui.button("Clear cursor step").clicked() {
            control.sequencer_record_command(
                LayerTarget::Explicit(layer),
                SequencerRecordCommand::ClearStep,
            );
        }
        if !state.confirm_clear_poly {
            if ui.button("Clear poly sequence...").clicked() {
                state.confirm_clear_poly = true;
            }
        } else {
            ui.colored_label(ui.visuals().warn_fg_color, "Clear all 64 steps?");
            if ui.button("Confirm clear").clicked() {
                patch.layer_mut(layer).sequence.poly = synth_core::PolySequence::default();
                control.clear_sequence(LayerTarget::Explicit(layer), SequenceClear::Polyphonic);
                state.confirm_clear_poly = false;
            }
            if ui.button("Cancel").clicked() {
                state.confirm_clear_poly = false;
            }
        }
    });
    ui.label("Event: note, = (Tie), - (Rest), or < (Reset). Velocity is editable for notes.");

    let viewport_width = ui.available_width();
    let mut scroll =
        egui::ScrollArea::horizontal().id_salt(("poly_grid_scroll", layer_index(layer)));
    let index = layer_index(layer);
    let cell_has_focus = state.poly_cell_editing[index];
    if playback.running
        && !cell_has_focus
        && let Some(step) = playback.active_step
    {
        scroll = scroll.horizontal_scroll_offset(poly_follow_offset(step, viewport_width));
    }
    let mut cell_editing = false;
    scroll.show_viewport(ui, |ui, viewport| {
        ui.set_min_width(POLY_GRID_WIDTH);
        ui.spacing_mut().item_spacing = egui::vec2(POLY_CELL_GAP, 3.0);
        let visible_steps = poly_visible_step_range(viewport);
        poly_position_headers(ui, state, layer, playback, visible_steps.clone());
        for lane in 0..6 {
            poly_lane_rows(
                ui,
                patch,
                layer,
                lane,
                control,
                visible_steps.clone(),
                &mut cell_editing,
            );
        }
    });
    state.poly_cell_editing[index] = cell_editing;
}

fn poly_position_headers(
    ui: &mut egui::Ui,
    state: &SequencerViewState,
    layer: LayerId,
    playback: SequencerPlaybackStatus,
    visible_steps: Range<usize>,
) {
    ui.horizontal(|ui| {
        ui.add_space(POLY_GRID_LEADING_WIDTH);
        let selected = state.position(layer, SequencerType::Polyphonic);
        ui.add_space(visible_steps.start as f32 * POLY_CELL_STRIDE);
        for step in visible_steps {
            position_header(
                ui,
                step,
                selected == step as u8,
                playback.running && playback.active_step == Some(step as u8),
                false,
                POLY_CELL_WIDTH,
            );
        }
    });
}

fn poly_lane_rows(
    ui: &mut egui::Ui,
    patch: &mut Patch,
    layer: LayerId,
    lane: usize,
    control: &SynthEngineControl,
    visible_steps: Range<usize>,
    cell_editing: &mut bool,
) {
    let row_height = ui.spacing().interact_size.y;
    let rows_height = row_height * 2.0 + ui.spacing().item_spacing.y;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LANE_NUMBER_WIDTH, rows_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space((rows_height - row_height) * 0.5);
                ui.strong(format!("{}", lane + 1));
            },
        );
        ui.vertical(|ui| {
            poly_editor_row(ui, "Event", visible_steps.clone(), |ui, step| {
                poly_event_cell(ui, patch, layer, step, lane, control, cell_editing);
            });
            poly_editor_row(ui, "Velocity", visible_steps, |ui, step| {
                poly_velocity_cell(ui, patch, layer, step, lane, control, cell_editing);
            });
        });
    });
}

fn poly_editor_row(
    ui: &mut egui::Ui,
    label: &str,
    visible_steps: Range<usize>,
    mut add_cell: impl FnMut(&mut egui::Ui, usize),
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [FIELD_LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(label),
        );
        ui.add_space(visible_steps.start as f32 * POLY_CELL_STRIDE);
        for step in visible_steps {
            add_cell(ui, step);
        }
    });
}

fn poly_visible_step_range(viewport: egui::Rect) -> Range<usize> {
    let relative_min = (viewport.min.x - POLY_GRID_LEADING_WIDTH) / POLY_CELL_STRIDE;
    let relative_max = (viewport.max.x - POLY_GRID_LEADING_WIDTH) / POLY_CELL_STRIDE;
    let start = (relative_min.floor() as isize - POLY_GRID_OVERSCAN)
        .clamp(0, POLY_STEP_COUNT as isize) as usize;
    let end = (relative_max.ceil() as isize + POLY_GRID_OVERSCAN)
        .clamp(start as isize, POLY_STEP_COUNT as isize) as usize;
    start..end
}

fn poly_follow_offset(step: u8, viewport_width: f32) -> f32 {
    let center = POLY_GRID_LEADING_WIDTH
        + f32::from(step.min((POLY_STEP_COUNT - 1) as u8)) * POLY_CELL_STRIDE
        + POLY_CELL_WIDTH * 0.5;
    (center - viewport_width * 0.5).clamp(0.0, (POLY_GRID_WIDTH - viewport_width).max(0.0))
}

fn position_header(
    ui: &mut egui::Ui,
    step: usize,
    selected: bool,
    playing: bool,
    follow_playback: bool,
    column_width: f32,
) {
    let text = position_header_label(step, selected);
    let response = ui.allocate_ui_with_layout(
        egui::vec2(column_width, ui.spacing().interact_size.y),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let mut text = egui::RichText::new(text).monospace();
            if playing {
                text = text.color(ui.visuals().selection.stroke.color);
            }
            ui.label(text);
        },
    );
    if playing && follow_playback {
        response.response.scroll_to_me(Some(egui::Align::Center));
    }
}

fn position_header_label(step: usize, selected: bool) -> String {
    if selected {
        format!("[ {:02} ]", step + 1)
    } else {
        format!("  {:02}  ", step + 1)
    }
}

fn poly_event_cell(
    ui: &mut egui::Ui,
    patch: &mut Patch,
    layer: LayerId,
    step: usize,
    lane: usize,
    control: &SynthEngineControl,
    cell_editing: &mut bool,
) {
    #[cfg(test)]
    POLY_CELL_RENDER_COUNT.with(|count| count.set(count.get() + 1));
    let lane_step = patch.layer(layer).sequence.poly.steps[step].lanes[lane];
    let mut raw = i32::from(poly_event_raw(lane_step));
    let response = ui
        .push_id(("poly_event", layer_index(layer), lane, step), |ui| {
            ui.add_sized(
                [POLY_CELL_WIDTH, ui.spacing().interact_size.y],
                egui::DragValue::new(&mut raw)
                    .range(0..=130)
                    .speed(0.2)
                    .custom_formatter(|value, _| poly_event_label(value as u16))
                    .custom_parser(parse_poly_event),
            )
        })
        .inner;
    *cell_editing |= response.has_focus() || response.dragged();
    if response.changed() {
        let value = poly_lane_for_event(lane_step, raw as u16);
        patch.layer_mut(layer).sequence.poly.steps[step].lanes[lane] = value;
        control.set_sequence(
            LayerTarget::Explicit(layer),
            SequenceUpdate::PolyLaneStep {
                step: step as u8,
                lane: lane as u8,
                value,
            },
        );
    }
}

fn poly_velocity_cell(
    ui: &mut egui::Ui,
    patch: &mut Patch,
    layer: LayerId,
    step: usize,
    lane: usize,
    control: &SynthEngineControl,
    cell_editing: &mut bool,
) {
    #[cfg(test)]
    POLY_CELL_RENDER_COUNT.with(|count| count.set(count.get() + 1));
    let lane_step = patch.layer(layer).sequence.poly.steps[step].lanes[lane];
    let current = lane_step.velocity;
    let mut raw = i32::from(current.rev2_raw());
    if !poly_velocity_enabled(lane_step) {
        ui.push_id(("poly_velocity", layer_index(layer), lane, step), |ui| {
            ui.add_enabled_ui(false, |ui| {
                ui.add_sized(
                    [POLY_CELL_WIDTH, ui.spacing().interact_size.y],
                    egui::DragValue::new(&mut raw).custom_formatter(|_, _| String::new()),
                );
            });
        });
        return;
    }
    let response = ui
        .push_id(("poly_velocity", layer_index(layer), lane, step), |ui| {
            ui.add_sized(
                [POLY_CELL_WIDTH, ui.spacing().interact_size.y],
                egui::DragValue::new(&mut raw)
                    .range(129..=255)
                    .speed(0.5)
                    .custom_formatter(|value, _| poly_velocity_label(value as u16))
                    .custom_parser(parse_poly_velocity),
            )
        })
        .inner;
    *cell_editing |= response.has_focus() || response.dragged();
    if response.changed() {
        let value = PolyVelocity::from_rev2_raw(raw as u16);
        patch.layer_mut(layer).sequence.poly.steps[step].lanes[lane].velocity = value;
        control.set_sequence(
            LayerTarget::Explicit(layer),
            SequenceUpdate::PolyVelocity {
                step: step as u8,
                lane: lane as u8,
                value,
            },
        );
    }
}

fn poly_velocity_enabled(step: PolyLaneStep) -> bool {
    matches!(
        step,
        PolyLaneStep {
            note: PolyNote::Note(_),
            velocity: PolyVelocity::Velocity(_),
        }
    )
}

fn layer_index(layer: LayerId) -> usize {
    match layer {
        LayerId::A => 0,
        LayerId::B => 1,
    }
}

fn gated_mode_name(mode: GatedSequencerMode) -> &'static str {
    match mode {
        GatedSequencerMode::Normal => "Normal",
        GatedSequencerMode::NoReset => "No Reset",
        GatedSequencerMode::NoGate => "No Gate",
        GatedSequencerMode::NoGateNoReset => "No Gate / No Reset",
        GatedSequencerMode::KeyStep => "Key Step",
    }
}

fn gated_destination_name(destination: GatedDestination) -> &'static str {
    match destination {
        GatedDestination::Off => "Off",
        GatedDestination::Slew => "Slew previous track",
        GatedDestination::Modulation(destination) => destination.name(),
    }
}

fn gated_value_label(raw: u16) -> String {
    match raw {
        126 => "<".to_owned(),
        127.. => "-".to_owned(),
        value => value.to_string(),
    }
}

fn parse_gated_value(text: &str) -> Option<f64> {
    let text = text.trim();
    if text == "<" || text.eq_ignore_ascii_case("reset") {
        return Some(126.0);
    }
    if text == "-" || text.eq_ignore_ascii_case("rest") {
        return Some(127.0);
    }
    text.parse::<u16>()
        .ok()
        .filter(|value| *value <= 127)
        .map(f64::from)
}

fn poly_note_label(raw: u16) -> String {
    if raw >= 128 {
        return "=".to_owned();
    }
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        names[usize::from(raw % 12)],
        i16::from(raw as u8) / 12 - 1
    )
}

fn poly_event_raw(step: PolyLaneStep) -> u16 {
    match step.velocity {
        PolyVelocity::Reset => 130,
        PolyVelocity::Rest => 129,
        PolyVelocity::Velocity(_) => step.note.rev2_raw(),
    }
}

fn poly_event_label(raw: u16) -> String {
    match raw {
        128 => "=".to_owned(),
        129 => "-".to_owned(),
        130 => "<".to_owned(),
        value => poly_note_label(value.min(127)),
    }
}

fn parse_poly_event(text: &str) -> Option<f64> {
    let text = text.trim();
    if text == "-" || text.eq_ignore_ascii_case("rest") {
        return Some(129.0);
    }
    if text == "<" || text.eq_ignore_ascii_case("reset") {
        return Some(130.0);
    }
    parse_poly_note(text)
}

fn poly_lane_for_event(current: PolyLaneStep, raw: u16) -> PolyLaneStep {
    match raw {
        0..=127 => PolyLaneStep {
            note: PolyNote::Note(raw as u8),
            velocity: numeric_velocity_or_default(current.velocity),
        },
        128 => PolyLaneStep {
            note: PolyNote::Tie,
            velocity: numeric_velocity_or_default(current.velocity),
        },
        129 => PolyLaneStep {
            velocity: PolyVelocity::Rest,
            ..current
        },
        _ => PolyLaneStep {
            velocity: PolyVelocity::Reset,
            ..current
        },
    }
}

fn numeric_velocity_or_default(velocity: PolyVelocity) -> PolyVelocity {
    match velocity {
        PolyVelocity::Velocity(_) => velocity,
        PolyVelocity::Reset | PolyVelocity::Rest => PolyVelocity::Velocity(127),
    }
}

fn parse_poly_note(text: &str) -> Option<f64> {
    let text = text.trim();
    if text == "=" || text.eq_ignore_ascii_case("tie") {
        return Some(128.0);
    }
    if let Ok(raw) = text.parse::<u8>() {
        return (raw <= 127).then_some(f64::from(raw));
    }

    let bytes = text.as_bytes();
    let note = *bytes.first()?;
    let semitone = match note.to_ascii_uppercase() {
        b'C' => 0_i16,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut octave_start = 1;
    let accidental = match bytes.get(1).copied() {
        Some(b'#') => {
            octave_start = 2;
            1_i16
        }
        Some(b'b') => {
            octave_start = 2;
            -1_i16
        }
        _ => 0,
    };
    let octave = text.get(octave_start..)?.parse::<i16>().ok()?;
    let midi_note = (octave + 1) * 12 + semitone + accidental;
    (0..=127)
        .contains(&midi_note)
        .then_some(f64::from(midi_note))
}

fn poly_velocity_label(raw: u16) -> String {
    match raw {
        0..=127 => "<".to_owned(),
        128 => "-".to_owned(),
        value => format!("{}", value.min(255) - 128),
    }
}

fn parse_poly_velocity(text: &str) -> Option<f64> {
    let text = text.trim();
    text.parse::<u16>()
        .ok()
        .filter(|velocity| (1..=127).contains(velocity))
        .map(|velocity| f64::from(velocity + 128))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_grid_labels_cover_minimum_maximum_and_special_values() {
        assert_eq!(gated_value_label(0), "0");
        assert_eq!(gated_value_label(125), "125");
        assert_eq!(gated_value_label(126), "<");
        assert_eq!(gated_value_label(127), "-");
        assert_eq!(poly_note_label(0), "C-1");
        assert_eq!(poly_note_label(127), "G9");
        assert_eq!(poly_note_label(128), "=");
        assert_eq!(poly_event_label(128), "=");
        assert_eq!(poly_event_label(129), "-");
        assert_eq!(poly_event_label(130), "<");
        assert_eq!(poly_velocity_label(0), "<");
        assert_eq!(poly_velocity_label(128), "-");
        assert_eq!(poly_velocity_label(255), "127");
        for label in [
            gated_value_label(126),
            gated_value_label(127),
            poly_note_label(61),
            poly_event_label(128),
            poly_event_label(129),
            poly_event_label(130),
            poly_velocity_label(0),
            poly_velocity_label(128),
            poly_velocity_label(255),
        ] {
            assert!(label.is_ascii(), "non-ASCII sequencer label: {label:?}");
        }
    }

    #[test]
    fn text_parsers_accept_notes_velocities_and_ascii_special_events() {
        assert_eq!(parse_gated_value("0"), Some(0.0));
        assert_eq!(parse_gated_value("125"), Some(125.0));
        assert_eq!(parse_gated_value("<"), Some(126.0));
        assert_eq!(parse_gated_value("-"), Some(127.0));
        assert_eq!(parse_gated_value("128"), None);

        assert_eq!(parse_poly_note("C-1"), Some(0.0));
        assert_eq!(parse_poly_note("C4"), Some(60.0));
        assert_eq!(parse_poly_note("c#4"), Some(61.0));
        assert_eq!(parse_poly_note("Db4"), Some(61.0));
        assert_eq!(parse_poly_note("G9"), Some(127.0));
        assert_eq!(parse_poly_note("127"), Some(127.0));
        assert_eq!(parse_poly_note("128"), None);
        assert_eq!(parse_poly_note("="), Some(128.0));
        assert_eq!(parse_poly_note("tie"), Some(128.0));
        assert_eq!(parse_poly_note("-"), None);
        assert_eq!(parse_poly_note("C10"), None);

        assert_eq!(parse_poly_event("C4"), Some(60.0));
        assert_eq!(parse_poly_event("="), Some(128.0));
        assert_eq!(parse_poly_event("-"), Some(129.0));
        assert_eq!(parse_poly_event("rest"), Some(129.0));
        assert_eq!(parse_poly_event("<"), Some(130.0));
        assert_eq!(parse_poly_event("reset"), Some(130.0));
        assert_eq!(parse_poly_event("129"), None);

        assert_eq!(parse_poly_velocity("<"), None);
        assert_eq!(parse_poly_velocity("reset"), None);
        assert_eq!(parse_poly_velocity("-"), None);
        assert_eq!(parse_poly_velocity("rest"), None);
        assert_eq!(parse_poly_velocity("1"), Some(129.0));
        assert_eq!(parse_poly_velocity("127"), Some(255.0));
        assert_eq!(parse_poly_velocity("0"), None);
        assert_eq!(parse_poly_velocity("128"), None);
        assert_eq!(parse_poly_velocity("="), None);
    }

    #[test]
    fn feedback_moves_cursor_and_tracks_overflow_per_layer() {
        let mut state = SequencerViewState::default();
        state.apply_feedback(SequencerFeedback::RecordStatus {
            layer: LayerId::B,
            recording: true,
            cursor: 63,
        });
        assert_eq!(state.cursor[1], 63);
        assert!(state.recording[1]);
        state.apply_feedback(SequencerFeedback::RecordOverflow {
            layer: LayerId::B,
            cursor: 63,
        });
        assert!(state.overflow[1]);
        assert!(!state.overflow[0]);
    }

    #[test]
    fn position_navigation_wraps_at_each_sequence_range() {
        assert_eq!(offset_position(0, 16, -1), 15);
        assert_eq!(offset_position(15, 16, 1), 0);
        assert_eq!(offset_position(0, 64, -1), 63);
        assert_eq!(offset_position(63, 64, 1), 0);
    }

    #[test]
    fn poly_grid_constructs_only_visible_columns_with_overscan() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(640.0, 400.0));
        assert_eq!(poly_visible_step_range(viewport), 0..13);

        let viewport =
            egui::Rect::from_min_max(egui::pos2(1_000.0, 0.0), egui::pos2(1_640.0, 400.0));
        assert_eq!(poly_visible_step_range(viewport), 17..33);

        let viewport =
            egui::Rect::from_min_max(egui::pos2(3_000.0, 0.0), egui::pos2(3_640.0, 400.0));
        assert_eq!(poly_visible_step_range(viewport), 57..64);
    }

    #[test]
    fn playback_follow_keeps_every_position_in_the_virtualized_range() {
        for viewport_width in [320.0, 640.0, 1_200.0] {
            for step in 0..POLY_STEP_COUNT as u8 {
                let offset = poly_follow_offset(step, viewport_width);
                let viewport = egui::Rect::from_min_max(
                    egui::pos2(offset, 0.0),
                    egui::pos2(offset + viewport_width, 400.0),
                );
                assert!(poly_visible_step_range(viewport).contains(&usize::from(step)));
            }
        }
        assert_eq!(poly_follow_offset(0, 640.0), 0.0);
        assert_eq!(poly_follow_offset(63, POLY_GRID_WIDTH * 2.0), 0.0);
    }

    #[test]
    fn gated_and_polyphonic_positions_are_independent_per_layer() {
        let mut state = SequencerViewState::default();
        state.cursor = [42, 63];
        state.gated_position = [7, 15];
        assert_eq!(state.position(LayerId::A, SequencerType::Polyphonic), 42);
        assert_eq!(state.position(LayerId::B, SequencerType::Polyphonic), 63);
        assert_eq!(state.position(LayerId::A, SequencerType::Gated), 7);
        assert_eq!(state.position(LayerId::B, SequencerType::Gated), 15);
    }

    #[test]
    fn semantic_events_preserve_inactive_raw_fields_and_remove_edit_dependencies() {
        assert_eq!(position_header_label(0, false), "  01  ");
        assert_eq!(position_header_label(1, true), "[ 02 ]");

        let note = PolyLaneStep {
            note: PolyNote::Note(60),
            velocity: PolyVelocity::Velocity(100),
        };
        assert_eq!(poly_event_raw(note), 60);
        assert!(poly_velocity_enabled(note));

        let tie = PolyLaneStep {
            note: PolyNote::Tie,
            ..note
        };
        assert_eq!(poly_event_raw(tie), 128);
        assert!(!poly_velocity_enabled(tie));

        let rest = PolyLaneStep {
            velocity: PolyVelocity::Rest,
            ..note
        };
        assert_eq!(poly_event_raw(rest), 129);
        assert!(!poly_velocity_enabled(rest));
        assert_eq!(poly_lane_for_event(rest, 129), rest);

        let tie_reset = PolyLaneStep {
            note: PolyNote::Tie,
            velocity: PolyVelocity::Reset,
        };
        assert_eq!(poly_event_raw(tie_reset), 130);
        assert!(!poly_velocity_enabled(tie_reset));
        assert_eq!(poly_lane_for_event(tie_reset, 130), tie_reset);

        assert_eq!(
            poly_lane_for_event(tie_reset, 128),
            PolyLaneStep {
                note: PolyNote::Tie,
                velocity: PolyVelocity::Velocity(127),
            }
        );
        assert_eq!(
            poly_lane_for_event(rest, 64),
            PolyLaneStep {
                note: PolyNote::Note(64),
                velocity: PolyVelocity::Velocity(127),
            }
        );
        assert_eq!(
            poly_lane_for_event(note, 129),
            PolyLaneStep {
                velocity: PolyVelocity::Rest,
                ..note
            }
        );
    }

    #[test]
    fn full_range_editors_render_without_mutating_program_data() {
        let (mut audio, bridge) = crate::engine::create_synth_engine_bridge(1);
        let mut state = SequencerViewState::default();
        let mut patch = Patch::default();
        patch.layer_a.sequence.poly.steps[0].lanes[0] = PolyLaneStep {
            note: PolyNote::Note(37),
            velocity: PolyVelocity::Rest,
        };
        patch.layer_a.sequence.poly.steps[1].lanes[0] = PolyLaneStep {
            note: PolyNote::Tie,
            velocity: PolyVelocity::Velocity(91),
        };
        let mut edit_layer = LayerId::A;
        let mut parameter_state = UiState::default();
        parameter_state.apply_from_patch(patch.layer(edit_layer));
        let layer_playback = LayerPlaybackStatus {
            mode: synth_core::LayerMode::Normal,
            edit_layer,
            rendered_mask: 0b01,
            degraded: false,
        };
        let expected = patch.layer_a.sequence;

        POLY_CELL_RENDER_COUNT.with(|count| count.set(0));
        egui::__run_test_ui(|ui| {
            ui.set_width(640.0);
            ui.set_min_height(800.0);
            show(
                ui,
                &mut state,
                &mut patch,
                &mut edit_layer,
                &mut parameter_state,
                &bridge.control,
                layer_playback,
                [SequencerPlaybackStatus::default(); 2],
            );
        });
        let rendered_cells = POLY_CELL_RENDER_COUNT.with(std::cell::Cell::get);
        assert!(
            rendered_cells <= 13 * 6 * 2,
            "640 px virtualized grid constructed {rendered_cells} cells"
        );
        assert_eq!(patch.layer_a.sequence, expected);
        let mut commands = 0;
        audio.control.drain(|_| commands += 1);
        assert_eq!(commands, 0, "drawing the grid must not emit edits");

        patch.layer_a.sequence.sequencer_type = SequencerType::Gated;
        let expected = patch.layer_a.sequence;
        parameter_state.apply_from_patch(&patch.layer_a);
        egui::__run_test_ui(|ui| {
            ui.set_width(640.0);
            ui.set_min_height(800.0);
            show(
                ui,
                &mut state,
                &mut patch,
                &mut edit_layer,
                &mut parameter_state,
                &bridge.control,
                layer_playback,
                [SequencerPlaybackStatus::default(); 2],
            );
        });
        assert_eq!(patch.layer_a.sequence, expected);
        audio.control.drain(|_| commands += 1);
        assert_eq!(commands, 0, "drawing the grid must not emit edits");
    }
}
