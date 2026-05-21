use std::collections::HashSet;
use std::time::Duration;
use sysinfo::System;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info};

use crate::decree::{EnvContext, Summons};

pub fn spawn_watcher(
    target_process_name: String,
    tx: mpsc::Sender<Summons>,
) -> broadcast::Sender<()> {
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    tokio::spawn(async move {
        let mut sys = System::new();
        let target_lower = target_process_name.to_lowercase();

        info!(target = %target_process_name, "Vigil: Process watcher active");

        let mut known_pids = HashSet::new();

        loop {
            if shutdown_rx.try_recv().is_ok() {
                info!(target = %target_process_name, "Vigil: Process watcher stopping");
                break;
            }

            sys.refresh_processes();

            let mut current_pids = HashSet::new();

            for (pid, process) in sys.processes() {
                let p_name = process.name().to_string().to_lowercase();

                if p_name.contains(&target_lower) {
                    current_pids.insert(*pid);

                    if !known_pids.contains(pid) {
                        debug!(%pid, %p_name, "Vigil: Target process discovered");

                        let mut context = EnvContext::new();
                        context.insert("process_name", process.name());
                        context.insert("process_pid", &pid.to_string());
                        context.insert("trigger_mode", "ProcessAppeared");
                        context.insert(
                            "timestamp",
                            &format!(
                                "{secs}",
                                secs = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            ),
                        );
                        context.insert(
                            "timestamp_local",
                            &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string(),
                        );

                        let summons = Summons::ProcessAppeared {
                            name: target_process_name.clone(),
                            context,
                        };

                        if tx.send(summons).await.is_err() {
                            return;
                        }
                    }
                }
            }

            known_pids = current_pids;

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    shutdown_tx
}
