//! main.rs — Vassal Engine entry point.
//!
//! The binary is a silent background service. On launch it:
//!   1. Initialises structured logging (tracing).
//!   2. Starts the tokio runtime on background threads.
//!   3. Boots The Atlas, The Executor, and opens watcher channels.
//!   4. Parks the main thread inside the tray event loop (OS requirement).
//!
//! No splash screen. No console window in release builds.
//! The only user-facing presence is the system tray icon.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use vassal_core::atlas::Atlas;
use vassal_core::ordinance::{NodeKind, OrdNode};

fn main() {
    // ── Logging ───────────────────────────────────────────────────────────────
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("vassal=info,warn")),
        )
        .with_target(false)
        .with_thread_names(true)
        .compact()
        .init();

    info!("╔═══════════════════════════════╗");
    info!("║   V A S S A L  v{}       ║", env!("CARGO_PKG_VERSION"));
    info!("╚═══════════════════════════════╝");
    info!("The duty is performed.");

    // ── Tokio Runtime ─────────────────────────────────────────────────────────
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("vassal-worker")
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    // ── Engine Boot ───────────────────────────────────────────────────────────
    let _guard = rt.enter();

    // Load Signet vault
    let signet_config = vassal_core::signet::load().unwrap_or_default();
    info!("Signet config loaded");

    // The Filter
    let filter = vassal_core::filter::VassalFilter::new();

    // Channels
    let (vigil_tx, vigil_rx) = vassal_core::vigil::channel(64);
    let (presence_tx, presence_rx) = tokio::sync::mpsc::channel(8);
    let (atlas_exec_tx, mut atlas_exec_rx) =
        tokio::sync::mpsc::channel::<vassal_core::ordinance::ExecData>(16);
    let (exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::channel(16);
    let (run_event_tx, run_event_rx) = tokio::sync::mpsc::channel(32);
    let (atlas_shutdown_tx, atlas_shutdown_rx) = tokio::sync::oneshot::channel();

    // 1. Spawn Executor
    vassal_bridge::executor::spawn_executor(
        exec_cmd_rx,
        1920,
        1080, // Hardcoded display resolution for Phase 2
        filter.clone(),
    );

    // 2. Spawn Mapping loop (Atlas -> Executor)
    let map_run_event_tx = run_event_tx.clone();
    let map_trusted = signet_config.trusted_paths.clone();
    let map_baton = signet_config.baton_allowed.clone();
    tokio::spawn(async move {
        while let Some(exec_data) = atlas_exec_rx.recv().await {
            let cmd = vassal_bridge::executor::ExecCmd::Run {
                nodes: exec_data.nodes,
                context: exec_data.context,
                abort_rx: exec_data.abort_rx,
                event_tx: map_run_event_tx.clone(),
                trusted_roots: map_trusted.iter().cloned().collect(),
                baton_allowed: map_baton.clone(),
            };
            let _ = exec_cmd_tx.send(cmd).await;
        }
    });

    // 3. Spawn Watchers
    let _ = vassal_core::vigil::keys::register_hotkey("Ctrl+Shift+D".into(), vigil_tx.clone());

    // Just an example to prove compilation and logic — assumes Downloads exist.
    if let Some(downloads) = dirs::download_dir() {
        vassal_core::vigil::fs::spawn_watcher(
            downloads.to_string_lossy().to_string(),
            "*.zip".into(),
            filter.clone(),
            vigil_tx.clone(),
        );
    }

    vassal_core::presence::spawn_monitor(presence_tx);
    info!("Presence monitor active");

    // 4. Initialise & Configure Atlas
    let mut atlas = Atlas::new();

    // Smoke Test 1: The Macro
    let macro_nodes = vec![
        OrdNode {
            id: "1".into(),
            label: "Start".into(),
            kind: NodeKind::Entry,
            internal_state: "".into(),
            next_nodes: [("Next".into(), "2".into())].into(),
        },
        OrdNode {
            id: "2".into(),
            label: "Open Start".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Navigate":"super"},"point":null,"delay_ms":0}"#
                .into(),
            next_nodes: [("Next".into(), "3".into())].into(),
        },
        OrdNode {
            id: "3".into(),
            label: "Wait".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Wait":300},"point":null,"delay_ms":0}"#.into(),
            next_nodes: [("Next".into(), "4".into())].into(),
        },
        OrdNode {
            id: "4".into(),
            label: "Type Discord".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Type":"discord"},"point":null,"delay_ms":0}"#
                .into(),
            next_nodes: [("Next".into(), "5".into())].into(),
        },
        OrdNode {
            id: "5".into(),
            label: "Wait".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Wait":200},"point":null,"delay_ms":0}"#.into(),
            next_nodes: [("Next".into(), "6".into())].into(),
        },
        OrdNode {
            id: "6".into(),
            label: "Enter".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Navigate":"return"},"point":null,"delay_ms":0}"#
                .into(),
            next_nodes: [].into(),
        },
    ];
    atlas.register_ordinance("Hotkey|Ctrl+Shift+D".into(), macro_nodes);

    // Smoke Test 2: The Organiser
    if let Some(archive_dir) = dirs::document_dir().map(|d| d.join("Vassal_Archives")) {
        let archive_path = archive_dir.to_string_lossy().to_string();
        // A bit hacky escaping for the JSON, but sufficient for the smoke test
        let internal_json = format!(
            r#"{{"action_type":{{"InscribeMove":{{"source":"${{env.file_path}}","destination":"{}\\\\${{env.file_name}}"}}}}}},"point":null,"delay_ms":0}}"#,
            archive_path.replace("\\", "\\\\")
        );

        let fs_nodes = vec![
            OrdNode {
                id: "1".into(),
                label: "Start".into(),
                kind: NodeKind::Entry,
                internal_state: "".into(),
                next_nodes: [("Next".into(), "2".into())].into(),
            },
            OrdNode {
                id: "2".into(),
                label: "Move File".into(),
                kind: NodeKind::Action,
                internal_state: internal_json,
                next_nodes: [].into(),
            },
        ];

        if let Some(downloads) = dirs::download_dir() {
            let fs_key = format!("FileCreated|{}|{}", downloads.to_string_lossy(), "*.zip");
            atlas.register_ordinance(fs_key, fs_nodes);
        }
    }

    // 5. Spawn Atlas loop
    tokio::spawn(async move {
        atlas
            .run(
                vigil_rx,
                presence_rx,
                run_event_rx,
                atlas_exec_tx,
                atlas_shutdown_rx,
            )
            .await;
        info!("Atlas loop terminated cleanly");
    });

    // Store the shutdown transmitter globally so the tray can signal it
    // We cheat a bit by keeping an Option in a Mutex, since run_event_loop takes FnOnce
    use std::sync::{Arc, Mutex};
    let shutdown_cell = Arc::new(Mutex::new(Some(atlas_shutdown_tx)));

    // ── Tray (blocks main thread) ─────────────────────────────────────────────
    tray::run_event_loop(move || {
        info!("Vassal shutting down — the servant is dismissed.");
        if let Ok(mut cell) = shutdown_cell.lock() {
            if let Some(tx) = cell.take() {
                let _ = tx.send(());
            }
        }
        rt.shutdown_background();
    });
}
