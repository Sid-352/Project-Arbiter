slint::include_modules!();

use std::rc::Rc;
use std::time::Duration;
use slint::{ComponentHandle, Model, ModelRc, VecModel, Color, SharedString};
use tracing::{info, warn};

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
//  Commands
// ─────────────────────────────────────────────────────────────────────────────

async fn send_command(cmd: &arbiter_core::ordinance::ForgeCommand) {
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::io::AsyncWriteExt;
    let pipe_name = r"\\.\pipe\arbiter_command";
    if let Ok(mut client) = ClientOptions::new().open(pipe_name) {
        if let Ok(json) = serde_json::to_string(cmd) {
            let _ = client.write_all(json.as_bytes()).await;
            let _ = client.write_all(b"\n").await;
        }
    }
}

fn collect_ordinance_from_ui(ui: &ArbiterForge) -> arbiter_core::ledger::OrdinanceDef {
    let id = ui.get_active_decree_id().to_string();
    let label = ui.get_active_decree_label().to_string();
    
    let trigger_type = ui.get_summons_trigger_type();
    let summons = match trigger_type {
        0 => arbiter_core::ledger::SummonsDef::FileCreated {
            ward_id: ui.get_summons_path().to_string(),
            glob: ui.get_summons_glob().to_string(),
        },
        1 => arbiter_core::ledger::SummonsDef::Hotkey {
            combo: ui.get_summons_combo().to_string(),
        },
        2 => arbiter_core::ledger::SummonsDef::ProcessAppeared {
            name: ui.get_summons_process().to_string(),
        },
        _ => arbiter_core::ledger::SummonsDef::Manual,
    };

    let mut nodes = Vec::new();
    // Entry node
    nodes.push(arbiter_core::ordinance::OrdNode {
        id: "entry".into(),
        label: "Start".into(),
        kind: arbiter_core::ordinance::NodeKind::Entry,
        internal_state: "".into(),
        next_nodes: std::collections::HashMap::new(),
    });

    // Map DecreeStep -> OrdNode
    STEP_MODEL.with(|m| {
        for i in 0..m.row_count() {
            if let Some(step) = m.row_data(i) {
                let action_type = match step.step_type {
                    0 => arbiter_core::ordinance::ActionType::InscribeMove {
                        source: step.arg_a.to_string().into(),
                        destination: step.arg_b.to_string().into(),
                    },
                    1 => arbiter_core::ordinance::ActionType::Shell {
                        command: step.arg_a.to_string(),
                        args: step.arg_b.split_whitespace().map(|s| s.to_string()).collect(),
                        detached: true,
                    },
                    2 => arbiter_core::ordinance::ActionType::Type(step.arg_a.to_string()),
                    _ => arbiter_core::ordinance::ActionType::Wait(1000),
                };

                let action = arbiter_core::ordinance::Action {
                    action_type,
                    point: None,
                    delay_ms: 0,
                };

                let step_id = format!("id-{}", i + 1);
                let next_id = if i + 1 < m.row_count() {
                    format!("id-{}", i + 2)
                } else {
                    "".to_string()
                };

                let mut next_nodes = std::collections::HashMap::new();
                if !next_id.is_empty() {
                    next_nodes.insert("Next".into(), next_id);
                }

                nodes.push(arbiter_core::ordinance::OrdNode {
                    id: step_id,
                    label: step.title.to_string(),
                    kind: arbiter_core::ordinance::NodeKind::Action,
                    internal_state: serde_json::to_string(&action).unwrap_or_default(),
                    next_nodes,
                });
            }
        }
    });

    // Fix the first node link if we have steps
    if nodes.len() > 1 {
        if let Some(entry) = nodes.iter_mut().find(|n| n.kind == arbiter_core::ordinance::NodeKind::Entry) {
            entry.next_nodes.insert("Next".into(), "id-1".into());
        }
    }

    arbiter_core::ledger::OrdinanceDef {
        id,
        label,
        summons,
        nodes,
        presence_config: arbiter_core::ordinance::PresenceConfig::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Ledger Logic
// ─────────────────────────────────────────────────────────────────────────────

fn load_ledger_into_ui() {
    let ledger = arbiter_core::ledger::load().unwrap_or_default();
    
    DECREE_MODEL.with(|m| {
        while m.row_count() > 0 { m.remove(0); }
        for ord in &ledger.ordinances {
            m.push(DecreeEntry {
                id: SharedString::from(&ord.id),
                label: SharedString::from(&ord.label),
                status: 1, // Ok/Loaded
            });
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Arbiter Forge: Launching Slint Interface");

    // Start with data from disk
    load_ledger_into_ui();

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

    // Select the first decree by default if it exists
    DECREE_MODEL.with(|m| {
        if let Some(first) = m.row_data(0) {
            ui.set_active_decree_id(first.id);
            ui.set_active_decree_label(first.label);
            // selection logic below
        }
    });

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
                        break; 
                    }
                    Ok(None) => {
                        tracing::warn!("Forge: telemetry pipe closed by engine.");
                        break;
                    }
                    Err(_) => {
                        tracing::error!("Forge: Watchdog expired (2s silence). Engine likely terminated. Requesting graceful exit.");
                        let _ = ui_handle_telemetry.upgrade_in_event_loop(|ui| {
                            ui.invoke_request_close();
                        });
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // ── Callbacks ─────────────────────────────────────────────────────────────

    ui.on_request_close(move || {
        info!("Forge: Received close request. Terminating event loop.");
        slint::quit_event_loop().unwrap();
    });

    // COMMIT CHANGES → save-decree
    ui.on_save_decree({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let def = collect_ordinance_from_ui(&ui);
                let cmd = arbiter_core::ordinance::ForgeCommand::SaveDecree(def);
                tokio::spawn(async move {
                    send_command(&cmd).await;
                });
                
                // Refresh sidebar list to reflect any label changes
                load_ledger_into_ui();
            }
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
                // Reset summons properties for new decree
                ui.set_summons_trigger_type(0);
                ui.set_summons_path("".into());
                ui.set_summons_glob("".into());
                ui.set_summons_combo("".into());
                ui.set_summons_process("".into());
            }
        }
    });

    // Sidebar item click → select-decree
    ui.on_select_decree({
        let ui_handle    = ui_handle.clone();
        move |id| {
            info!(decree_id = %id, "Forge: select-decree");
            let ledger = arbiter_core::ledger::load().unwrap_or_default();
            if let Some(ord) = ledger.ordinances.iter().find(|o| id == o.id) {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_active_decree_id(ord.id.clone().into());
                    ui.set_active_decree_label(ord.label.clone().into());
                    
                    // Sync Summons
                    match &ord.summons {
                        arbiter_core::ledger::SummonsDef::FileCreated { ward_id, glob } => {
                            ui.set_summons_trigger_type(0);
                            ui.set_summons_path(ward_id.clone().into());
                            ui.set_summons_glob(glob.clone().into());
                        }
                        arbiter_core::ledger::SummonsDef::Hotkey { combo } => {
                            ui.set_summons_trigger_type(1);
                            ui.set_summons_combo(combo.clone().into());
                        }
                        arbiter_core::ledger::SummonsDef::ProcessAppeared { name } => {
                            ui.set_summons_trigger_type(2);
                            ui.set_summons_process(name.clone().into());
                        }
                        arbiter_core::ledger::SummonsDef::Manual => {
                            ui.set_summons_trigger_type(3);
                        }
                    }

                    // Sync Steps
                    STEP_MODEL.with(|m| {
                        while m.row_count() > 0 { m.remove(0); }
                        for node in &ord.nodes {
                            if node.kind == arbiter_core::ordinance::NodeKind::Action {
                                if let Ok(action) = serde_json::from_str::<arbiter_core::ordinance::Action>(&node.internal_state) {
                                    let (step_type, arg_a, arg_b) = match &action.action_type {
                                        arbiter_core::ordinance::ActionType::InscribeMove { source, destination } => {
                                            (0, source.to_string_lossy().to_string(), destination.to_string_lossy().to_string())
                                        }
                                        arbiter_core::ordinance::ActionType::Shell { command, args, .. } => {
                                            (1, command.clone(), args.join(" "))
                                        }
                                        arbiter_core::ordinance::ActionType::Type(s) => {
                                            (2, s.clone(), "".to_string())
                                        }
                                        arbiter_core::ordinance::ActionType::Wait(ms) => {
                                            (3, ms.to_string(), "".to_string())
                                        }
                                        _ => (3, "".to_string(), "".to_string()),
                                    };

                                    m.push(DecreeStep {
                                        id: node.id.clone().into(),
                                        title: node.label.clone().into(),
                                        subtext: "".into(),
                                        step_type,
                                        is_active: false,
                                        baton_required: step_type == 1,
                                        arg_a: arg_a.into(),
                                        arg_b: arg_b.into(),
                                    });
                                }
                            }
                        }
                    });
                }
            }
        }
    });

    ui.on_step_edited({
        let step_model = step_model.clone();
        move |idx, a, b| {
            if let Some(mut row) = step_model.row_data(idx as usize) {
                if row.arg_a == a && row.arg_b == b {
                    return;
                }
                row.arg_a = a;
                row.arg_b = b;
                step_model.set_row_data(idx as usize, row);
            }
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

    ui.on_remove_decree({
        let ui_handle = ui_handle.clone();
        move |id| {
            info!(decree_id = %id, "Forge: remove-decree");
            let mut ledger = arbiter_core::ledger::load().unwrap_or_default();
            ledger.ordinances.retain(|o| id != o.id);
            let _ = arbiter_core::ledger::save(&ledger);
            
            // Refresh UI
            load_ledger_into_ui();
            
            // If the deleted one was active, clear the canvas
            if let Some(ui) = ui_handle.upgrade() {
                if ui.get_active_decree_id() == id {
                    ui.set_active_decree_id("".into());
                    ui.set_active_decree_label("No Decree Selected".into());
                    STEP_MODEL.with(|m| {
                        while m.row_count() > 0 { m.remove(0); }
                    });
                }
            }
        }
    });

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
