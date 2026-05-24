#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod ipc;

use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::time::Duration;
use tracing::info;

use arbiter_core::decree::{
    Action, ActionType, DecreeId, DecreeNode, NodeId, NodeKind, PresenceConfig,
};
use arbiter_core::ledger::SummonsDef;
use arbiter_core::normalize_windows_path;
use arbiter_core::protocol::ForgeCommand;

thread_local! {
    static LOG_MODEL:    Rc<VecModel<LogEntry>>    = Rc::new(VecModel::default());
    static DECREE_MODEL: Rc<VecModel<DecreeEntry>> = Rc::new(VecModel::default());
    static STEP_MODEL:   Rc<VecModel<DecreeStep>>  = Rc::new(VecModel::default());
    static WARD_MODEL:   Rc<VecModel<WardEntry>>   = Rc::new(VecModel::default());
    static TS_PATH_MODEL: Rc<VecModel<SharedString>> = Rc::new(VecModel::default());
    static BATON_MODEL:  Rc<VecModel<SharedString>> = Rc::new(VecModel::default());
}

fn generate_decree_id(label: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(1);

    let slug: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .filter(|&c| c.is_alphanumeric() || c == '-')
        .collect();

    if slug.is_empty() {
        format!("id-{id}", id = CTR.fetch_add(1, Ordering::Relaxed))
    } else {
        slug
    }
}

fn generate_step_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static STEP_CTR: AtomicU32 = AtomicU32::new(1);

    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let n = STEP_CTR.fetch_add(1, Ordering::Relaxed);
    format!("step-{epoch}-{n}")
}

fn collect_decree_from_ui(ui: &ArbiterForge) -> arbiter_core::ledger::DecreeDef {
    let label = ui.get_active_decree_label().to_string();
    let id_str = ui.get_active_decree_id().to_string();

    let id = if id_str.is_empty() {
        DecreeId(generate_decree_id(&label))
    } else {
        DecreeId(id_str)
    };

    let _trigger_type = ui.get_summons_trigger_type();
    let summons = match ui.get_summons_trigger_type() {
        0 => SummonsDef::FileCreated {
            ward_id: normalize_windows_path(ui.get_summons_path().as_ref()),
            pattern: ui.get_summons_pattern().to_string(),
            recursive: ui.get_summons_ward_recursive(),
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
    nodes.push(DecreeNode {
        id: NodeId("entry".into()),
        label: "Start".into(),
        state: arbiter_core::decree::NodeState::Empty,
        next_nodes: std::collections::HashMap::new(),
    });

    STEP_MODEL.with(|m| {
        for i in 0..m.row_count() {
            if let Some(step) = m.row_data(i) {
                let action_type = match step.step_type {
                    0 => {
                        if step.subtext.contains("Copy") {
                            ActionType::InscribeCopy {
                                source: step.arg_a.to_string().into(),
                                destination: step.arg_b.to_string().into(),
                            }
                        } else {
                            ActionType::InscribeMove {
                                source: step.arg_a.to_string().into(),
                                destination: step.arg_b.to_string().into(),
                            }
                        }
                    }
                    1 => ActionType::Shell {
                        command: step.arg_a.to_string(),
                        args: step
                            .arg_b
                            .split_whitespace()
                            .map(std::string::ToString::to_string)
                            .collect(),
                        detached: true,
                    },
                    2 => ActionType::Type(step.arg_a.to_string()),
                    3 => ActionType::Wait(step.arg_a.parse().unwrap_or(1000)),
                    4 => ActionType::Navigate(step.arg_a.to_string()),
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
                    } else {
                        None
                    }
                } else {
                    None
                };

                let mut next_nodes = std::collections::HashMap::new();
                if let Some(nid) = next_id {
                    next_nodes.insert("success".into(), nid);
                }

                nodes.push(DecreeNode {
                    id: step_id,
                    label: step.title.to_string(),
                    state: arbiter_core::decree::NodeState::Action {
                        action_type: action.action_type,
                        point: action.point,
                        delay_ms: action.delay_ms,
                    },
                    next_nodes,
                });
            }
        }
    });

    if nodes.len() > 1 {
        let first_action_id = nodes[1].id.clone();
        if let Some(entry) = nodes.iter_mut().find(|n| n.kind() == NodeKind::Entry) {
            entry.next_nodes.insert("success".into(), first_action_id);
        }
    }

    arbiter_core::ledger::DecreeDef {
        id,
        label,
        summons,
        nodes,
        presence_config: PresenceConfig {
            ignore_mouse: ui.get_presence_ignore_mouse(),
            ignore_keyboard: ui.get_presence_ignore_keyboard(),
        },
    }
}

fn next_new_decree_label() -> String {
    let mut max_n = 0u32;
    DECREE_MODEL.with(|m| {
        for i in 0..m.row_count() {
            if let Some(entry) = m.row_data(i) {
                let label = entry.label.to_string();
                if let Some(suffix) = label.strip_prefix("New Decree ") {
                    if let Ok(n) = suffix.trim().parse::<u32>() {
                        max_n = max_n.max(n);
                    }
                }
            }
        }
    });
    format!("New Decree {n}", n = max_n + 1)
}

fn sync_ledger_to_ui() {
    let ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
        tracing::error!("Forge: Failed to load ledger: {}", e);
        arbiter_core::ledger::ArbiterLedger::default()
    });

    DECREE_MODEL.with(|m| {
        let mut model_indices = std::collections::HashMap::new();
        for i in 0..m.row_count() {
            if let Some(row) = m.row_data(i) {
                model_indices.insert(row.id.to_string(), i);
            }
        }

        let mut seen_ids = std::collections::HashSet::new();

        for ord in &ledger.decrees {
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

    WARD_MODEL.with(|m| {
        let mut model_indices = std::collections::HashMap::new();
        for i in 0..m.row_count() {
            if let Some(row) = m.row_data(i) {
                model_indices.insert(row.id.to_string(), i);
            }
        }
        let mut seen_ids = std::collections::HashSet::new();
        for ward in &ledger.wards {
            let id_str = ward.id.clone();
            seen_ids.insert(id_str.clone());
            let entry = WardEntry {
                id: SharedString::from(&id_str),
                path: SharedString::from(ward.path.to_string_lossy().as_ref()),
                pattern: SharedString::from(&ward.pattern),
                recursive: ward.recursive,
                layer: match ward.layer {
                    arbiter_core::decree::WardLayer::Surface => 0,
                    arbiter_core::decree::WardLayer::Analytical => 1,
                },
            };
            if let Some(&idx) = model_indices.get(&id_str) {
                m.set_row_data(idx, entry);
            } else {
                m.push(entry);
            }
        }
        for i in (0..m.row_count()).rev() {
            if let Some(row) = m.row_data(i) {
                if !seen_ids.contains(&row.id.to_string()) {
                    m.remove(i);
                }
            }
        }
    });
}

fn sync_signet_to_ui(ui: &ArbiterForge) {
    let signet = arbiter_core::signet::load().unwrap_or_default();

    ui.set_launch_on_startup(signet.launch_on_startup);

    TS_PATH_MODEL.with(|m| {
        while m.row_count() > 0 {
            m.remove(0);
        }
        for p in &signet.trusted_paths {
            m.push(SharedString::from(p));
        }
    });

    BATON_MODEL.with(|m| {
        while m.row_count() > 0 {
            m.remove(0);
        }
        for b in &signet.baton_allowed {
            m.push(SharedString::from(b));
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("SLINT_STYLE", "fluent");
    tracing_subscriber::fmt::init();
    info!("Arbiter Forge: Launching Slint Interface");

    if !ipc::app_is_available(Duration::from_secs(4)).await {
        let _ = rfd::MessageDialog::new()
            .set_title("Arbiter Forge")
            .set_description("Arbiter App is not running. Start the app first.")
            .set_level(rfd::MessageLevel::Error)
            .show();
        return Ok(());
    }

    let ui = ArbiterForge::new()?;
    let ui_handle = ui.as_weak();

    let log_model = LOG_MODEL.with(std::clone::Clone::clone);
    let decree_model = DECREE_MODEL.with(std::clone::Clone::clone);
    let step_model = STEP_MODEL.with(std::clone::Clone::clone);
    let ward_model = WARD_MODEL.with(std::clone::Clone::clone);
    let ts_path_model = TS_PATH_MODEL.with(std::clone::Clone::clone);
    let baton_model = BATON_MODEL.with(std::clone::Clone::clone);

    ui.set_telemetry_logs(ModelRc::from(log_model.clone()));
    ui.set_decree_list(ModelRc::from(decree_model.clone()));
    ui.set_decree_steps(ModelRc::from(step_model.clone()));
    ui.set_ward_list(ModelRc::from(ward_model));
    ui.set_trusted_paths(ModelRc::from(ts_path_model));
    ui.set_baton_allowed(ModelRc::from(baton_model));

    sync_ledger_to_ui();
    sync_signet_to_ui(&ui);

    log_model.push(LogEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string().into(),
        tag: "FORGE".into(),
        tag_color: Color::from_rgb_u8(99, 102, 241),
        msg: "Terminal interface active. Waiting for telemetry pipe...".into(),
        decree_id: "".into(),
    });

    DECREE_MODEL.with(|m| {
        if let Some(first) = m.row_data(0) {
            let _ = ui_handle.upgrade_in_event_loop(move |ui| {
                ui.set_active_decree_id(first.id);
                ui.set_active_decree_label(first.label);
                ui.set_active_decree_status(0);
                ui.invoke_select_decree(ui.get_active_decree_id());
            });
        }
    });

    ipc::spawn_telemetry_listener(ui_handle.clone());

    ui.on_request_close(move || {
        info!("Forge: Received close request. Terminating event loop.");
        slint::quit_event_loop().unwrap();
    });

    ui.on_save_decree({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let def = collect_decree_from_ui(&ui);
                ui.set_active_decree_id(def.id.0.clone().into());

                if let Err(e) = def.validate() {
                    tracing::error!("Forge: Validation failed for decree: {}", e);
                    return;
                }

                let mut ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
                    tracing::error!("Forge: Failed to load ledger for saving: {}", e);
                    arbiter_core::ledger::ArbiterLedger::default()
                });

                if let arbiter_core::ledger::SummonsDef::FileCreated {
                    ward_id, recursive, ..
                } = &def.summons
                {
                    let mut found = false;
                    for ward in &mut ledger.wards {
                        if ward.id == *ward_id {
                            ward.recursive = *recursive;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        ledger.wards.push(arbiter_core::decree::WardConfig {
                            id: ward_id.clone(),
                            path: std::path::PathBuf::from(ward_id.clone()),
                            pattern: String::new(), // Ward pattern is ignored in Arbiter in favor of Summons pattern
                            recursive: *recursive,
                            layer: arbiter_core::decree::WardLayer::Surface,
                        });
                    }
                }

                if let Some(existing) = ledger.decrees.iter_mut().find(|o| o.id == def.id) {
                    existing.clone_from(&def);
                } else {
                    ledger.decrees.push(def.clone());
                }

                if let Err(e) = arbiter_core::ledger::save(&ledger) {
                    tracing::error!("Forge: Failed to save ledger to disk: {}", e);
                } else {
                    info!("Forge: Decree '{}' saved directly to disk", def.label);
                    sync_ledger_to_ui();
                }

                let cmd = ForgeCommand::SaveDecree(def);
                tokio::spawn(async move {
                    ipc::send_command(&cmd).await;
                });
            }
        }
    });

    ui.on_simulate_decree({
        let _ui_handle = ui_handle.clone();
        move || {
            if let Some(ui) = _ui_handle.upgrade() {
                let def = collect_decree_from_ui(&ui);
                if let Err(e) = def.validate() {
                    LOG_MODEL.with(|m| {
                        m.push(LogEntry {
                            time: chrono::Local::now().format("%H:%M:%S").to_string().into(),
                            tag: "VALIDATE".into(),
                            tag_color: Color::from_rgb_u8(244, 63, 94),
                            msg: format!("Validation Error: {e}").into(),
                            decree_id: "".into(),
                        });
                    });
                    return;
                }

                let key = match &def.summons {
                    SummonsDef::FileCreated {
                        ward_id, pattern, ..
                    } => format!("FileCreated|{ward_id}|{pattern}"),
                    SummonsDef::Hotkey { combo } => format!("Hotkey|{combo}"),
                    SummonsDef::ProcessAppeared { name } => format!("ProcessAppeared|{name}"),
                    SummonsDef::Clipboard => "Clipboard".to_string(),
                    SummonsDef::Manual => "Manual".to_string(),
                };
                let save_cmd = ForgeCommand::SaveDecree(def);
                let run_cmd = ForgeCommand::ManualRun {
                    summons_key: key,
                    dry_run: true,
                };
                tokio::spawn(async move {
                    ipc::send_command(&save_cmd).await;
                    ipc::send_command(&run_cmd).await;
                });
            }
        }
    });

    ui.on_new_decree({
        let decree_model = decree_model;
        let step_model = step_model.clone();
        let ui_handle = ui_handle.clone();
        move || {
            let label = next_new_decree_label();
            let id = generate_decree_id(&label);
            info!(new_id = %id, "Forge: new-decree");
            decree_model.push(DecreeEntry {
                id: id.clone().into(),
                label: label.clone().into(),
                status: 0,
            });
            while step_model.row_count() > 0 {
                step_model.remove(0);
            }
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_active_decree_id(id.into());
                ui.set_active_decree_label(label.into());
                ui.set_active_decree_status(0);
                ui.set_selected_step_id("".into());
                ui.set_presence_ignore_mouse(false);
                ui.set_presence_ignore_keyboard(false);
                ui.set_summons_trigger_type(0);
                ui.set_summons_path("".into());
                ui.set_summons_pattern("".into());
                ui.set_summons_combo("".into());
                ui.set_summons_process("".into());
            }
        }
    });

    ui.on_select_decree({
        let _ui_handle = ui_handle.clone();
        move |id| {
            if id.is_empty() {
                return;
            }
            if let Some(ui) = _ui_handle.upgrade() {
                // If this is already the active decree, don't reload from disk (prevents wiping unsaved edits)
                if ui.get_active_decree_id() == id && ui.get_active_decree_status() != 0 {
                    return;
                }

                info!(decree_id = %id, "Forge: select-decree");
                let ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
                    tracing::error!("Forge: Failed to load ledger for selection: {}", e);
                    arbiter_core::ledger::ArbiterLedger::default()
                });
                if let Some(ord) = ledger.decrees.iter().find(|o| id == o.id.0) {
                    ui.set_active_decree_id(ord.id.0.clone().into());
                    ui.set_active_decree_label(ord.label.clone().into());
                    ui.set_active_decree_status(1);
                    ui.set_selected_step_id("".into());
                    ui.set_presence_ignore_mouse(ord.presence_config.ignore_mouse);
                    ui.set_presence_ignore_keyboard(ord.presence_config.ignore_keyboard);

                    ui.set_summons_path("".into());
                    ui.set_summons_pattern("".into());
                    ui.set_summons_combo("".into());
                    ui.set_summons_process("".into());

                    match &ord.summons {
                        SummonsDef::FileCreated {
                            ward_id,
                            pattern,
                            recursive,
                        } => {
                            ui.set_summons_trigger_type(0);
                            ui.set_summons_path(normalize_windows_path(ward_id).into());
                            ui.set_summons_pattern(pattern.clone().into());
                            ui.set_summons_ward_recursive(*recursive);
                        }
                        SummonsDef::Hotkey { combo } => {
                            ui.set_summons_trigger_type(1);
                            ui.set_summons_combo(combo.clone().into());
                        }
                        SummonsDef::ProcessAppeared { name } => {
                            ui.set_summons_trigger_type(2);
                            ui.set_summons_process(name.clone().into());
                        }
                        SummonsDef::Clipboard => {
                            ui.set_summons_trigger_type(4);
                        }
                        SummonsDef::Manual => {
                            ui.set_summons_trigger_type(3);
                        }
                    }

                    STEP_MODEL.with(|m| {
                        let mut incoming_steps = Vec::new();
                        for node in &ord.nodes {
                            if node.kind() == NodeKind::Action {
                                if let arbiter_core::decree::NodeState::Action {
                                    action_type, ..
                                } = &node.state
                                {
                                    let (step_type, arg_a, arg_b, subtext) = match action_type {
                                        ActionType::InscribeMove {
                                            source,
                                            destination,
                                        } => (
                                            0,
                                            source.to_string_lossy().to_string(),
                                            destination.to_string_lossy().to_string(),
                                            "Inscribe: Move Mode".to_string(),
                                        ),
                                        ActionType::InscribeCopy {
                                            source,
                                            destination,
                                        } => (
                                            0,
                                            source.to_string_lossy().to_string(),
                                            destination.to_string_lossy().to_string(),
                                            "Inscribe: Copy Mode".to_string(),
                                        ),
                                        ActionType::Shell { command, args, .. } => (
                                            1,
                                            command.clone(),
                                            args.join(" "),
                                            "Shell: execute program".to_string(),
                                        ),
                                        ActionType::Type(s) => (
                                            2,
                                            s.clone(),
                                            String::new(),
                                            "Synthetic: emit keys".to_string(),
                                        ),
                                        ActionType::Wait(ms) => (
                                            3,
                                            ms.to_string(),
                                            String::new(),
                                            "Steady Wait".to_string(),
                                        ),
                                        ActionType::Navigate(s) => {
                                            (4, s.clone(), String::new(), "Navigate".to_string())
                                        }
                                        _ => (5, String::new(), String::new(), String::new()),
                                    };

                                    incoming_steps.push(DecreeStep {
                                        id: node.id.0.clone().into(),
                                        title: node.label.clone().into(),
                                        subtext: subtext.into(),
                                        step_type,
                                        is_active: false,
                                        is_running: false,
                                        baton_required: step_type == 1,
                                        arg_a: arg_a.into(),
                                        arg_b: arg_b.into(),
                                    });
                                }
                            }
                        }

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
        move |id, title, a, b| {
            for i in 0..step_model.row_count() {
                if let Some(mut row) = step_model.row_data(i) {
                    if row.id == id {
                        // Special case: If this is an Inscribe step (type 0),
                        // toggling the mode via the button should flip the subtext.
                        if row.step_type == 0
                            && row.title == title
                            && row.arg_a == a
                            && row.arg_b == b
                        {
                            if row.subtext.contains("Move") {
                                row.subtext = "Inscribe: Copy Mode".into();
                            } else {
                                row.subtext = "Inscribe: Move Mode".into();
                            }
                        } else {
                            row.title = title;
                            row.arg_a = a;
                            row.arg_b = b;
                        }
                        step_model.set_row_data(i, row);
                        break;
                    }
                }
            }
        }
    });

    ui.on_append_step({
        let step_model = step_model.clone();
        let ui_handle = ui_handle.clone();
        move |step_type| {
            let id = generate_step_id();
            let (title, subtext, arg_a, arg_b) = match step_type {
                0 => (
                    "Move File",
                    "Inscribe: relocate artifact",
                    "${env.file_path}",
                    "C:\\Destination\\",
                ),
                1 => (
                    "Shell Command",
                    "Shell: execute external program",
                    "program.exe",
                    "${env.file_path}",
                ),
                2 => (
                    "Type Text",
                    "Synthetic: emit keystrokes",
                    "TYPE",
                    "${env.file_name}",
                ),
                3 => ("Steady Wait", "Wait for condition to stabilise", "1000", ""),
                4 => ("Navigate", "OS-native navigation keystroke", "win+s", ""),
                _ => ("Action", "Arbiter node", "", ""),
            };
            info!(step_type, new_id = %id, "Forge: append-step");
            step_model.push(DecreeStep {
                id: id.clone().into(),
                title: title.into(),
                subtext: subtext.into(),
                step_type,
                is_active: false,
                is_running: false,
                baton_required: step_type == 1,
                arg_a: arg_a.into(),
                arg_b: arg_b.into(),
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
                tracing::error!("Forge: Failed to load ledger for remove: {}", e);
                arbiter_core::ledger::ArbiterLedger::default()
            });
            let id_str = id.to_string();
            let before = ledger.decrees.len();
            ledger.decrees.retain(|d| d.id.0 != id_str);
            if ledger.decrees.len() != before {
                if let Err(e) = arbiter_core::ledger::save(&ledger) {
                    tracing::error!("Forge: Failed to persist decree removal: {}", e);
                } else {
                    info!(decree_id = %id_str, "Forge: Decree removed directly from disk");
                }
            }

            let cmd = ForgeCommand::RemoveDecree { decree_id: id_str };
            tokio::spawn(async move {
                ipc::send_command(&cmd).await;
            });

            if let Some(ui) = ui_handle.upgrade() {
                if ui.get_active_decree_id() == id {
                    ui.set_active_decree_id("".into());
                    ui.set_active_decree_label("No Decree Selected".into());
                    ui.set_active_decree_status(0);
                    STEP_MODEL.with(|m| {
                        while m.row_count() > 0 {
                            m.remove(0);
                        }
                    });
                }
            }
        }
    });

    ui.on_remove_step({
        let step_model = step_model;
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
            use std::io::Write;
            use std::process::{Command, Stdio};
            info!("Copying to clipboard: {text}");
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
        if id.is_empty() || new_label.trim().is_empty() {
            return;
        }
        DECREE_MODEL.with(|m| {
            for i in 0..m.row_count() {
                if let Some(mut entry) = m.row_data(i) {
                    if entry.id == id {
                        entry.label = new_label.clone();
                        m.set_row_data(i, entry);
                        break;
                    }
                }
            }
        });

        let cmd = ForgeCommand::RenameDecree {
            decree_id: id.to_string(),
            label: new_label.to_string(),
        };
        tokio::spawn(async move {
            ipc::send_command(&cmd).await;
        });
    });

    let ward_model_cb = WARD_MODEL.with(std::clone::Clone::clone);
    ui.on_add_ward({
        let ward_model_cb = ward_model_cb.clone();
        move || {
            let id = generate_decree_id("");
            ward_model_cb.push(WardEntry {
                id: id.into(),
                path: "".into(),
                pattern: "".into(),
                recursive: true,
                layer: 0,
            });
        }
    });

    ui.on_remove_ward({
        let ward_model_cb = ward_model_cb.clone();
        move |id| {
            for i in 0..ward_model_cb.row_count() {
                if let Some(w) = ward_model_cb.row_data(i) {
                    if w.id == id {
                        ward_model_cb.remove(i);
                        break;
                    }
                }
            }
        }
    });

    ui.on_set_ward_layer({
        let ward_model_cb = ward_model_cb.clone();
        move |id, layer| {
            for i in 0..ward_model_cb.row_count() {
                if let Some(mut w) = ward_model_cb.row_data(i) {
                    if w.id == id {
                        w.layer = layer;
                        ward_model_cb.set_row_data(i, w);
                        break;
                    }
                }
            }
        }
    });

    ui.on_update_ward_path({
        let ward_model_cb = ward_model_cb.clone();
        move |id, path| {
            for i in 0..ward_model_cb.row_count() {
                if let Some(mut w) = ward_model_cb.row_data(i) {
                    if w.id == id {
                        w.path = path;
                        ward_model_cb.set_row_data(i, w);
                        break;
                    }
                }
            }
        }
    });

    ui.on_update_ward_pattern({
        let ward_model_cb = ward_model_cb.clone();
        move |id, pattern| {
            for i in 0..ward_model_cb.row_count() {
                if let Some(mut w) = ward_model_cb.row_data(i) {
                    if w.id == id {
                        w.pattern = pattern;
                        ward_model_cb.set_row_data(i, w);
                        break;
                    }
                }
            }
        }
    });

    ui.on_toggle_ward_recursive({
        let ward_model_cb = ward_model_cb.clone();
        move |id| {
            for i in 0..ward_model_cb.row_count() {
                if let Some(mut w) = ward_model_cb.row_data(i) {
                    if w.id == id {
                        w.recursive = !w.recursive;
                        ward_model_cb.set_row_data(i, w);
                        break;
                    }
                }
            }
        }
    });

    let ts_path_model_cb = TS_PATH_MODEL.with(std::clone::Clone::clone);
    ui.on_add_trusted_path({
        let ts_path_model_cb = ts_path_model_cb.clone();
        move || {
            ts_path_model_cb.push(SharedString::from(""));
        }
    });

    ui.on_update_trusted_path({
        let ts_path_model_cb = ts_path_model_cb.clone();
        move |idx, val| {
            if idx >= 0 && (idx as usize) < ts_path_model_cb.row_count() {
                ts_path_model_cb.set_row_data(idx as usize, val);
            }
        }
    });

    ui.on_remove_trusted_path({
        let ts_path_model_cb = ts_path_model_cb.clone();
        move |idx| {
            if idx >= 0 && (idx as usize) < ts_path_model_cb.row_count() {
                ts_path_model_cb.remove(idx as usize);
            }
        }
    });

    let baton_model_cb = BATON_MODEL.with(std::clone::Clone::clone);
    ui.on_add_baton({
        let baton_model_cb = baton_model_cb.clone();
        move || {
            baton_model_cb.push("".into());
        }
    });

    ui.on_update_baton({
        let baton_model_cb = baton_model_cb.clone();
        move |idx, val| {
            if idx >= 0 && (idx as usize) < baton_model_cb.row_count() {
                baton_model_cb.set_row_data(idx as usize, val);
            }
        }
    });

    ui.on_remove_baton({
        let baton_model_cb = baton_model_cb.clone();
        move |idx| {
            if idx >= 0 && (idx as usize) < baton_model_cb.row_count() {
                baton_model_cb.remove(idx as usize);
            }
        }
    });

    ui.on_pick_folder(move || {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            SharedString::from(path.to_string_lossy().as_ref())
        } else {
            "".into()
        }
    });

    ui.on_browse_monitor_path({
        let ui_handle = ui_handle.clone();
        move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = ui_handle.upgrade() {
                    let path_str = normalize_windows_path(&path.to_string_lossy());
                    ui.set_summons_path(path_str.into());
                }
            }
        }
    });

    ui.on_save_wards({
        let ward_model_cb = ward_model_cb;
        move || {
            let mut wards = Vec::new();
            let mut seen_paths = std::collections::HashSet::new();
            for i in 0..ward_model_cb.row_count() {
                if let Some(w) = ward_model_cb.row_data(i) {
                    if !w.path.is_empty() {
                        let normalized_path = normalize_windows_path(w.path.as_ref());
                        if !seen_paths.insert(normalized_path.clone()) {
                            continue;
                        }
                        wards.push(arbiter_core::decree::WardConfig {
                            id: normalized_path.clone(),
                            path: normalized_path.into(),
                            pattern: w.pattern.to_string(),
                            layer: match w.layer {
                                1 => arbiter_core::decree::WardLayer::Analytical,
                                _ => arbiter_core::decree::WardLayer::Surface,
                            },
                            recursive: w.recursive,
                        });
                    }
                }
            }

            let mut ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
                tracing::error!("Forge: Failed to load ledger for ward save: {}", e);
                arbiter_core::ledger::ArbiterLedger::default()
            });
            ledger.wards = wards.clone();
            if let Err(e) = arbiter_core::ledger::save(&ledger) {
                tracing::error!("Forge: Failed to save wards to disk: {}", e);
            } else {
                info!("Forge: Wards saved directly to disk");
            }

            let cmd = ForgeCommand::SaveWards(wards);
            tokio::spawn(async move {
                ipc::send_command(&cmd).await;
            });
            info!("Forge: Sent SaveWards command");
        }
    });

    ui.on_save_signet({
        let ts_path_model_cb = ts_path_model_cb;
        let baton_model_cb = baton_model_cb;
        let ui_handle = ui_handle;
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let mut trusted_paths = std::collections::HashSet::new();
                for i in 0..ts_path_model_cb.row_count() {
                    if let Some(p) = ts_path_model_cb.row_data(i) {
                        if !p.is_empty() {
                            trusted_paths.insert(p.to_string());
                        }
                    }
                }

                let mut baton_allowed = std::collections::HashSet::new();
                for i in 0..baton_model_cb.row_count() {
                    if let Some(b) = baton_model_cb.row_data(i) {
                        if !b.is_empty() {
                            baton_allowed.insert(b.to_string());
                        }
                    }
                }

                let cfg = arbiter_core::signet::ArbiterConfig {
                    trusted_paths,
                    restricted_paths: std::collections::HashSet::new(),
                    baton_allowed,
                    launch_on_startup: ui.get_launch_on_startup(),
                };

                if let Err(e) = arbiter_core::signet::save(&cfg) {
                    tracing::error!("Forge: Failed to save signet vault: {}", e);
                } else {
                    info!("Forge: Signet vault saved directly to disk");
                }

                let cmd = ForgeCommand::SaveSignet(cfg);
                tokio::spawn(async move {
                    ipc::send_command(&cmd).await;
                });
                info!("Forge: Sent SaveSignet command");
            }
        }
    });

    ui.run()?;
    Ok(())
}
