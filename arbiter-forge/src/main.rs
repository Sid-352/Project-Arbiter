slint::include_modules!();

use std::time::Duration;
use tokio::time::sleep;
use slint::{ComponentHandle, VecModel, Model, Color, ModelRc};
use tracing::{info};

thread_local! {
    static LOG_MODEL: std::rc::Rc<VecModel<LogEntry>> = std::rc::Rc::new(VecModel::default());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Arbiter Forge: Launching Slint Interface");

    let ui = ArbiterForge::new()?;
    let ui_handle = ui.as_weak();

    // ── Telemetry Polling (The Pulse) ─────────────────────────────────────────
    // Reads from arbiter.log and pushes parsed entries to the Slint model.
    let log_model = LOG_MODEL.with(|m| m.clone());
    ui.set_telemetry_logs(ModelRc::from(log_model));

    let ui_handle_copy = ui_handle.clone();
    tokio::spawn(async move {
        let mut last_line_count = 0;
        loop {
            sleep(Duration::from_millis(500)).await;
            
            if let Ok(content) = std::fs::read_to_string("doc/logs/arbiter.log") {
                let lines: Vec<&str> = content.lines().collect();
                if lines.len() > last_line_count {
                    let new_lines = &lines[last_line_count..];
                    last_line_count = lines.len();
                    
                    for line in new_lines {
                        // Simple Parser for: "[TIME] TAG MSG" or similar patterns
                        // Example: "[11:42:00] VIGIL FS_CREATE Z:/..."
                        let entry = parse_log_line(line);
                        
                        let ui_handle_for_loop = ui_handle_copy.clone();
                        let _ = ui_handle_for_loop.upgrade_in_event_loop(move |_ui| {
                            LOG_MODEL.with(|m| {
                                m.push(entry);
                                // Limit to 50 entries for performance
                                if m.row_count() > 50 {
                                    m.remove(0);
                                }
                            });
                        });
                    }
                }
            }
        }
    });

    // ── Slint Callbacks ───────────────────────────────────────────────────────
    
    ui.on_save_decree({
        let _ui_handle_for_callback = ui_handle.clone();
        move || {
            info!("Forge: Commit requested — serialising Ordinance graph");
            // Integration: Gather Slint properties and save to Signet
        }
    });

    // ── Run UI ────────────────────────────────────────────────────────────────
    ui.run()?;

    Ok(())
}

fn parse_log_line(line: &str) -> LogEntry {
    // Expected format: "2026-04-11T03:44:07.940124Z  INFO TAG: MESSAGE"
    let parts: Vec<&str> = line.split_whitespace().collect();
    
    if parts.len() < 3 {
        return LogEntry {
            time: "--:--:--".into(),
            tag: "INFO".into(),
            msg: line.into(),
            tag_color: Color::from_rgb_u8(161, 161, 170),
        };
    }

    // Extract time from ISO timestamp: 2026-04-11T03:44:07.940124Z -> 03:44:07
    let timestamp = parts.first().unwrap_or(&"");
    let time = if timestamp.len() > 18 && timestamp.contains('T') {
        if let Some(t_pos) = timestamp.find('T') {
            if timestamp.len() >= t_pos + 9 {
                timestamp[t_pos+1..t_pos+9].to_string()
            } else {
                "--:--:--".to_string()
            }
        } else {
            "--:--:--".to_string()
        }
    } else {
        "--:--:--".to_string()
    };

    let tag = parts[1].trim_matches(':').to_string();
    let msg = parts[2..].join(" ");

    // Semantic Color Mapping
    let tag_color = match tag.as_str() {
        "VIGIL" | "Vigil-fs" | "Vigil-keys" => Color::from_rgb_u8(99, 102, 241),    // Accent Indigo
        "ATLAS" | "Atlas" => Color::from_rgb_u8(245, 158, 11),   // Warning Orange
        "EXECUTOR" | "Executor" => Color::from_rgb_u8(16, 185, 129), // Success Emerald
        "BATON" | "Baton" => Color::from_rgb_u8(244, 63, 94),    // Critical Rose
        "ERROR" => Color::from_rgb_u8(244, 63, 94),
        "WARN" => Color::from_rgb_u8(245, 158, 11),
        _ => Color::from_rgb_u8(161, 161, 170),        // Text Secondary Gray
    };

    LogEntry {
        time: time.into(),
        tag: tag.into(),
        msg: msg.into(),
        tag_color,
    }
}
