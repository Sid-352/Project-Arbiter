//! main.rs — Vassal Engine entry point.
//!
//! The binary is a silent background service. On launch it:
//!   1. Initialises structured logging (tracing).
//!   2. Starts the tokio runtime on background threads.
//!   3. Boots The Atlas and opens the Vigil + Presence channels.
//!   4. Parks the main thread inside the tray event loop (OS requirement).
//!
//! No splash screen. No console window in release builds.
//! The only user-facing presence is the system tray icon.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use vassal_core::atlas::Atlas;

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
    // Feature-gated modules (vigil, presence) are conditionally compiled via
    // vassal-core's feature flags. The cfg checks live in vassal-core/src/lib.rs.
    rt.block_on(async {
        let _atlas = Atlas::new();
        info!("Atlas initialised — engine standing by");

        // The Vigil — file and hotkey watchers
        let (_vigil_tx, _vigil_rx) = vassal_core::vigil::channel(64);
        info!("Vigil channel open — awaiting Summons");
        // TODO: wire vigil_rx → atlas loop

        // Presence — human input detection
        let (presence_tx, _presence_rx) = tokio::sync::mpsc::channel(8);
        vassal_core::presence::spawn_monitor(presence_tx);
        info!("Presence monitor active");
        // TODO: wire presence_rx → atlas.yield_to_presence()
    });

    // ── Tray (blocks main thread) ─────────────────────────────────────────────
    // FnOnce — the runtime is consumed exactly once on shutdown.
    // run_event_loop only returns when Quit is selected.
    tray::run_event_loop(move || {
        info!("Vassal shutting down — the servant is dismissed.");
        rt.shutdown_background();
    });
}
