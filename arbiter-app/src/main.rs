//! main.rs — Arbiter background service entry point.
//!
//! Responsibilities:
//!   - Initialise the Tokio async runtime.
//!   - Setup structured logging (Stdout + Daily Rolling File).
//!   - Start The Atlas (Brain), The Runner (Muscle), and The Vigil (Senses).
//!   - Expose real-time IPC telemetry via Windows Named Pipes.
//!   - Host the system tray lifecycle (blocking main thread).

use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, mpsc};
use tokio_util::codec::{FramedWrite, LinesCodec};
use futures::{SinkExt, StreamExt};
use tracing::info;
use tracing_subscriber::{prelude::*, EnvFilter};

use arbiter_core::{
    atlas::Atlas,
    filter::ArbiterFilter,
    ordinance::ExecData,
    protocol::{LogEntry, ForgeCommand, PIPE_TELEMETRY, PIPE_COMMAND},
};

mod tray;

// ── Daily Rolling Writer ──────────────────────────────────────────────────────

/// A simple daily rolling writer for tracing-appender.
struct ArbiterRollingWriter {
    base_dir: std::path::PathBuf,
}

impl ArbiterRollingWriter {
    fn new(dir: &str) -> Self {
        Self { base_dir: dir.into() }
    }
}

impl std::io::Write for ArbiterRollingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let now = chrono::Local::now();
        let filename = format!("arbiter.{}.log", now.format("%Y-%m-%d"));
        let path = self.base_dir.join(filename);

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── Main Entry ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 0. Logging & Professional Banner
    let file_appender = ArbiterRollingWriter::new("arbiter-data/logs");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);
    
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false)
        .with_target(false)
        .compact();

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_target(false)
        .compact();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,arbiter=debug")))
        .with(file_layer)
        .with(stdout_layer)
        .init();

    println!(r#"
    
    █▀▀█ █▀▀█ █▀▀█ ▀█▀ ▀▀█▀▀ █▀▀ █▀▀█
    █▄▄█ █▄▄▀ █▀▀▄  █    █   █▀▀ █▄▄▀
    ▀  ▀ ▀ ▀▀ ▀▀▀▀ ▀▀▀   ▀   ▀▀▀ ▀ ▀▀
    Deterministic System Orchestration
    
    "#);
    info!("Arbiter Engine: booting version 0.1.0");

    // ── Infrastructure ────────────────────────────────────────────────────────
    let filter = ArbiterFilter::new();
    let signet_config = arbiter_core::signet::load().unwrap_or_default();
    
    let (vigil_tx, mut vigil_rx) = mpsc::channel(100);
    let (presence_tx, mut presence_rx) = mpsc::channel(100);
    let (run_event_tx, mut run_event_rx) = mpsc::channel(100);
    let (exec_cmd_tx, exec_cmd_rx) = mpsc::channel(100);
    
    let (atlas_shutdown_tx, mut atlas_shutdown_rx) = tokio::sync::oneshot::channel();
    let (atlas_exec_tx, mut atlas_exec_rx) = mpsc::channel::<ExecData>(100);
    let (reset_tx, mut reset_rx) = mpsc::channel::<()>(1);
    let (forge_cmd_tx, mut forge_cmd_rx) = mpsc::channel::<ForgeCommand>(10);

    // IPC Broadcast for Named Pipe consumers
    let (log_broadcast_tx, _) = broadcast::channel::<LogEntry>(1024);

    // ── Components ────────────────────────────────────────────────────────────

    // IPC Server (Telemetry): Named Pipe PIPE_TELEMETRY
    let ipc_broadcast = log_broadcast_tx.clone();
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ServerOptions;
        
        loop {
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(PIPE_TELEMETRY)
                .or_else(|_| ServerOptions::new().create(PIPE_TELEMETRY));
            
            if let Ok(server) = server {
                if server.connect().await.is_ok() {
                    let mut rx = ipc_broadcast.subscribe();
                    let (_, writer) = tokio::io::split(server);
                    let mut framed = FramedWrite::new(writer, LinesCodec::new());
                    while let Ok(entry) = rx.recv().await {
                        if let Ok(json) = serde_json::to_string(&entry) {
                            if framed.send(json).await.is_err() { break; }
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    // IPC Server (Commands): Named Pipe PIPE_COMMAND
    let cmd_tx = forge_cmd_tx.clone();
    tokio::spawn(async move {
        use tokio::net::windows::named_pipe::ServerOptions;
        use tokio_util::codec::FramedRead;
        loop {
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(PIPE_COMMAND)
                .or_else(|_| ServerOptions::new().create(PIPE_COMMAND));

            if let Ok(server) = server {
                if server.connect().await.is_ok() {
                    let (reader, _) = tokio::io::split(server);
                    let mut framed = FramedRead::new(reader, LinesCodec::new());
                    while let Some(res) = framed.next().await {
                        if let Ok(line) = res {
                            if let Ok(cmd) = serde_json::from_str::<ForgeCommand>(&line) {
                                let _ = cmd_tx.send(cmd).await;
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    // 1. Spawn Runner
    // ...
    info!("Arbiter Engine: standing by");
    let _ = log_broadcast_tx.send(LogEntry {
        time: chrono::Utc::now().to_rfc3339(),
        tag: "ATLAS".into(),
        message: "Arbiter Engine: system services active and standing by.".into(),
        is_error: false,
        ordinance_id: None,
    });

    // Heartbeat task
    let heartbeat_broadcast = log_broadcast_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let _ = heartbeat_broadcast.send(LogEntry {
                time: chrono::Utc::now().to_rfc3339(),
                tag: "VIGIL".into(),
                message: "Heartbeat: Watchers operational.".into(),
                is_error: false,
                ordinance_id: None,
            });
        }
    });
    let (screen_width, screen_height) = {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
        }
        #[cfg(not(windows))]
        { (1920, 1080) }
    };
    info!("Runner: mapping display boundaries to {}x{}", screen_width, screen_height);
    arbiter_bridge::runner::spawn(exec_cmd_rx, screen_width, screen_height, filter.clone());

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
                ordinance_id: exec_data.ordinance_id,
                trigger_time: exec_data.trigger_time,
                dry_run: false,
            };
            let _ = exec_cmd_tx.send(cmd).await;
        }
    });

    // 3. Spawn Watchers
    arbiter_core::presence::spawn_monitor(presence_tx, filter.clone());
    info!("Vigil: presence monitoring active");

    // 4. Initialise Atlas & Load Ledger
    let mut atlas = Atlas::new();
    let ledger = arbiter_core::ledger::load().unwrap_or_else(|e| {
        tracing::error!("Failed to load ledger: {}", e);
        arbiter_core::ledger::ArbiterLedger::default()
    });
    arbiter_core::ledger::apply(&ledger, &mut atlas, &vigil_tx, &filter);
    info!("Atlas: engine core ready");

    // 5. Spawn Atlas loop
    let atlas_broadcast = log_broadcast_tx.clone();
    let atlas_loop_broadcast = atlas_broadcast.clone();
    let atlas_vigil_tx = vigil_tx.clone();
    tokio::spawn(async move {
        atlas.run(
            &mut vigil_rx,
            atlas_vigil_tx,
            #[cfg(feature = "presence")] &mut presence_rx,
            #[cfg(not(feature = "presence"))] &mut tokio::sync::mpsc::channel(1).1,
            &mut run_event_rx,
            atlas_exec_tx.clone(),
            &mut reset_rx,
            &mut forge_cmd_rx,
            &mut atlas_shutdown_rx,
            atlas_loop_broadcast.clone(),
        ).await;
        info!("Atlas: run loop terminated cleanly");
    });

    let shutdown_cell = Arc::new(std::sync::Mutex::new(Some(atlas_shutdown_tx)));
    let reset_cell = Arc::new(std::sync::Mutex::new(reset_tx));

    // ── Tray (blocks main thread) ─────────────────────────────────────────────
    let tray_broadcast = atlas_broadcast.clone();
    tray::run_event_loop(move |event, proxy| {
        match event {
            tray::TrayAppEvent::Shutdown => {
                if let Ok(mut cell) = shutdown_cell.lock() {
                    if let Some(tx) = cell.take() { let _ = tx.send(()); }
                }
            }
            tray::TrayAppEvent::Reset => {
                if let Ok(cell) = reset_cell.lock() { let _ = cell.try_send(()); }
            }
            _ => {}
        }

        static ONCE: std::sync::Once = std::sync::Once::new();
        let proxy_atlas = proxy.clone();
        let atlas_logs = tray_broadcast.clone();
        ONCE.call_once(move || {
            let mut log_rx = atlas_logs.subscribe();
            tokio::spawn(async move {
                while let Ok(entry) = log_rx.recv().await {
                    match entry.tag.as_str() {
                        "ATLAS" => {
                            if entry.message.contains("matched") {
                                if let Some(id) = entry.ordinance_id {
                                    let _ = proxy_atlas.send_event(tray::TrayAppEvent::StatusUpdate(format!("Executing: {}", id)));
                                }
                            } else if entry.message.contains("complete") {
                                let _ = proxy_atlas.send_event(tray::TrayAppEvent::StatusUpdate("Standing By".into()));
                            }
                        }
                        "PRESN" => {
                            let _ = proxy_atlas.send_event(tray::TrayAppEvent::StatusUpdate("Yielded".into()));
                        }
                        _ => {}
                    }
                }
            });
        });
    });

    Ok(())
}
