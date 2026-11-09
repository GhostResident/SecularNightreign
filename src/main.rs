#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod download;
mod events;
mod game_update;
mod install;
mod package;
mod platform;
mod seamless;
mod state;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([920.0, 720.0])
            .with_min_inner_size([760.0, 620.0])
            .with_title(config::APP_TITLE),
        ..Default::default()
    };
    eframe::run_native(
        config::APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(app::LauncherApp::new(cc)))),
    )
}
