mod app;
mod audio;
mod config;
mod db;
mod model;
mod scan;
mod subsonic;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 560.0])
            .with_app_id("aural")
            .with_title("Aural"),
        ..Default::default()
    };

    eframe::run_native(
        "Aural",
        options,
        Box::new(|cc| Ok(Box::new(app::AuralApp::new(cc)))),
    )
}
