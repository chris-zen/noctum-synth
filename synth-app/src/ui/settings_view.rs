use eframe::egui;
use serde::{Deserialize, Serialize};
use synth_core::FilterOversampling;

use crate::engine::SynthEngineControl;
use crate::{audio, midi};

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub midi_port: Option<String>,
    pub audio_device: Option<String>,
    #[serde(default)]
    pub audio_input: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub filter_oversampling: FilterOversampling,
    pub dark_theme: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            midi_port: None,
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
const PANEL_SPACING: f32 = 12.0;
const SAMPLE_RATE_PANEL_WIDTH: f32 = 160.0;
const RESTART_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 180, 60);

/// Snapshot of the audio settings that were actually applied when the app
/// launched. Used to detect pending changes that only take effect on restart.
#[derive(Clone)]
pub struct AudioBaseline {
    audio_device: Option<String>,
    audio_input: Option<String>,
    sample_rate: Option<u32>,
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
    control: &crate::engine::SynthEngineControl,
    midi_conn: &mut Option<midir::MidiInputConnection<()>>,
) {
    let midi_ports = midi::list_ports();
    let audio_devices = audio::list_output_devices();
    let audio_inputs = audio::list_input_devices();

    let output_restart = settings.audio_device != baseline.audio_device;
    let input_restart = settings.audio_input != baseline.audio_input;
    let rate_restart = settings.sample_rate != baseline.sample_rate;

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll")
        .show(ui, |ui| {
            let full_width = ui.available_width();

            midi_panel(ui, full_width, settings, control, midi_conn, &midi_ports);
            ui.add_space(PANEL_SPACING);

            // Audio output, audio input and sample rate share a row. The sample
            // rate panel is narrower since it only lists a handful of rates.
            let gap = ui.spacing().item_spacing.x;
            let device_width = ((full_width - SAMPLE_RATE_PANEL_WIDTH - 2.0 * gap) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                audio_panel(ui, device_width, output_restart, settings, &audio_devices);
                audio_input_panel(ui, device_width, input_restart, settings, &audio_inputs);
                sample_rate_panel(ui, SAMPLE_RATE_PANEL_WIDTH, rate_restart, settings);
            });
            ui.add_space(PANEL_SPACING);

            oversampling_panel(ui, full_width, settings, control);
            ui.add_space(PANEL_SPACING);
            general_panel(ui, full_width, settings);
        });
}

/// Renders a fixed-height settings group whose frame fits within `width`. The
/// content width (already accounting for the frame margins) is passed to
/// `add_contents` so inner widgets never overflow the right edge. When
/// `restart_required` is set, a right-aligned "Restart required" note is shown
/// in the title row.
fn settings_panel(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    restart_required: bool,
    add_contents: impl FnOnce(&mut egui::Ui, f32),
) {
    let frame = egui::Frame::group(ui.style());
    let content_width = (width - frame.total_margin().sum().x).max(0.0);
    frame.show(ui, |ui| {
        // Force a top-down layout so the panel stacks correctly even when the
        // parent places panels side by side in a horizontal row.
        ui.vertical(|ui| {
            ui.set_width(content_width);
            ui.set_height(PANEL_HEIGHT);
            ui.horizontal(|ui| {
                ui.strong(title);
                if restart_required {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(RESTART_COLOR, "Restart required");
                    });
                }
            });
            ui.separator();
            add_contents(ui, content_width);
        });
    });
}

/// Scrolling list area used by the selectable-option panels.
fn settings_list(ui: &mut egui::Ui, id: &str, width: f32, add_items: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(PANEL_HEIGHT - 56.0)
        .show(ui, |ui| {
            ui.set_width(width);
            add_items(ui);
        });
}

fn midi_panel(
    ui: &mut egui::Ui,
    width: f32,
    settings: &mut Settings,
    control: &crate::engine::SynthEngineControl,
    midi_conn: &mut Option<midir::MidiInputConnection<()>>,
    ports: &[String],
) {
    settings_panel(ui, width, "MIDI Input Device", false, |ui, width| {
        settings_list(ui, "midi_list_scroll", width, |ui| {
            if ports.is_empty() {
                ui.label("No MIDI input devices detected.");
            }
            for port in ports {
                let selected = settings.midi_port.as_deref() == Some(port.as_str());
                if ui.selectable_label(selected, port).clicked() && !selected {
                    settings.midi_port = Some(port.clone());
                    *midi_conn = midi::start_midi(Some(&port), control.clone());
                }
            }
            let none_selected = settings.midi_port.is_none();
            if ui
                .selectable_label(none_selected, "None (disconnect)")
                .clicked()
                && !none_selected
            {
                settings.midi_port = None;
                *midi_conn = None;
            }
        });

        ui.add_space(4.0);
        if midi_conn.is_some() {
            if let Some(ref port) = settings.midi_port {
                ui.colored_label(egui::Color32::GREEN, format!("Connected: {port}"));
            }
        } else if settings.midi_port.is_some() {
            ui.colored_label(
                egui::Color32::RED,
                "Failed to connect. Port may be unavailable.",
            );
        }
    });
}

fn audio_panel(
    ui: &mut egui::Ui,
    width: f32,
    restart_required: bool,
    settings: &mut Settings,
    devices: &[String],
) {
    settings_panel(
        ui,
        width,
        "Audio Output Device",
        restart_required,
        |ui, width| {
            settings_list(ui, "audio_list_scroll", width, |ui| {
                if devices.is_empty() {
                    ui.label("No audio output devices detected.");
                }
                for device in devices {
                    let selected = settings.audio_device.as_deref() == Some(device.as_str());
                    if ui.selectable_label(selected, device).clicked() && !selected {
                        settings.audio_device = Some(device.clone());
                    }
                }
            });
        },
    );
}

fn audio_input_panel(
    ui: &mut egui::Ui,
    width: f32,
    restart_required: bool,
    settings: &mut Settings,
    devices: &[String],
) {
    settings_panel(
        ui,
        width,
        "Audio Input Device",
        restart_required,
        |ui, width| {
            settings_list(ui, "audio_input_list_scroll", width, |ui| {
                if devices.is_empty() {
                    ui.label("No audio input devices detected.");
                }
                for device in devices {
                    let selected = settings.audio_input.as_deref() == Some(device.as_str());
                    if ui.selectable_label(selected, device).clicked() && !selected {
                        settings.audio_input = Some(device.clone());
                    }
                }
                let none_selected = settings.audio_input.is_none();
                if ui
                    .selectable_label(none_selected, "None (disabled)")
                    .clicked()
                    && !none_selected
                {
                    settings.audio_input = None;
                }
            });
        },
    );
}

fn sample_rate_panel(
    ui: &mut egui::Ui,
    width: f32,
    restart_required: bool,
    settings: &mut Settings,
) {
    settings_panel(ui, width, "Sample Rate", restart_required, |ui, width| {
        settings_list(ui, "sample_rate_list_scroll", width, |ui| {
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
        });
    });
}

fn oversampling_panel(
    ui: &mut egui::Ui,
    width: f32,
    settings: &mut Settings,
    control: &SynthEngineControl,
) {
    settings_panel(ui, width, "Filter Oversampling", false, |ui, width| {
        settings_list(ui, "filter_oversampling_scroll", width, |ui| {
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
        });

        ui.add_space(4.0);
        ui.label("Applies immediately and is saved for the next launch.");
    });
}

fn general_panel(ui: &mut egui::Ui, width: f32, settings: &mut Settings) {
    settings_panel(ui, width, "General", false, |ui, _width| {
        ui.checkbox(&mut settings.dark_theme, "Dark theme");
        ui.add_space(8.0);
    });
}
