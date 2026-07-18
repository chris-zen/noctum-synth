//! Desktop synthesizer application: real-time audio, MIDI, and egui UI.

mod audio;
mod config;
mod engine;
mod midi;
mod p08_factory_presets;
mod rev2_factory_presets;
mod ui;

use crate::audio::{AudioConfig, AudioManager};
use crate::engine::create_synth_engine_bridge;
use crate::ui::app::APP_TITLE;

fn main() -> eframe::Result {
    let (engine_audio, engine_bridge) = create_synth_engine_bridge(synth_core::VOICE_COUNT);

    let mut args = std::env::args().skip(1);
    let midi_port = args.next().filter(|arg| !arg.is_empty());
    let audio_device_arg = args.next().filter(|arg| !arg.is_empty());
    let audio_input_arg = args.next().filter(|arg| !arg.is_empty());

    let config = match config::Config::try_new() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to load configuration: {err}");
            std::process::exit(1);
        }
    };

    let audio_device = audio_device_arg.or_else(|| config.settings.audio_device.clone());
    let audio_input = audio_input_arg.or_else(|| config.settings.audio_input.clone());
    let sample_rate = config.settings.sample_rate;
    let filter_oversampling = config.settings.filter_oversampling;
    let filter_type = config.filter_type;

    let audio_config = AudioConfig {
        output_device: audio_device,
        input_device: audio_input,
        sample_rate,
        filter_oversampling,
        filter_type,
    };
    let audio_manager = AudioManager::start(engine_bridge.clone(), engine_audio, audio_config);

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
        .expect("app icon png is valid");

    let native_options = eframe::NativeOptions {
        viewport: ui::viewport::main_viewport_builder(config.main_viewport).with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(ui::app::App::new(
                cc,
                engine_bridge,
                audio_manager,
                midi_port,
                config,
            )))
        }),
    )
}
