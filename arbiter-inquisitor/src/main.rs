#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use arbiter_core::protocol::PIPE_TELEMETRY;
use arbiter_core::{
    decree::{preview_analytics, regex_pattern_matches, AnalyticalPreview},
    protocol::LogEntry,
};
use eframe::{egui, epaint};
#[cfg(target_os = "windows")]
use futures::StreamExt;
use globset::Glob;
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

struct Palette;

impl Palette {
    const SUCCESS: egui::Color32 = egui::Color32::from_rgb(16, 185, 129);
    const WARN: egui::Color32 = egui::Color32::from_rgb(245, 158, 11);
    const ERROR: egui::Color32 = egui::Color32::from_rgb(244, 63, 94);
    const SYSTEM: egui::Color32 = egui::Color32::from_rgb(99, 102, 241);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Glob,
    Regex,
}

struct InquisitorApp {
    logs: Arc<Mutex<Vec<LogEntry>>>,
    test_path: String,
    pattern: String,
    match_mode: MatchMode,
    is_match: bool,
    match_error: Option<String>,
    analytics: AnalyticalPreview,
    analytics_error: Option<String>,
}

impl InquisitorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let ctx = cc.egui_ctx.clone();

        // UI Theme
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = egui::Rounding::ZERO;
        visuals.menu_rounding = egui::Rounding::ZERO;
        visuals.panel_fill = egui::Color32::from_rgb(10, 10, 10);
        visuals.window_shadow = epaint::Shadow::NONE;
        ctx.set_visuals(visuals);
        let mut style = (*ctx.style()).clone();

        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::monospace(14.0));

        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::monospace(18.0));

        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::monospace(14.0));

        ctx.set_style(style);

        #[cfg(target_os = "windows")]
        {
            let logs_clone = logs.clone();
            let telemetry_ctx = ctx.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();

                rt.block_on(async move {
                    loop {
                        use tokio::net::windows::named_pipe::ClientOptions;

                        if let Ok(client) = ClientOptions::new().open(PIPE_TELEMETRY) {
                            let mut framed = FramedRead::new(client, LengthDelimitedCodec::new());

                            while let Some(Ok(bytes)) = framed.next().await {
                                if let Ok(mut entry) = rmp_serde::from_slice::<LogEntry>(&bytes) {
                                    if entry.time.is_empty() {
                                        entry.time =
                                            chrono::Local::now().format("%H:%M:%S").to_string();
                                    } else if let Ok(dt) =
                                        chrono::DateTime::parse_from_rfc3339(&entry.time)
                                    {
                                        entry.time = dt
                                            .with_timezone(&chrono::Local)
                                            .format("%H:%M:%S")
                                            .to_string();
                                    }
                                    let mut logs = logs_clone.lock().unwrap();
                                    logs.push(entry);

                                    if logs.len() > 2000 {
                                        logs.remove(0);
                                    }

                                    telemetry_ctx.request_repaint();
                                }
                            }
                        }

                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                });
            });
        }

        Self {
            logs,
            test_path: String::new(),
            pattern: String::new(),
            match_mode: MatchMode::Glob,
            is_match: false,
            match_error: None,
            analytics: AnalyticalPreview {
                content_sha256: None,
                content_mime: None,
            },
            analytics_error: None,
        }
    }

    fn update_match_status(&mut self) {
        self.match_error = None;

        if self.pattern.is_empty() || self.test_path.is_empty() {
            self.is_match = false;
            return;
        }

        match self.match_mode {
            MatchMode::Glob => match Glob::new(&self.pattern) {
                Ok(glob) => {
                    let matcher = glob.compile_matcher();
                    self.is_match = matcher.is_match(&self.test_path);
                }
                Err(err) => {
                    self.is_match = false;
                    self.match_error = Some(format!("Invalid glob: {}", err));
                }
            },
            MatchMode::Regex => match regex_pattern_matches(&self.pattern, &self.test_path) {
                Ok(is_match) => {
                    self.is_match = is_match;
                }
                Err(err) => {
                    self.is_match = false;
                    self.match_error = Some(format!("Invalid regex: {}", err));
                }
            },
        }
    }

    fn update_analytics(&mut self) {
        self.analytics = AnalyticalPreview {
            content_sha256: None,
            content_mime: None,
        };
        self.analytics_error = None;

        if self.test_path.trim().is_empty() {
            return;
        }

        let path = std::path::PathBuf::from(self.test_path.trim());
        if !path.is_file() {
            self.analytics_error = Some("Enter a readable file path for analytics.".to_string());
            return;
        }

        self.analytics = preview_analytics(path);

        if self.analytics.content_sha256.is_none() && self.analytics.content_mime.is_none() {
            self.analytics_error = Some(
                "Analytical extraction is unavailable for this build or file type.".to_string(),
            );
        }
    }

    fn refresh_sandbox(&mut self) {
        self.update_match_status();
        self.update_analytics();
    }

    fn match_mode_label(&self) -> &'static str {
        match self.match_mode {
            MatchMode::Glob => "Glob Pattern",
            MatchMode::Regex => "Regex Pattern",
        }
    }

    fn match_mode_hint(&self) -> &'static str {
        match self.match_mode {
            MatchMode::Glob => "Example: src/**/*.rs",
            MatchMode::Regex => "Example: ^src/.+\\.rs$",
        }
    }

    fn analytics_value(value: &Option<String>) -> &str {
        match value {
            Some(value) => value.as_str(),
            None => "unavailable",
        }
    }
}

impl eframe::App for InquisitorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::right("sandbox_panel")
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading(
                    egui::RichText::new("INQUISITOR SANDBOX")
                        .strong()
                        .color(Palette::SYSTEM),
                );

                ui.separator();
                ui.add_space(6.0);

                ui.label("Test Path");
                if ui.text_edit_singleline(&mut self.test_path).changed() {
                    self.refresh_sandbox();
                }

                ui.add_space(4.0);
                let previous_mode = self.match_mode;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.match_mode, MatchMode::Glob, "Glob");
                    ui.selectable_value(&mut self.match_mode, MatchMode::Regex, "Regex");
                });
                if previous_mode != self.match_mode {
                    self.update_match_status();
                }

                ui.label(self.match_mode_label());
                ui.small(self.match_mode_hint());

                if ui.text_edit_singleline(&mut self.pattern).changed() {
                    self.update_match_status();
                }

                ui.add_space(10.0);

                let status_text = if self.is_match { "MATCH" } else { "NO MATCH" };

                let status_color = if self.match_error.is_some() {
                    Palette::WARN
                } else if self.is_match {
                    Palette::SUCCESS
                } else {
                    Palette::ERROR
                };

                ui.label(
                    egui::RichText::new(status_text)
                        .strong()
                        .color(status_color),
                );

                if let Some(err) = &self.match_error {
                    ui.label(egui::RichText::new(err).color(Palette::WARN).small());
                }

                ui.separator();
                ui.label(egui::RichText::new("ANALYTICS").strong());
                ui.monospace(format!(
                    "content_sha256: {}",
                    Self::analytics_value(&self.analytics.content_sha256)
                ));
                ui.monospace(format!(
                    "content_mime: {}",
                    Self::analytics_value(&self.analytics.content_mime)
                ));

                if let Some(err) = &self.analytics_error {
                    ui.label(egui::RichText::new(err).color(Palette::WARN).small());
                }
            });

        // ===================== LOGS (CENTER PANEL) =====================
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("ARBITER INQUISITOR // VIVISECTION TABLE")
                        .strong()
                        .color(Palette::SYSTEM),
                );

                if ui.button("CLEAR").clicked() {
                    self.logs.lock().unwrap().clear();
                }
            });

            ui.separator();

            let logs = self.logs.lock().unwrap();

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    use egui_extras::{Column, TableBuilder};

                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .column(Column::initial(100.0))
                        .column(Column::initial(80.0))
                        .column(Column::remainder())
                        .header(20.0, |mut header| {
                            header.col(|ui| {
                                ui.strong("TIME");
                            });
                            header.col(|ui| {
                                ui.strong("TAG");
                            });
                            header.col(|ui| {
                                ui.strong("MESSAGE");
                            });
                        })
                        .body(|body| {
                            body.rows(18.0, logs.len(), |mut row| {
                                let log = &logs[row.index()];

                                row.col(|ui| {
                                    ui.label(egui::RichText::new(&log.time).monospace().small());
                                });

                                row.col(|ui| {
                                    let color = match log.tag.as_str() {
                                        "ATLAS" => Palette::WARN,
                                        "VIGIL" | "Vigil-fs" => Palette::SYSTEM,
                                        "RUNNER" | "Runner" => Palette::SUCCESS,
                                        "PRESN" => Palette::ERROR,
                                        _ => egui::Color32::LIGHT_GRAY,
                                    };
                                    ui.label(
                                        egui::RichText::new(&log.tag).color(color).strong().small(),
                                    );
                                });

                                row.col(|ui| {
                                    let text_color = if log.is_error {
                                        Palette::ERROR
                                    } else {
                                        egui::Color32::LIGHT_GRAY
                                    };

                                    ui.label(
                                        egui::RichText::new(&log.message).color(text_color).small(),
                                    );
                                });
                            });
                        });
                });
        });
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 600.0])
            .with_title("Arbiter Inquisitor"),
        ..Default::default()
    };

    eframe::run_native(
        "Arbiter Inquisitor",
        native_options,
        Box::new(|cc| Ok(Box::new(InquisitorApp::new(cc)))),
    )?;

    Ok(())
}
