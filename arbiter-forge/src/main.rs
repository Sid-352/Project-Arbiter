slint::include_modules!();

use std::rc::Rc;
use std::time::Duration;
use slint::{ComponentHandle, Model, ModelRc, VecModel, Color, SharedString};
use tracing::info;

use arbiter_core::protocol::{ForgeCommand, LogEntry as WireLogEntry, PIPE_COMMAND, PIPE_TELEMETRY};
use arbiter_core::ordinance::{DecreeId, NodeId, NodeKind, OrdNode, Action, ActionType, PresenceConfig};
use arbiter_core::ledger::SummonsDef;

thread_local! {
    static LOG_MODEL:    Rc<VecModel<LogEntry>>    = Rc::new(VecModel::default());
    static DECREE_MODEL: Rc<VecModel<DecreeEntry>> = Rc::new(VecModel::default());
    static STEP_MODEL:   Rc<VecModel<DecreeStep>>  = Rc::new(VecModel::default());
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tiny helper to generate sequential IDs.
// ─────────────────────────────────────────────────────────────────────────────
fn next_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(1);
    format!("id-{}", CTR.fetch_add(1, Ordering::Relaxed))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Commands
// ─────────────────────────────────────────────────────────────────────────────

async fn send_command(cmd: &ForgeCommand) {
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::io::AsyncWriteExt;
    if let Ok(mut client) = ClientOptions::new().open(PIPE_COMMAND) {
        if let Ok(json) = serde_json::to_string(cmd) {
            let _ = client.write_all(json.as_bytes()).await;
            let _ = client.write_all(b"\n").await;
        }
    }
}

fn collect_ordinance_from_ui(ui: &ArbiterForge) -> arbiter_core::ledger::OrdinanceDef {
    let id = DecreeId(ui.get_active_decree_id().to_string());
    let label = ui.get_active_decree_label().to_string();
    
    let trigger_type = ui.get_summons_trigger_type();
    let summons = match trigger_type {
        0 => SummonsDef::FileCreated {
            ward_id: ui.get_summons_path().to_string(),
            pattern: ui.get_summons_pattern().to_string(),
        },
        1 => SummonsDef::Hotkey {
            combo: ui.get_summons_combo().to_string(),
        },
        2 => SummonsDef::ProcessAppeared {
            name: ui.get_summons_process().to_string(),
        },
        _ => SummonsDef::Manual,
    };

    let mut nodes = Vec::new();
    // Entry node
    nodes.push(OrdNode {
        id: NodeId("entry".into()),
        label: "Start".into(),
        kind: NodeKind::Entry,
        internal_state: "".into(),
        next_nodes: std::collections::HashMap::new(),
    });

    // Map DecreeStep -> OrdNode
    STEP_MODEL.with(|m| {
        for i in 0..m.row_count() {
            if let Some(step) = m.row_data(i) {
                let action_type = match step.step_type {
                    0 => ActionType::InscribeMove {
                        source: step.arg_a.to_string().into(),
                        destination: step.arg_b.to_string().into(),
                    },
                    1 => ActionType::Shell {
                        command: step.arg_a.to_string(),
                        args: step.arg_b.split_whitespace().map(|s| s.to_string()).collect(),
                        detached: true,
                    },
                    2 => ActionType::Type(step.arg_a.to_string()),
                    _ => ActionType::Wait(1000),
                };

                let action = Action {
                    action_type,
                    point: None,
                    delay_ms: 0,
                };

                let step_id = NodeId(step.id.to_string());
                let next_id = if i + 1 < m.row_count() {
                    if let Some(next_step) = m.row_data(i + 1) {
                        Some(NodeId(next_step.id.to_string()))
                    } else { None }
                } else {
                    None
                };

                let mut next_nodes = std::collections::HashMap::new();
                if let Some(nid) = next_id {
                    next_nodes.insert("Next".into(), nid);
                }

                nodes.push(OrdNode {
                    id: step_id,
                    label: step.title.to_string(),
                    kind: NodeKind::Action,
                    internal_state: serde_json::to_string(&action).unwrap_or_default(),
                    next_nodes,
                });
            }
        }
    });

    // Fix the first node link if we have steps
    if nodes.len() > 1 {
        let first_action_id = nodes[1].id.clone();
        if let Some(entry) = nodes.iter_mut().find(|n| n.kind == NodeKind::Entry) {
            entry.next_nodes.insert("Next".into(), first_action_id);
        }
    }

    arbiter_core::ledger::OrdinanceDef {
        id,
        label,
        summons,
        nodes,
        presence_config: PresenceConfig::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Ledger Logic
// ─────────────────────────────────────────────────────────────────────────────

fn sync_ledger_to_ui() {
    let ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
        tracing::error!("Forge: Failed to load ledger: {}", e);
        arbiter_core::ledger::ArbiterLedger::default()
    });
    
    DECREE_MODEL.with(|m| {
        // Simple reconciliation: Update in-place to avoid flicker
        let mut model_indices = std::collections::HashMap::new();
        for i in 0..m.row_count() {
            if let Some(row) = m.row_data(i) {
                model_indices.insert(row.id.to_string(), i);
            }
        }

        let mut seen_ids = std::collections::HashSet::new();

        for ord in &ledger.ordinances {
            let id_str = ord.id.0.clone();
            seen_ids.insert(id_str.clone());

            let entry = DecreeEntry {
                id: SharedString::from(&id_str),
                label: SharedString::from(&ord.label),
                status: 1, // Ok/Loaded
            };

            if let Some(&idx) = model_indices.get(&id_str) {
                m.set_row_data(idx, entry);
            } else {
                m.push(entry);
            }
        }

        // Remove rows no longer in ledger
        for i in (0..m.row_count()).rev() {
            if let Some(row) = m.row_data(i) {
                if !seen_ids.contains(&row.id.to_string()) {
                    m.remove(i);
                }
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Arbiter Forge: Launching Slint Interface");

    let ui = ArbiterForge::new()?;
    let ui_handle = ui.as_weak();

    // ── Push models into the UI ───────────────────────────────────────────────
    let log_model     = LOG_MODEL.with(|m| m.clone());
    let decree_model  = DECREE_MODEL.with(|m| m.clone());
    let step_model    = STEP_MODEL.with(|m| m.clone());

    ui.set_telemetry_logs(ModelRc::from(log_model.clone()));
    ui.set_decree_list(ModelRc::from(decree_model.clone()));
    ui.set_decree_steps(ModelRc::from(step_model.clone()));

    // Sync with data from disk
    sync_ledger_to_ui();

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
            let _ = ui_handle.upgrade_in_event_loop(move |ui| {
                ui.set_active_decree_id(first.id);
                ui.set_active_decree_label(first.label);
                ui.set_active_decree_status(first.status);
                // Trigger selection logic manually
                ui.invoke_select_decree(ui.get_active_decree_id());
            });
        }
    });

    // ── Telemetry: Named Pipe from arbiter-app ────────────────────────────────
    let ui_handle_telemetry = ui_handle.clone();
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ClientOptions;
        use tokio_util::codec::{FramedRead, LinesCodec};
        use futures::StreamExt;
        use tokio::time::timeout;

        let watchdog_duration = Duration::from_secs(2);

        loop {
            let client = match ClientOptions::new().open(PIPE_TELEMETRY) {
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
                        match serde_json::from_str::<WireLogEntry>(&line) {
                            Ok(core_entry) => {
                                if core_entry.tag == "VIGIL" && core_entry.message.contains("Heartbeat") {
                                    continue;
                                }

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

                                let time_str = if core_entry.time.is_empty() {
                                    chrono::Local::now().format("%H:%M:%S").to_string()
                                } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&core_entry.time) {
                                    dt.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()
                                } else {
                                    core_entry.time.clone()
                                };

                                let entry = LogEntry {
                                    time:      time_str.into(),
                                    tag:       core_entry.tag.into(),
                                    msg:       core_entry.message.into(),
                                    tag_color,
                                    ordinance_id: core_entry.ordinance_id.unwrap_or_default().into(),
                                };

                                let _ = ui_copy.upgrade_in_event_loop(move |ui| {
                                    LOG_MODEL.with(|m| {
                                        m.push(entry);
                                        if m.row_count() > 50 {
                                            m.remove(0);
                                        }
                                    });
                                    ui.invoke_scroll_logs_to_bottom();
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
                // Validate before sending (Phase 3.2)
                if let Err(e) = def.validate() {
                    LOG_MODEL.with(|m| {
                        m.push(LogEntry {
                            time: chrono::Local::now().format("%H:%M:%S").to_string().into(),
                            tag: "VALIDATE".into(),
                            tag_color: Color::from_rgb_u8(244, 63, 94),
                            msg: format!("Validation Error: {}", e).into(),
                            ordinance_id: "".into(),
                        });
                    });
                    return;
                }

                let cmd = ForgeCommand::SaveDecree(def);
                tokio::spawn(async move {
                    send_command(&cmd).await;
                });
                
                // Refresh sidebar list to reflect any label changes
                sync_ledger_to_ui();
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
                id:     id.clone().into(),
                label:  "New Decree".into(),
                status: 0,
            });
            // Clear the step canvas for the new decree
            while step_model.row_count() > 0 {
                step_model.remove(0);
            }
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_active_decree_id(id.into());
                ui.set_active_decree_label("New Decree".into());
                ui.set_active_decree_status(0);
                ui.set_selected_step_id("".into());
                ui.set_summons_trigger_type(0);
                ui.set_summons_path("".into());
                ui.set_summons_pattern("".into());
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
            let ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
                tracing::error!("Forge: Failed to load ledger for selection: {}", e);
                arbiter_core::ledger::ArbiterLedger::default()
            });
            if let Some(ord) = ledger.ordinances.iter().find(|o| id == o.id.0) {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_active_decree_id(ord.id.0.clone().into());
                    ui.set_active_decree_label(ord.label.clone().into());
                    ui.set_active_decree_status(1); 
                    ui.set_selected_step_id("".into());
                    
                    // Clear all Summons fields first to prevent 'bleeding'
                    ui.set_summons_path("".into());
                    ui.set_summons_pattern("".into());
                    ui.set_summons_combo("".into());
                    ui.set_summons_process("".into());
                    
                    // Sync Summons
                    match &ord.summons {
                        SummonsDef::FileCreated { ward_id, pattern } => {
                            ui.set_summons_trigger_type(0);
                            ui.set_summons_path(ward_id.clone().into());
                            ui.set_summons_pattern(pattern.clone().into());
                        }
                        SummonsDef::Hotkey { combo } => {
                            ui.set_summons_trigger_type(1);
                            ui.set_summons_combo(combo.clone().into());
                        }
                        SummonsDef::ProcessAppeared { name } => {
                            ui.set_summons_trigger_type(2);
                            ui.set_summons_process(name.clone().into());
                        }
                        SummonsDef::Manual => {
                            ui.set_summons_trigger_type(3);
                        }
                    }

                    // Sync Steps (Reconciliation)
                    STEP_MODEL.with(|m| {
                        let mut incoming_steps = Vec::new();
                        for node in &ord.nodes {
                            if node.kind == NodeKind::Action {
                                if let Ok(action) = serde_json::from_str::<Action>(&node.internal_state) {
                                    let (step_type, arg_a, arg_b) = match &action.action_type {
                                        ActionType::InscribeMove { source, destination } => {
                                            (0, source.to_string_lossy().to_string(), destination.to_string_lossy().to_string())
                                        }
                                        ActionType::Shell { command, args, .. } => {
                                            (1, command.clone(), args.join(" "))
                                        }
                                        ActionType::Type(s) => {
                                            (2, s.clone(), "".to_string())
                                        }
                                        ActionType::Wait(ms) => {
                                            (3, ms.to_string(), "".to_string())
                                        }
                                        _ => (3, "".to_string(), "".to_string()),
                                    };

                                    incoming_steps.push(DecreeStep {
                                        id: node.id.0.clone().into(),
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

                        // Update STEP_MODEL in-place
                        while m.row_count() > incoming_steps.len() {
                            m.remove(m.row_count() - 1);
                        }
                        for (i, step) in incoming_steps.into_iter().enumerate() {
                            if i < m.row_count() {
                                m.set_row_data(i, step);
                            } else {
                                m.push(step);
                            }
                        }
                    });
                }
            }
        }
    });

    ui.on_step_edited({
        let step_model = step_model.clone();
        move |id, a, b| {
            for i in 0..step_model.row_count() {
                if let Some(mut row) = step_model.row_data(i) {
                    if row.id == id {
                        if row.arg_a == a && row.arg_b == b {
                            return;
                        }
                        row.arg_a = a;
                        row.arg_b = b;
                        step_model.set_row_data(i, row);
                        break;
                    }
                }
            }
        }
    });

    // + Append Action Step
    ui.on_append_step({
        let step_model = step_model.clone();
        let ui_handle = ui_handle.clone();
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
                id:             id.clone().into(),
                title:          title.into(),
                subtext:        subtext.into(),
                step_type,
                is_active:      false,
                baton_required: step_type == 1,
                arg_a:          arg_a.into(),
                arg_b:          arg_b.into(),
            });
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_selected_step_id(id.into());
            }
        }
    });

    ui.on_remove_decree({
        let ui_handle = ui_handle.clone();
        move |id| {
            info!(decree_id = %id, "Forge: remove-decree");
            let mut ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
                tracing::error!("Forge: Failed to load ledger for removal: {}", e);
                arbiter_core::ledger::ArbiterLedger::default()
            });
            ledger.ordinances.retain(|o| id != o.id.0);
            let _ = arbiter_core::ledger::save(&ledger);
            
            sync_ledger_to_ui();
            
            if let Some(ui) = ui_handle.upgrade() {
                if ui.get_active_decree_id() == id {
                    ui.set_active_decree_id("".into());
                    ui.set_active_decree_label("No Decree Selected".into());
                    ui.set_active_decree_status(0);
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

    ui.on_copy_env(move |text| {
        #[cfg(windows)]
        {
            use std::process::{Command, Stdio};
            use std::io::Write;
            info!("Copying to clipboard: {}", text);
            if let Ok(mut child) = Command::new("clip").stdin(Stdio::piped()).spawn() {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }
    });

    ui.on_active_decree_renamed(move |id, new_label| {
        info!(id = %id, label = %new_label, "Forge: active-decree-renamed");
        DECREE_MODEL.with(|m| {
            for i in 0..m.row_count() {
                if let Some(mut entry) = m.row_data(i) {
                    if entry.id == id {
                        entry.label = new_label.clone().into();
                        m.set_row_data(i, entry);
                        break;
                    }
                }
            }
        });

        // Also update in ledger file
        let mut ledger = arbiter_core::ledger::load().unwrap_or_else(|_| arbiter_core::ledger::ArbiterLedger::default());
        if let Some(ord) = ledger.ordinances.iter_mut().find(|o| o.id.0 == id.as_str()) {
            ord.label = new_label.to_string();
            let _ = arbiter_core::ledger::save(&ledger);
        }
    });

    // ── Run UI ────────────────────────────────────────────────────────────────
    ui.run()?;
    Ok(())
}
