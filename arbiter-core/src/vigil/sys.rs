//! sys.rs — The Native Process Watcher.
//!
//! Exposes a polling watcher ensuring new process appearances trigger FSM summons.
//! Utilizing the `sysinfo` crate locally without full-system resource burn
//! by only filtering explicit names.

use std::collections::HashSet;
use std::time::Duration;
use sysinfo::System;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info};

use crate::decree::{EnvContext, Summons};

/// Spawns a background task watching for a specific executable name.
/// e.g. `"photoshop.exe"` or `"Notepad"`. Case-insensitive.
///
/// Returns a `broadcast::Sender<()>` that can be used as a shutdown signal,
/// matching the same pattern used by `vigil::fs::spawn_watcher`.
/// Drop or send on the returned sender to stop the watcher task.
pub fn spawn_watcher(
    target_process_name: String,
    tx: mpsc::Sender<Summons>,
) -> broadcast::Sender<()> {
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    tokio::spawn(async move {
        // We only instantiate a lightweight local system map.
        let mut sys = System::new();
        let target_lower = target_process_name.to_lowercase();

        info!(target = %target_process_name, "Vigil: Process watcher active");

        // Keep track of PIDs we have already announced to avoid spamming the
        // channel every second. When a process exits and re-launches, its new
        // PID will be absent from `known_pids` and fire a fresh summons.
        let mut known_pids = HashSet::new();

        loop {
            // Check for shutdown signal before polling.
            if shutdown_rx.try_recv().is_ok() {
                info!(target = %target_process_name, "Vigil: Process watcher stopping");
                break;
            }

            // refresh_processes() without loading CPU/Memory data is incredibly fast.
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);

            let mut current_pids = HashSet::new();

            for (pid, process) in sys.processes() {
                let p_name = process.name().to_string_lossy().to_lowercase();

                if p_name.contains(&target_lower) {
                    current_pids.insert(*pid);

                    if !known_pids.contains(pid) {
                        debug!(%pid, %p_name, "Vigil: Target process discovered");

                        let mut context = EnvContext::new();
                        context.insert("process_name", &process.name().to_string_lossy());
                        context.insert("process_pid", &pid.to_string());
                        context.insert("trigger_mode", "ProcessAppeared");
                        context.insert("timestamp", &format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
                        context.insert("timestamp_local", &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string());

                        let summons = Summons::ProcessAppeared {
                            name: target_process_name.clone(),
                            context,
                        };

                        if tx.send(summons).await.is_err() {
                            // Receiver dropped — terminate the watcher.
                            return;
                        }
                    }
                }
            }

            // Sync known PIDs so if it closes we forget it, allowing it to
            // trigger again on re-launch.
            known_pids = current_pids;

            // Poll every 2 seconds. We don't need hyper-aggression for process
            // launch events.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    shutdown_tx
}
