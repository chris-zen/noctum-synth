use eframe::egui;
use serde::{Deserialize, Serialize};
use synth_core::LayerPatch;
use synth_core::dsp::{FilterOversampling, FilterType};
use synth_core::midi::clock::{MidiClockMode, MidiTransportState};

use crate::audio::{AppliedAudioConfig, AudioConfig, AudioManager};
use crate::engine::SynthEngineControl;
use crate::{audio, midi};

fn default_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MidiInputEntry {
    pub port: String,
    #[serde(default = "default_true")]
    pub control: bool,
    #[serde(default = "default_true")]
    pub patches: bool,
    #[serde(default)]
    pub forward: bool,
}

impl MidiInputEntry {
    pub fn new(port: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            control: true,
            patches: true,
            forward: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub midi_inputs: Vec<MidiInputEntry>,
    #[serde(default, skip_serializing)]
    midi_port: Option<String>,
    #[serde(default)]
    pub midi_output_port: Option<String>,
    #[serde(default)]
    pub midi_clock_mode: MidiClockMode,
    #[serde(default)]
    pub midi_clock_source: Option<String>,
    #[serde(default)]
    pub midi_output_clock_mode: MidiClockMode,
    pub audio_device: Option<String>,
    #[serde(default)]
    pub audio_input: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub filter_oversampling: FilterOversampling,
    pub dark_theme: bool,
}

impl Settings {
    pub fn migrate_legacy_midi_port(&mut self) {
        if self.midi_inputs.is_empty() {
            if let Some(port) = self.midi_port.take() {
                self.midi_inputs.push(MidiInputEntry::new(port));
            }
        } else {
            self.midi_port = None;
        }
        if self
            .midi_clock_source
            .as_ref()
            .is_some_and(|source| !self.midi_inputs.iter().any(|entry| entry.port == *source))
        {
            self.midi_clock_source = None;
        }
        self.midi_clock_mode = match self.midi_clock_mode {
            MidiClockMode::Off | MidiClockMode::Slave | MidiClockMode::Master => {
                self.midi_clock_mode
            }
            MidiClockMode::SlaveThru | MidiClockMode::SlaveNoStartStop => MidiClockMode::Slave,
        };
        if !self.midi_output_clock_mode.is_supported() {
            self.midi_output_clock_mode = MidiClockMode::Off;
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            midi_inputs: Vec::new(),
            midi_port: None,
            midi_output_port: None,
            midi_clock_mode: MidiClockMode::Off,
            midi_clock_source: None,
            midi_output_clock_mode: MidiClockMode::Off,
            audio_device: None,
            audio_input: None,
            sample_rate: None,
            filter_oversampling: FilterOversampling::Auto,
            dark_theme: true,
        }
    }
}

const SAMPLE_RATE_OPTIONS: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

const PANEL_HEIGHT: f32 = 180.0;
const AUDIO_PANEL_HEIGHT: f32 = PANEL_HEIGHT + 36.0;
const MIDI_PANEL_HEIGHT: f32 = PANEL_HEIGHT + 84.0;
const PANEL_SPACING: f32 = 12.0;
const COLUMN_LIST_HEIGHT: f32 = PANEL_HEIGHT - 56.0;
const SAMPLE_RATE_COLUMN_WIDTH: f32 = 160.0;
const APPLY_BUTTON_FILL: egui::Color32 = egui::Color32::from_rgb(40, 100, 180);
const UNAVAILABLE_PORT_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 80, 80);

/// Snapshot of the audio settings currently in use by the live audio session.
#[derive(Clone)]
pub struct AudioBaseline {
    pub audio_device: Option<String>,
    pub audio_input: Option<String>,
    pub sample_rate: Option<u32>,
}

impl AudioBaseline {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            audio_device: settings.audio_device.clone(),
            audio_input: settings.audio_input.clone(),
            sample_rate: settings.sample_rate,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    baseline: &AudioBaseline,
    applied: &AppliedAudioConfig,
    audio_manager: &AudioManager,
    filter_type: FilterType,
    control: &crate::engine::SynthEngineControl,
    midi_inputs: &mut midi::MidiInputManager,
    current_patch: &LayerPatch,
    muted: bool,
) {
    let previous_clock_mode = settings.midi_clock_mode;
    let previous_clock_source = settings.midi_clock_source.clone();
    let previous_output_clock_mode = settings.midi_output_clock_mode;
    let midi_input_ports = midi::list_input_ports();
    let midi_output_ports = midi::list_output_ports();
    midi_inputs.refresh_available_ports();
    control.midi_output().refresh_available_ports();
    let audio_devices = audio::list_output_devices();
    let audio_inputs = audio::list_input_devices();

    let output_pending = settings.audio_device != baseline.audio_device;
    let input_pending = settings.audio_input != baseline.audio_input;
    let rate_pending = settings.sample_rate != baseline.sample_rate;
    let audio_pending = output_pending || input_pending || rate_pending;

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll")
        .show(ui, |ui| {
            let full_width = ui.available_width();

            midi_settings_panel(
                ui,
                full_width,
                settings,
                midi_inputs,
                control,
                &midi_input_ports,
                &midi_output_ports,
                current_patch,
                muted,
            );
            ui.add_space(PANEL_SPACING);

            audio_settings_panel(
                ui,
                full_width,
                audio_pending,
                applied,
                settings,
                audio_manager,
                filter_type,
                &audio_devices,
                &audio_inputs,
            );
            ui.add_space(PANEL_SPACING);

            oversampling_panel(ui, full_width, settings, control);
            ui.add_space(PANEL_SPACING);

            general_panel(ui, full_width, settings);
        });

    midi_inputs.sync(
        &settings.midi_inputs,
        settings.midi_clock_source.as_deref(),
        settings.midi_clock_mode,
    );
    if settings.midi_clock_mode != previous_clock_mode {
        control.set_midi_clock_mode(settings.midi_clock_mode);
    } else if settings.midi_clock_source != previous_clock_source
        && settings.midi_clock_mode.receives_clock()
    {
        control.set_midi_clock_mode(MidiClockMode::Off);
        control.set_midi_clock_mode(settings.midi_clock_mode);
    }
    if settings.midi_output_clock_mode != previous_output_clock_mode {
        control.set_midi_output_clock_mode(settings.midi_output_clock_mode);
    }
}

fn midi_settings_panel(
    ui: &mut egui::Ui,
    width: f32,
    settings: &mut Settings,
    midi_inputs: &mut midi::MidiInputManager,
    control: &SynthEngineControl,
    midi_input_ports: &[String],
    midi_output_ports: &[String],
    current_patch: &LayerPatch,
    muted: bool,
) {
    let frame = egui::Frame::group(ui.style());
    let content_width = (width - frame.total_margin().sum().x).max(0.0);
    ui.set_width(width);
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(content_width);
            ui.set_max_width(content_width);
            ui.set_height(MIDI_PANEL_HEIGHT);
            ui.strong("MIDI");
            ui.separator();

            let gap = ui.spacing().item_spacing.x;
            let col_width = ((content_width - gap) / 2.0).max(0.0);

            ui.horizontal_top(|ui| {
                let configured_inputs: Vec<String> = settings
                    .midi_inputs
                    .iter()
                    .map(|entry| entry.port.clone())
                    .collect();
                let merged_inputs = midi::merged_port_list(midi_input_ports, &configured_inputs);

                settings_column(
                    ui,
                    col_width,
                    "Input Devices",
                    "midi_input_list_scroll",
                    COLUMN_LIST_HEIGHT,
                    |ui| {
                        if merged_inputs.is_empty() {
                            ui.label("No MIDI input devices detected.");
                        }
                        for port in &merged_inputs {
                            let selected =
                                settings.midi_inputs.iter().any(|entry| entry.port == *port);
                            let clock_selected =
                                settings.midi_clock_source.as_deref() == Some(port.as_str());
                            let unavailable = selected
                                && midi_inputs.connection_state(port)
                                    != midi::PortConnectionState::Connected;
                            let mut clock_change = None;

                            ui.horizontal(|ui| {
                                let label = if unavailable {
                                    egui::RichText::new(port).color(UNAVAILABLE_PORT_COLOR)
                                } else {
                                    egui::RichText::new(port)
                                };
                                if ui.selectable_label(selected, label).clicked() {
                                    if selected {
                                        settings.midi_inputs.retain(|entry| entry.port != *port);
                                        if settings.midi_clock_source.as_deref()
                                            == Some(port.as_str())
                                        {
                                            settings.midi_clock_source = None;
                                        }
                                    } else {
                                        settings
                                            .midi_inputs
                                            .push(MidiInputEntry::new(port.clone()));
                                    }
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if let Some(entry) = settings
                                            .midi_inputs
                                            .iter_mut()
                                            .find(|entry| entry.port == *port)
                                        {
                                            let enabled = !unavailable;
                                            ui.add_enabled_ui(enabled, |ui| {
                                                ui.toggle_value(&mut entry.forward, "Forward");
                                                ui.toggle_value(&mut entry.patches, "Patches");
                                                ui.toggle_value(&mut entry.control, "Control");
                                                let mut clock = clock_selected;
                                                if ui
                                                    .add_enabled(
                                                        settings.midi_clock_mode.receives_clock(),
                                                        egui::Button::selectable(clock, "Clock"),
                                                    )
                                                    .clicked()
                                                {
                                                    clock = !clock;
                                                    clock_change = Some(clock);
                                                }
                                            });
                                        }
                                    },
                                );
                            });

                            match clock_change {
                                Some(true) => settings.midi_clock_source = Some(port.clone()),
                                Some(false) if clock_selected => settings.midi_clock_source = None,
                                _ => {}
                            }
                        }
                    },
                );

                ui.separator();

                let configured_outputs = settings
                    .midi_output_port
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                let merged_outputs = midi::merged_port_list(midi_output_ports, &configured_outputs);

                settings_column(
                    ui,
                    col_width,
                    "Output Device",
                    "midi_output_list_scroll",
                    COLUMN_LIST_HEIGHT,
                    |ui| {
                        if merged_outputs.is_empty() {
                            ui.label("No MIDI output devices detected.");
                        }
                        for port in &merged_outputs {
                            let selected =
                                settings.midi_output_port.as_deref() == Some(port.as_str());
                            let unavailable = selected
                                && control.midi_output().connection_state()
                                    != midi::PortConnectionState::Connected;

                            let label = if unavailable {
                                egui::RichText::new(port).color(UNAVAILABLE_PORT_COLOR)
                            } else {
                                egui::RichText::new(port)
                            };
                            if ui.selectable_label(selected, label).clicked()
                                && (!selected || !control.midi_output_connected())
                            {
                                settings.midi_output_port = Some(port.clone());
                                if control.set_midi_output_port(Some(port)) {
                                    control.load_patch_respecting_mute(current_patch, muted);
                                }
                            }
                        }
                        let none_selected = settings.midi_output_port.is_none();
                        if ui
                            .selectable_label(none_selected, "None (disconnect)")
                            .clicked()
                            && !none_selected
                        {
                            settings.midi_output_port = None;
                            control.set_midi_output_port(None);
                        }
                    },
                );
            });

            ui.separator();

            ui.add_space(4.0);

            egui::Grid::new("midi_clock_settings")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("MIDI Clock");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for mode in [
                            MidiClockMode::Off,
                            MidiClockMode::Master,
                            MidiClockMode::Slave,
                        ] {
                            if ui
                                .add(egui::Button::selectable(
                                    settings.midi_clock_mode == mode,
                                    mode.name(),
                                ))
                                .clicked()
                            {
                                settings.midi_clock_mode = mode;
                            }
                        }
                        ui.separator();
                        let source_enabled = settings.midi_clock_mode.receives_clock();
                        let status_text = if settings.midi_clock_mode == MidiClockMode::Off {
                            "Using patch BPM".to_owned()
                        } else if settings.midi_clock_mode == MidiClockMode::Master {
                            "Sending MIDI clock".to_owned()
                        } else if source_enabled && settings.midi_clock_source.is_none() {
                            "Select a clock source".to_owned()
                        } else if let Some(status) = control.clock_status_for_ui() {
                            if status.live && settings.midi_clock_mode.receives_start_stop() {
                                let transport = match status.transport {
                                    MidiTransportState::Running => "Running",
                                    MidiTransportState::Stopped => "Stopped",
                                };
                                format!("Live · {:.1} BPM · {transport}", status.effective_bpm)
                            } else if status.live {
                                format!("Live · {:.1} BPM", status.effective_bpm)
                            } else if status.learned_bpm.is_some() {
                                format!("Lost · {:.1} BPM", status.effective_bpm)
                            } else {
                                "Waiting for clock".to_owned()
                            }
                        } else {
                            "Waiting for clock".to_owned()
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(status_text).weak()).truncate(),
                        );
                    });
                    ui.end_row();

                    ui.strong("MIDI Output Clock");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for mode in [
                            MidiClockMode::Off,
                            MidiClockMode::Master,
                            MidiClockMode::Slave,
                            MidiClockMode::SlaveThru,
                            MidiClockMode::SlaveNoStartStop,
                        ] {
                            let supported = mode.is_supported();
                            ui.add_enabled_ui(supported, |ui| {
                                let response = ui.add(egui::Button::selectable(
                                    settings.midi_output_clock_mode == mode,
                                    mode.name(),
                                ));
                                if response.clicked() {
                                    settings.midi_output_clock_mode = mode;
                                }
                                if !supported {
                                    response.on_disabled_hover_text(
                                        "Future work; not supported by Daisy",
                                    );
                                }
                            });
                        }
                    });
                    ui.end_row();
                });
        });
    });
}

/// Renders a fixed-height settings group whose frame fits within `width`. The
/// content width (already accounting for the frame margins) is passed to
/// `add_contents` so inner widgets never overflow the right edge.
fn settings_panel(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui, f32),
) {
    let frame = egui::Frame::group(ui.style());
    let content_width = (width - frame.total_margin().sum().x).max(0.0);
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(content_width);
            ui.set_height(PANEL_HEIGHT);
            ui.strong(title);
            ui.separator();
            add_contents(ui, content_width);
        });
    });
}

/// Scrolling list area used by the selectable-option panels.
fn settings_list(
    ui: &mut egui::Ui,
    id: &str,
    width: f32,
    max_height: f32,
    add_items: impl FnOnce(&mut egui::Ui),
) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_width(width);
            add_items(ui);
        });
}

fn settings_column(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    list_id: &str,
    list_height: f32,
    add_items: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.set_max_width(width);
        ui.strong(title);
        ui.separator();
        settings_list(ui, list_id, width, list_height, add_items);
    });
}

fn truncated_selectable(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.add(
        egui::Button::selectable(selected, label)
            .frame_when_inactive(true)
            .truncate(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_normalizes_modes_not_exposed_by_each_clock_selector() {
        let mut settings = Settings {
            midi_clock_mode: MidiClockMode::SlaveNoStartStop,
            midi_output_clock_mode: MidiClockMode::Master,
            ..Settings::default()
        };

        settings.migrate_legacy_midi_port();

        assert_eq!(settings.midi_clock_mode, MidiClockMode::Slave);
        assert_eq!(settings.midi_output_clock_mode, MidiClockMode::Off);
    }
}

fn audio_column_widths(content_width: f32, spacing: f32) -> (f32, f32, f32) {
    const SEPARATOR_WIDTH: f32 = 8.0;
    let gaps = spacing * 4.0;
    let usable = (content_width - gaps - 2.0 * SEPARATOR_WIDTH).max(0.0);
    if usable <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let rate = (usable * 0.26).clamp(80.0, SAMPLE_RATE_COLUMN_WIDTH);
    let device = ((usable - rate) / 2.0).max(0.0);
    (device, device, rate)
}

fn audio_settings_panel(
    ui: &mut egui::Ui,
    width: f32,
    pending: bool,
    applied: &AppliedAudioConfig,
    settings: &mut Settings,
    audio_manager: &AudioManager,
    filter_type: FilterType,
    output_devices: &[String],
    input_devices: &[String],
) {
    let frame = egui::Frame::group(ui.style());
    let content_width = (width - frame.total_margin().sum().x).max(0.0);
    ui.set_width(width);
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(content_width);
            ui.set_max_width(content_width);
            ui.set_height(AUDIO_PANEL_HEIGHT);
            ui.strong("Audio");
            ui.separator();

            let spacing = ui.spacing().item_spacing.x;
            let (device_width, _, rate_width) = audio_column_widths(content_width, spacing);

            ui.horizontal_top(|ui| {
                ui.set_max_width(content_width);
                let configured_outputs = settings.audio_device.iter().cloned().collect::<Vec<_>>();
                let merged_outputs = midi::merged_port_list(output_devices, &configured_outputs);

                settings_column(
                    ui,
                    device_width,
                    "Output Device",
                    "audio_list_scroll",
                    COLUMN_LIST_HEIGHT,
                    |ui| {
                        if merged_outputs.is_empty() {
                            ui.label("No audio output devices detected.");
                        }
                        for device in &merged_outputs {
                            let selected =
                                settings.audio_device.as_deref() == Some(device.as_str());
                            let unavailable =
                                selected && !output_devices.iter().any(|listed| listed == device);
                            let label = if unavailable {
                                egui::RichText::new(device).color(UNAVAILABLE_PORT_COLOR)
                            } else {
                                egui::RichText::new(device)
                            };
                            if truncated_selectable(ui, selected, label).clicked() && !selected {
                                settings.audio_device = Some(device.clone());
                            }
                        }
                    },
                );
                ui.separator();
                let configured_inputs = settings.audio_input.iter().cloned().collect::<Vec<_>>();
                let merged_inputs = midi::merged_port_list(input_devices, &configured_inputs);

                settings_column(
                    ui,
                    device_width,
                    "Input Device",
                    "audio_input_list_scroll",
                    COLUMN_LIST_HEIGHT,
                    |ui| {
                        if merged_inputs.is_empty() {
                            ui.label("No audio input devices detected.");
                        }
                        for device in &merged_inputs {
                            let selected = settings.audio_input.as_deref() == Some(device.as_str());
                            let unavailable =
                                selected && !input_devices.iter().any(|listed| listed == device);
                            let label = if unavailable {
                                egui::RichText::new(device).color(UNAVAILABLE_PORT_COLOR)
                            } else {
                                egui::RichText::new(device)
                            };
                            if truncated_selectable(ui, selected, label).clicked() && !selected {
                                settings.audio_input = Some(device.clone());
                            }
                        }
                        let none_selected = settings.audio_input.is_none();
                        if truncated_selectable(ui, none_selected, "None (disabled)").clicked()
                            && !none_selected
                        {
                            settings.audio_input = None;
                        }
                    },
                );
                ui.separator();
                settings_column(
                    ui,
                    rate_width,
                    "Sample Rate",
                    "sample_rate_list_scroll",
                    COLUMN_LIST_HEIGHT,
                    |ui| {
                        let default_selected = settings.sample_rate.is_none();
                        if ui
                            .selectable_label(default_selected, "Device default")
                            .clicked()
                            && !default_selected
                        {
                            settings.sample_rate = None;
                        }
                        for rate in SAMPLE_RATE_OPTIONS {
                            let selected = settings.sample_rate == Some(rate);
                            if ui
                                .selectable_label(selected, format!("{rate} Hz"))
                                .clicked()
                                && !selected
                            {
                                settings.sample_rate = Some(rate);
                            }
                        }
                    },
                );
            });

            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let reconnect_enabled = !applied.applying;
                if ui
                    .add_enabled(reconnect_enabled, egui::Button::new("Reconnect"))
                    .clicked()
                {
                    audio_manager.apply(AudioConfig {
                        output_device: settings.audio_device.clone(),
                        input_device: settings.audio_input.clone(),
                        sample_rate: settings.sample_rate,
                        filter_oversampling: settings.filter_oversampling,
                        filter_type,
                    });
                }

                let enabled = pending && !applied.applying;
                let label = if applied.applying {
                    "Applying..."
                } else {
                    "Apply changes"
                };
                let mut button = egui::Button::new(label);
                if enabled {
                    button = button.fill(APPLY_BUTTON_FILL);
                }
                if ui.add_enabled(enabled, button).clicked() {
                    audio_manager.apply(AudioConfig {
                        output_device: settings.audio_device.clone(),
                        input_device: settings.audio_input.clone(),
                        sample_rate: settings.sample_rate,
                        filter_oversampling: settings.filter_oversampling,
                        filter_type,
                    });
                }
            });
        });
    });
}

fn oversampling_panel(
    ui: &mut egui::Ui,
    width: f32,
    settings: &mut Settings,
    control: &SynthEngineControl,
) {
    settings_panel(ui, width, "Filter Oversampling", |ui, width| {
        settings_list(
            ui,
            "filter_oversampling_scroll",
            width,
            COLUMN_LIST_HEIGHT,
            |ui| {
                for (mode, label) in [
                    (FilterOversampling::Auto, "Auto"),
                    (FilterOversampling::Off, "Off"),
                    (FilterOversampling::X2, "2x"),
                    (FilterOversampling::X4, "4x"),
                ] {
                    if ui
                        .selectable_value(&mut settings.filter_oversampling, mode, label)
                        .changed()
                    {
                        control.set_filter_oversampling(settings.filter_oversampling);
                    }
                }
            },
        );
    });
}

fn general_panel(ui: &mut egui::Ui, width: f32, settings: &mut Settings) {
    settings_panel(ui, width, "General", |ui, _width| {
        ui.checkbox(&mut settings.dark_theme, "Dark theme");
        ui.add_space(8.0);
    });
}
