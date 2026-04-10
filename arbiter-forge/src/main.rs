#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod app;
mod theme;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Setup tracing locally (optional for the terminal itself)
    tracing_subscriber::fmt::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false) // Frameless window
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0])
            .with_transparent(true),
        ..Default::default()
    };

    eframe::run_native(
        "Arbiter Terminal",
        options,
        Box::new(|_cc| Box::new(app::TerminalApp::default()) as Box<dyn eframe::App>),
    )
}
