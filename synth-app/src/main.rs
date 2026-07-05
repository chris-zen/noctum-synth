//! Desktop synthesizer application: real-time audio, MIDI, and egui UI.

mod audio;
mod config;
mod engine;
mod midi;
mod ui;

use crate::engine::create_synth_engine_bridge;
use crate::ui::app::APP_TITLE;

fn main() -> eframe::Result {
    let (engine_audio, engine_bridge) = create_synth_engine_bridge(synth_core::VOICE_COUNT);

    let mut args = std::env::args().skip(1);
    let midi_port = args.next().filter(|arg| !arg.is_empty());
    let audio_device = args.next().filter(|arg| !arg.is_empty());

    audio::start_audio(engine_audio, audio_device);

    let config = match config::Config::try_new() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to load configuration: {err}");
            std::process::exit(1);
        }
    };

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
                midi_port,
                config,
            )))
        }),
    )
}
