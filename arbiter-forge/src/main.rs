slint::include_modules!();

use std::time::Duration;
use slint::{ComponentHandle, VecModel, Color, ModelRc, Model};
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

    // Initialize the UI model
    let log_model = LOG_MODEL.with(|m| m.clone());
    ui.set_telemetry_logs(ModelRc::from(log_model));

    let ui_handle_for_telemetry = ui_handle.clone();
    // ── Telemetry (The Pulse) ───────────────────────────────────────────────
    // Receives real-time logs from arbiter-app over a Named Pipe IPC channel.
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ClientOptions;
        use tokio_util::codec::{FramedRead, LinesCodec};
        use futures::StreamExt;
        use arbiter_core::ordinance::LogEntry as CoreLogEntry;

        let pipe_name = r"\\.\pipe\arbiter_telemetry";
        
        loop {
            // Attempt to connect to the pipe
            let client = match ClientOptions::new().open(pipe_name) {
                Ok(c) => c,
                Err(_) => {
                    // App not running or pipe not ready, retry in 1s
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let mut framed = FramedRead::new(client, LinesCodec::new());
            
            while let Some(Ok(line)) = framed.next().await {
                if let Ok(core_entry) = serde_json::from_str::<CoreLogEntry>(&line) {
                    let ui_handle_copy = ui_handle_for_telemetry.clone();
                    
                    // Map core LogEntry to Slint LogEntry
                    let tag_color = match core_entry.tag.as_str() {
                        "VIGIL" | "Vigil-fs" | "Vigil-keys" => Color::from_rgb_u8(99, 102, 241),
                        "ATLAS" | "Atlas" => Color::from_rgb_u8(245, 158, 11),
                        "RUNNER" | "Runner" => Color::from_rgb_u8(16, 185, 129),
                        "BATON" | "Baton" => Color::from_rgb_u8(244, 63, 94),
                        "ERROR" => Color::from_rgb_u8(244, 63, 94),
                        "WARN" => Color::from_rgb_u8(245, 158, 11),
                        _ => Color::from_rgb_u8(161, 161, 170),
                    };

                    let slint_entry = LogEntry {
                        time: chrono::Local::now().format("%H:%M:%S").to_string().into(),
                        tag: core_entry.tag.into(),
                        msg: core_entry.message.into(),
                        tag_color,
                    };

                    let _ = ui_handle_copy.upgrade_in_event_loop(move |_ui| {
                        LOG_MODEL.with(|m| {
                            m.push(slint_entry);
                            if m.row_count() > 50 {
                                m.remove(0);
                            }
                        });
                    });
                }
            }
            
            // Connection lost, retry
            tokio::time::sleep(Duration::from_millis(500)).await;
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
