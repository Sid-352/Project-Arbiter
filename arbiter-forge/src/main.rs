slint::include_modules!();

use std::rc::Rc;
use std::time::Duration;
use slint::{ComponentHandle, Model, ModelRc, VecModel, Color};
use tracing::info;

thread_local! {
    static LOG_MODEL:    Rc<VecModel<LogEntry>>    = Rc::new(VecModel::default());
    static DECREE_MODEL: Rc<VecModel<DecreeEntry>> = Rc::new(VecModel::default());
    static STEP_MODEL:   Rc<VecModel<DecreeStep>>  = Rc::new(VecModel::default());
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tiny helper to generate sequential IDs without pulling in uuid.
// ─────────────────────────────────────────────────────────────────────────────
fn next_id() -> slint::SharedString {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(1);
    format!("id-{}", CTR.fetch_add(1, Ordering::Relaxed)).into()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Smoke-test seed data — replaced by ledger.json in Session 2.
// ─────────────────────────────────────────────────────────────────────────────
fn seed_models() {
    DECREE_MODEL.with(|m| {
        m.push(DecreeEntry {
            id:     "decree-1".into(),
            label:  "Archive Automation".into(),
            status: 1,
        });
        m.push(DecreeEntry {
            id:     "decree-2".into(),
            label:  "Registry Scrubber".into(),
            status: 2,
        });
    });

    STEP_MODEL.with(|m| {
        m.push(DecreeStep {
            id:             "step-1".into(),
            title:          "Successive Size Check".into(),
            subtext:        "Awaiting stability pulse".into(),
            step_type:      3,   // Steady
            is_active:      true,
            baton_required: false,
            arg_a:          "".into(),
            arg_b:          "".into(),
        });
        m.push(DecreeStep {
            id:             "step-2".into(),
            title:          "Atomic Relocation".into(),
            subtext:        "Move verified artifact to vault".into(),
            step_type:      0,   // Inscribe
            is_active:      false,
            baton_required: false,
            arg_a:          "${env.file_path}".into(),
            arg_b:          "C:/Archive/".into(),
        });
        m.push(DecreeStep {
            id:             "step-3".into(),
            title:          "Shell Dispatch".into(),
            subtext:        "Firing post-process signal".into(),
            step_type:      1,   // Shell
            is_active:      false,
            baton_required: true,
            arg_a:          "7z.exe".into(),
            arg_b:          "a archive.zip ${env.file_path}".into(),
        });
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Arbiter Forge: Launching Slint Interface");

    seed_models();

    let ui = ArbiterForge::new()?;
    let ui_handle = ui.as_weak();

    // ── Push models into the UI ───────────────────────────────────────────────
    let log_model     = LOG_MODEL.with(|m| m.clone());
    let decree_model  = DECREE_MODEL.with(|m| m.clone());
    let step_model    = STEP_MODEL.with(|m| m.clone());

    ui.set_telemetry_logs(ModelRc::from(log_model.clone()));
    ui.set_decree_list(ModelRc::from(decree_model.clone()));
    ui.set_decree_steps(ModelRc::from(step_model.clone()));

    // Seed a startup log
    log_model.push(LogEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string().into(),
        tag: "FORGE".into(),
        tag_color: Color::from_rgb_u8(99, 102, 241),
        msg: "Terminal interface active. Waiting for telemetry pipe...".into(),
        ordinance_id: "".into(),
    });

    // Select the first decree by default
    ui.set_active_decree_id("decree-1".into());
    ui.set_active_decree_label("Archive Automation".into());

    // ── Telemetry: Named Pipe from arbiter-app ────────────────────────────────
    let ui_handle_telemetry = ui_handle.clone();
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ClientOptions;
        use tokio_util::codec::{FramedRead, LinesCodec};
        use futures::StreamExt;
        use arbiter_core::ordinance::LogEntry as CoreLogEntry;
        use tokio::time::timeout;

        let pipe_name = r"\\.\pipe\arbiter_telemetry";
        let watchdog_duration = Duration::from_secs(2);

        loop {
            let client = match ClientOptions::new().open(pipe_name) {
                Ok(c)  => c,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let mut framed = FramedRead::new(client, LinesCodec::new());

            // The Watchdog: If we get zero data for 30s, the engine is dead.
            loop {
                match timeout(watchdog_duration, framed.next()).await {
                    Ok(Some(Ok(line))) => {
                        match serde_json::from_str::<CoreLogEntry>(&line) {
                            Ok(core_entry) => {
                                let ui_copy = ui_handle_telemetry.clone();

                                let tag_color = match core_entry.tag.as_str() {
                                    "VIGIL" | "Vigil-fs" | "Vigil-keys" => Color::from_rgb_u8(99, 102, 241),
                                    "ATLAS" | "Atlas"                    => Color::from_rgb_u8(245, 158, 11),
                                    "RUNNER" | "Runner"                  => Color::from_rgb_u8(16, 185, 129),
                                    "BATON" | "Baton"                    => Color::from_rgb_u8(244, 63, 94),
                                    "ERROR"                              => Color::from_rgb_u8(244, 63, 94),
                                    "WARN"                               => Color::from_rgb_u8(245, 158, 11),
                                    _                                    => Color::from_rgb_u8(161, 161, 170),
                                };

                                let entry = LogEntry {
                                    time:      chrono::Local::now().format("%H:%M:%S").to_string().into(),
                                    tag:       core_entry.tag.into(),
                                    msg:       core_entry.message.into(),
                                    tag_color,
                                    ordinance_id: core_entry.ordinance_id.unwrap_or_default().into(),
                                };

                                let _ = ui_copy.upgrade_in_event_loop(move |_ui| {
                                    LOG_MODEL.with(|m| {
                                        m.push(entry);
                                        if m.row_count() > 50 {
                                            m.remove(0);
                                        }
                                    });
                                });
                            }
                            Err(e) => {
                                tracing::error!("Forge: failed to parse telemetry JSON: {} | Line: {}", e, line);
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        tracing::error!("Forge: telemetry pipe error: {}", e);
                        break; // Trigger reconnection
                    }
                    Ok(None) => {
                        tracing::warn!("Forge: telemetry pipe closed by engine.");
                        break; // Trigger reconnection / exit
                    }
                    Err(_) => {
                        tracing::error!("Forge: Watchdog expired (30s silence). Engine likely terminated. Exiting.");
                        std::process::exit(0);
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // ── Callbacks ─────────────────────────────────────────────────────────────

    // COMMIT CHANGES → save-decree
    ui.on_save_decree({
        let step_model = step_model.clone();
        let decree_model = decree_model.clone();
        let ui_handle = ui_handle.clone();
        move || {
            let active_id = ui_handle.upgrade()
                .map(|u| u.get_active_decree_id().to_string())
                .unwrap_or_default();
            let step_count = step_model.row_count();
            info!(
                decree_id = %active_id,
                step_count,
                "Forge: save-decree — persisting to ledger (Session 2)"
            );
            // TODO Session 2: serialize and send over arbiter_command pipe
            let _ = decree_model; // silence unused warning until Session 2
        }
    });

    // + New decree in sidebar header
    ui.on_new_decree({
        let decree_model = decree_model.clone();
        let step_model   = step_model.clone();
        let ui_handle    = ui_handle.clone();
        move || {
            let id = next_id();
            info!(new_id = %id, "Forge: new-decree");
            decree_model.push(DecreeEntry {
                id:     id.clone(),
                label:  "New Decree".into(),
                status: 0,
            });
            // Clear the step canvas for the new decree
            while step_model.row_count() > 0 {
                step_model.remove(0);
            }
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_active_decree_id(id);
                ui.set_active_decree_label("New Decree".into());
            }
        }
    });

    // Sidebar item click → select-decree
    ui.on_select_decree({
        let decree_model = decree_model.clone();
        let ui_handle    = ui_handle.clone();
        move |id| {
            info!(decree_id = %id, "Forge: select-decree");
            // Find the label for the selected decree and update the topbar
            let label = (0..decree_model.row_count())
                .find_map(|i| {
                    let d = decree_model.row_data(i)?;
                    if d.id == id { Some(d.label) } else { None }
                })
                .unwrap_or_else(|| "Unknown".into());
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_active_decree_id(id);
                ui.set_active_decree_label(label);
            }
            // TODO Session 2: load steps for this decree from the ledger
        }
    });

    // + Append Action Step
    ui.on_append_step({
        let step_model = step_model.clone();
        move |step_type| {
            let id = next_id();
            let (title, subtext, arg_a, arg_b) = match step_type {
                0 => ("Move File",     "Inscribe: relocate artifact",      "${env.file_path}", "C:/Destination/"),
                1 => ("Shell Command", "Shell: execute external program",  "program.exe",      "${env.file_path}"),
                2 => ("Type Text",     "Somatic: emit keystrokes",         "TYPE",             "${env.file_name}"),
                _ => ("Steady Wait",   "Wait for condition to stabilise",  "",                 ""),
            };
            info!(step_type, new_id = %id, "Forge: append-step");
            step_model.push(DecreeStep {
                id,
                title:          title.into(),
                subtext:        subtext.into(),
                step_type,
                is_active:      false,
                baton_required: step_type == 1,
                arg_a:          arg_a.into(),
                arg_b:          arg_b.into(),
            });
        }
    });

    // Remove step (not yet wired to a UI button — callback ready for Session 1b)
    ui.on_remove_step({
        let step_model = step_model.clone();
        move |step_id| {
            info!(step_id = %step_id, "Forge: remove-step");
            for i in 0..step_model.row_count() {
                if let Some(s) = step_model.row_data(i) {
                    if s.id == step_id {
                        step_model.remove(i);
                        break;
                    }
                }
            }
        }
    });

    // ── Run UI ────────────────────────────────────────────────────────────────
    ui.run()?;
    Ok(())
}
