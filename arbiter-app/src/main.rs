//! main.rs — Arbiter Engine entry point.
//!
//! The binary is a silent background service. On launch it:
//!   1. Initialises structured logging (tracing).
//!   2. Starts the tokio runtime on background threads.
//!   3. Boots The Atlas, The Runner, and opens watcher channels.
//!   4. Parks the main thread inside the tray event loop (OS requirement).
//!
//! No splash screen. No console window in release builds.
//! The only user-facing presence is the system tray icon.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use arbiter_core::atlas::Atlas;
use arbiter_core::ordinance::{NodeKind, OrdNode};
use serde_json;

fn banner(title: impl std::fmt::Display, subtitle: impl std::fmt::Display) {
    let width = 56;

    println!("╔{}╗", "═".repeat(width - 2));
    println!("│{:^54}│", title);
    println!("│{:^54}│", subtitle);
    println!("╚{}╝", "═".repeat(width - 2));
}

fn main() {
    // ── Logging ───────────────────────────────────────────────────────────────
    // Optional local terminal output for development
    let stdout_log = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_thread_names(true)
        .compact();

    // Persistent file log for the UI (arbiter-forge) to tail
    let file_appender = tracing_appender::rolling::never("doc/logs", "arbiter.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let file_log = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false)
        .compact();

    use tracing_subscriber::layer::SubscriberExt;
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("arbiter=info,warn")))
        .with(stdout_log)
        .with(file_log);
        
    tracing::subscriber::set_global_default(subscriber).expect("Unable to set global tracing subscriber");

    banner(
        format!("ARBITER v{}", env!("CARGO_PKG_VERSION")),
        "Command & Control Orchestration Engine",
    );

    info!("Status: Initialising mechanical bridges...");

    // ── Tokio Runtime ─────────────────────────────────────────────────────────
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("arbiter-worker")
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    // ── Engine Boot ───────────────────────────────────────────────────────────
    let _guard = rt.enter();

    // Load Signet vault
    let signet_config = arbiter_core::signet::load().unwrap_or_default();
    info!("Signet: secure configuration loaded");

    // The Filter
    let filter = arbiter_core::filter::ArbiterFilter::new();

    // Channels
    let (vigil_tx, mut vigil_rx) = arbiter_core::vigil::channel(64);
    let (presence_tx, mut presence_rx) = tokio::sync::mpsc::channel(8);
    let (atlas_exec_tx, mut atlas_exec_rx) =
        tokio::sync::mpsc::channel::<arbiter_core::ordinance::ExecData>(16);
    let (exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::channel(16);
    let (run_event_tx, mut run_event_rx) = tokio::sync::mpsc::channel(32);
    let (atlas_shutdown_tx, mut atlas_shutdown_rx) = tokio::sync::oneshot::channel();
    let (reset_tx, mut reset_rx) = tokio::sync::mpsc::channel(8);

    // IPC: Named Pipe Server for real-time telemetry
    let (log_broadcast_tx, _) = tokio::sync::broadcast::channel::<arbiter_core::ordinance::LogEntry>(1024);
    let ipc_broadcast = log_broadcast_tx.clone();
    
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ServerOptions;
        use tokio_util::codec::{FramedWrite, LinesCodec};
        use futures::SinkExt;

        let pipe_name = r"\\.\pipe\arbiter_telemetry";
        
        loop {
            let server = match ServerOptions::new()
                .first_pipe_instance(true)
                .create(pipe_name) 
            {
                Ok(s) => s,
                Err(e) => {
                    error!(%e, "IPC: telemetry pipe creation failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            // Wait for a client to connect
            if server.connect().await.is_ok() {
                let mut rx = ipc_broadcast.subscribe();
                let (_, writer) = tokio::io::split(server);
                let mut framed = FramedWrite::new(writer, LinesCodec::new());

                tokio::spawn(async move {
                    while let Ok(entry) = rx.recv().await {
                        if let Ok(json) = serde_json::to_string(&entry) {
                            if framed.send(json).await.is_err() {
                                break; // Client disconnected
                            }
                        }
                    }
                });
            }

            // Create subsequent instances for more clients
            loop {
                let server = match ServerOptions::new().create(pipe_name) {
                    Ok(s) => s,
                    Err(_) => break,
                };
                if server.connect().await.is_ok() {
                    let mut rx = ipc_broadcast.subscribe();
                    let (_, writer) = tokio::io::split(server);
                    let mut framed = FramedWrite::new(writer, LinesCodec::new());
                    tokio::spawn(async move {
                        while let Ok(entry) = rx.recv().await {
                            if let Ok(json) = serde_json::to_string(&entry) {
                                if framed.send(json).await.is_err() {
                                    break;
                                }
                            }
                        }
                    });
                }
            }
        }
    });

    // 1. Spawn Runner
    let (screen_width, screen_height) = {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
            unsafe {
                (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
            }
        }
        #[cfg(not(windows))]
        {
            (1920, 1080) // Fallback for other OS or headless
        }
    };
    info!("Runner: display boundaries mapped to {}x{}", screen_width, screen_height);

    arbiter_bridge::runner::spawn_runner(
        exec_cmd_rx,
        screen_width,
        screen_height,
        filter.clone(),
    );

    // 2. Spawn Mapping loop (Atlas -> Runner)
    let map_run_event_tx = run_event_tx.clone();
    let map_trusted = signet_config.trusted_paths.clone();
    let map_baton = signet_config.baton_allowed.clone();
    tokio::spawn(async move {
        while let Some(exec_data) = atlas_exec_rx.recv().await {
            let cmd = arbiter_bridge::runner::ExecCmd::Run {
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
    let _ = arbiter_core::vigil::keys::register_hotkey("Ctrl+Shift+D".into(), vigil_tx.clone());

    // Just an example to prove compilation and logic — assumes Downloads exist.
    if let Some(downloads) = dirs::download_dir() {
        arbiter_core::vigil::fs::spawn_watcher(
            downloads.to_string_lossy().to_string(),
            "*.zip".into(),
            filter.clone(),
            vigil_tx.clone(),
        );
    }

    arbiter_core::presence::spawn_monitor(presence_tx, filter.clone());
    info!("Vigil: presence monitoring active");

    // 4. Initialise & Configure Atlas
    let mut atlas = Atlas::new();
    atlas.presence_config.ignore_mouse = true; // Refined: Mouse move won't abort, only keys
    info!("Atlas: engine core ready (Sensitivity: Keyboard Only)");

    // Smoke Test 1: The Macro
    let macro_nodes = vec![
        OrdNode {
            id: "1".into(),
            label: "Start".into(),
            kind: NodeKind::Entry,
            internal_state: "".into(),
            next_nodes: [("Next".into(), "7".into())].into(),
        },
        OrdNode {
            id: "7".into(),
            label: "Presence Buffer".into(),
            kind: NodeKind::Action,
            internal_state: r#"{"action_type":{"Wait":1500},"point":null,"delay_ms":0}"#.into(),
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
            internal_state: r#"{"action_type":{"Wait":500},"point":null,"delay_ms":0}"#.into(),
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
            internal_state: r#"{"action_type":{"Wait":800},"point":null,"delay_ms":0}"#.into(),
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
    if let Some(archive_dir) = dirs::document_dir().map(|d| d.join("Arbiter_Archives")) {
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
    let atlas_broadcast = log_broadcast_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = atlas.run(
                    &mut vigil_rx,
                    #[cfg(feature = "presence")]
                    &mut presence_rx,
                    #[cfg(not(feature = "presence"))]
                    &mut tokio::sync::mpsc::channel(1).1,
                    &mut run_event_rx,
                    atlas_exec_tx.clone(),
                    &mut atlas_shutdown_rx,
                    atlas_broadcast.clone(),
                ) => {
                    info!("Atlas loop terminated cleanly");
                    break;
                }
                _ = reset_rx.recv() => {
                    if atlas.state == arbiter_core::atlas::EngineState::Faulted {
                        info!("Atlas: reset signal received, clearing Faulted state");
                        atlas.state = arbiter_core::atlas::EngineState::Idle;
                        let _ = atlas_broadcast.send(arbiter_core::ordinance::LogEntry {
                            tag: "ATLAS".into(),
                            message: "Engine fault cleared manually.".into(),
                            is_error: false,
                        });
                    }
                }
            }
        }
    });

    // Store the shutdown transmitter globally so the tray can signal it
    // We cheat a bit by keeping an Option in a Mutex, since run_event_loop takes FnOnce
    use std::sync::{Arc, Mutex};
    let shutdown_cell = Arc::new(Mutex::new(Some(atlas_shutdown_tx)));
    let reset_cell = Arc::new(Mutex::new(reset_tx));

    // ── Tray (blocks main thread) ─────────────────────────────────────────────
    tray::run_event_loop(move |event| {
        match event {
            tray::TrayAppEvent::Shutdown => {
                info!("Arbiter shutting down — the servant is dismissed.");
                if let Ok(mut cell) = shutdown_cell.lock() {
                    if let Some(tx) = cell.take() {
                        let _ = tx.send(());
                    }
                }
            }
            tray::TrayAppEvent::Reset => {
                if let Ok(cell) = reset_cell.lock() {
                    let _ = cell.try_send(());
                }
            }
            _ => {}
        }
    });
}
