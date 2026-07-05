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

pub fn show(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    control: &crate::engine::SynthEngineControl,
    midi_conn: &mut Option<midir::MidiInputConnection<()>>,
) {
    let midi_ports = midi::list_ports();
    let audio_devices = audio::list_output_devices();
    let audio_inputs = audio::list_input_devices();

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll")
        .show(ui, |ui| {
            let width = ui.available_width();

            midi_panel(ui, width, settings, control, midi_conn, &midi_ports);
            ui.add_space(PANEL_SPACING);
            audio_panel(ui, width, settings, &audio_devices);
            ui.add_space(PANEL_SPACING);
            audio_input_panel(ui, width, settings, &audio_inputs);
            ui.add_space(PANEL_SPACING);
            sample_rate_panel(ui, width, settings);
            ui.add_space(PANEL_SPACING);
            oversampling_panel(ui, width, settings, control);
            ui.add_space(PANEL_SPACING);
            general_panel(ui, width, settings);
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
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("MIDI Input Device");
        ui.separator();

        let list_h = PANEL_HEIGHT - 56.0;
        egui::ScrollArea::vertical()
            .id_salt("midi_list_scroll")
            .max_height(list_h)
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
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

fn audio_panel(ui: &mut egui::Ui, width: f32, settings: &mut Settings, devices: &[String]) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("Audio Output Device");
        ui.separator();

        let list_h = PANEL_HEIGHT - 56.0;
        egui::ScrollArea::vertical()
            .id_salt("audio_list_scroll")
            .max_height(list_h)
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
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

        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(200, 180, 60),
            "Audio device changes require restart to take effect.",
        );
    });
}

fn audio_input_panel(ui: &mut egui::Ui, width: f32, settings: &mut Settings, devices: &[String]) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("Audio Input Device");
        ui.separator();

        let list_h = PANEL_HEIGHT - 56.0;
        egui::ScrollArea::vertical()
            .id_salt("audio_input_list_scroll")
            .max_height(list_h)
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
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

        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(200, 180, 60),
            "Input changes require restart. Input must match the output sample rate.",
        );
    });
}

fn sample_rate_panel(ui: &mut egui::Ui, width: f32, settings: &mut Settings) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("Sample Rate");
        ui.separator();

        let list_h = PANEL_HEIGHT - 56.0;
        egui::ScrollArea::vertical()
            .id_salt("sample_rate_list_scroll")
            .max_height(list_h)
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
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

        ui.add_space(4.0);
        ui.colored_label(
            egui::Color32::from_rgb(200, 180, 60),
            "Sample rate changes require restart to take effect.",
        );
    });
}

fn oversampling_panel(
    ui: &mut egui::Ui,
    width: f32,
    settings: &mut Settings,
    control: &SynthEngineControl,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("Filter Oversampling");
        ui.separator();

        let list_h = PANEL_HEIGHT - 56.0;
        egui::ScrollArea::vertical()
            .id_salt("filter_oversampling_scroll")
            .max_height(list_h)
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
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
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_height(PANEL_HEIGHT);
        ui.strong("General");
        ui.separator();

        ui.checkbox(&mut settings.dark_theme, "Dark theme");
        ui.add_space(8.0);
    });
}
