//! executor.rs — The Executor: background orchestration task.
//!
//! Owns The Hand, interfaces with The Inscribe and The Baton, and
//! processes instructions sequentially under a Singleton Queue Lock.

use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::{hand::HardwareBridge, inscribe, shell};
use vassal_core::{
    filter::VassalFilter,
    ordinance::{Action, ActionType, EnvContext, NodeKind, OrdNode, RunEvent},
};

// ── Executor Commands ────────────────────────────────────────────────────────

pub enum ExecCmd {
    /// Request to run a sequence of nodes.
    Run {
        nodes: Vec<OrdNode>,
        context: EnvContext,
        abort_rx: oneshot::Receiver<()>,
        event_tx: mpsc::Sender<RunEvent>,
        // Signet contextual data
        trusted_roots: Vec<String>,
        baton_allowed: HashSet<String>,
    },
}

// ── Singleton Queue ──────────────────────────────────────────────────────────

lazy_static::lazy_static! {
    /// A global lock to ensure only one sequence can execute at a time.
    static ref QUEUE_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

// ── Interpolation ────────────────────────────────────────────────────────────

fn interpolate_str(text: &str, ctx: &EnvContext) -> String {
    let mut result = text.to_string();
    for (k, v) in &ctx.variables {
        result = result.replace(&format!("${{env.{k}}}"), v);
    }
    result
}

fn interpolate_action(action: &mut ActionType, ctx: &EnvContext) {
    match action {
        ActionType::Type(ref mut s) | ActionType::Navigate(ref mut s) => {
            *s = interpolate_str(s, ctx);
        }
        ActionType::InscribeMove {
            source,
            destination,
        }
        | ActionType::InscribeCopy {
            source,
            destination,
        } => {
            *source = interpolate_str(source, ctx);
            *destination = interpolate_str(destination, ctx);
        }
        ActionType::InscribeDelete { target } => {
            *target = interpolate_str(target, ctx);
        }
        ActionType::Shell { command, args, .. } => {
            *command = interpolate_str(command, ctx);
            for arg in args.iter_mut() {
                *arg = interpolate_str(arg, ctx);
            }
        }
        _ => {}
    }
}

// ── Execution Task ───────────────────────────────────────────────────────────

/// Spawns the executor background task.
pub fn spawn_executor(
    mut cmd_rx: mpsc::Receiver<ExecCmd>,
    screen_width: i32,
    screen_height: i32,
    filter: VassalFilter,
) {
    tokio::spawn(async move {
        info!("Executor task started");

        // The Hand is owned locally by this task and only used while holding QUEUE_LOCK
        let mut hand = HardwareBridge::new(screen_width, screen_height);

        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                ExecCmd::Run {
                    nodes,
                    context,
                    mut abort_rx,
                    event_tx,
                    trusted_roots,
                    baton_allowed,
                } => {
                    info!("Executor: acquiring queue lock");
                    let _guard = QUEUE_LOCK.lock().await;
                    info!("Executor: lock acquired, beginning ordinance");

                    for (idx, node) in nodes.iter().enumerate() {
                        // Check for abort signal before every node
                        if abort_rx.try_recv().is_ok() {
                            warn!("Executor: sequence aborted by yield");
                            let _ = event_tx.send(RunEvent::Done).await;
                            break;
                        }

                        if node.kind != NodeKind::Action {
                            continue;
                        }

                        // Parse the internal state into an Action
                        let parsed: Result<Action, _> = serde_json::from_str(&node.internal_state);
                        match parsed {
                            Ok(mut action) => {
                                interpolate_action(&mut action.action_type, &context);

                                let exec_result = match action.action_type {
                                    // Somatic actions
                                    ActionType::Wait(_)
                                    | ActionType::Click
                                    | ActionType::DoubleClick
                                    | ActionType::RightClick
                                    | ActionType::Type(_)
                                    | ActionType::Scroll(_)
                                    | ActionType::Navigate(_) => hand.execute(&action),

                                    // Inscribe actions
                                    ActionType::InscribeMove {
                                        source,
                                        destination,
                                    } => {
                                        let copy_tgt = Path::new(&destination);
                                        filter.mark(copy_tgt);
                                        let r = inscribe::move_file(
                                            Path::new(&source),
                                            copy_tgt,
                                            &trusted_roots,
                                        )
                                        .map_err(|e| e.to_string());
                                        filter.unmark(copy_tgt);
                                        r
                                    }
                                    ActionType::InscribeCopy {
                                        source,
                                        destination,
                                    } => {
                                        let copy_tgt = Path::new(&destination);
                                        filter.mark(copy_tgt);
                                        let r = inscribe::copy_file(
                                            Path::new(&source),
                                            copy_tgt,
                                            &trusted_roots,
                                        )
                                        .map(|_| ())
                                        .map_err(|e| e.to_string());
                                        filter.unmark(copy_tgt);
                                        r
                                    }
                                    ActionType::InscribeDelete { target } => {
                                        inscribe::delete_file(Path::new(&target), &trusted_roots)
                                            .map_err(|e| e.to_string())
                                    }

                                    // Shell actions
                                    ActionType::Shell {
                                        command,
                                        args,
                                        detached,
                                    } => {
                                        let arg_refs: Vec<&str> =
                                            args.iter().map(|s| s.as_str()).collect();
                                        if detached {
                                            shell::spawn_detached(
                                                &command,
                                                &command,
                                                &arg_refs,
                                                &baton_allowed,
                                            )
                                            .map_err(|e| e.to_string())
                                        } else {
                                            shell::run(
                                                &command,
                                                &command,
                                                &arg_refs,
                                                &baton_allowed,
                                            )
                                            .map(|_| ())
                                            .map_err(|e| e.to_string())
                                        }
                                    }
                                };

                                if let Err(e) = exec_result {
                                    error!(%e, "Executor: action failed");
                                    let _ = event_tx.send(RunEvent::Panic(e)).await;
                                    break;
                                }

                                let _ = event_tx.send(RunEvent::Progress(idx)).await;
                            }
                            Err(e) => {
                                error!(%e, id = %node.id, "Executor: failed to parse JSON action");
                                let _ = event_tx
                                    .send(RunEvent::Panic(format!("Parse failure: {}", e)))
                                    .await;
                                break;
                            }
                        }
                    } // end for

                    info!("Executor: ordinance complete, releasing lock");
                    let _ = event_tx.send(RunEvent::Done).await;
                }
            }
        }

        info!("Executor task shutting down");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolation_replaces_correctly() {
        let mut ctx = EnvContext::new();
        ctx.insert("file_path", "C:\\dummy.zip");
        ctx.insert("file_name", "dummy.zip");

        let mut action = ActionType::Shell {
            command: "echo".to_string(),
            args: vec![
                "Opened: ${env.file_name}".to_string(),
                "From: ${env.file_path}".to_string(),
            ],
            detached: false,
        };

        interpolate_action(&mut action, &ctx);

        match action {
            ActionType::Shell { args, .. } => {
                assert_eq!(args[0], "Opened: dummy.zip");
                assert_eq!(args[1], "From: C:\\dummy.zip");
            }
            _ => panic!("Wrong action type after interpolation"),
        }
    }

    #[test]
    fn test_interpolation_ignores_missing_keys() {
        let ctx = EnvContext::new();

        let mut action = ActionType::Type("${env.missing_key}".to_string());
        interpolate_action(&mut action, &ctx);

        match action {
            ActionType::Type(text) => {
                assert_eq!(text, "${env.missing_key}");
            }
            _ => panic!("Wrong action type"),
        }
    }
}
