//! runner.rs — The Runner: background orchestration task.
//!
//! Owns The Hand, interfaces with The Inscribe and The Baton, and
//! processes instructions sequentially under a Singleton Queue Lock.

use std::{collections::HashSet, sync::Arc};
use regex::Regex;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::{hand::HardwareBridge, inscribe, shell};
use arbiter_core::{
    filter::ArbiterFilter,
    ordinance::{Action, ActionType, EnvContext, NodeKind, OrdNode, RunEvent, DecreeId},
    protocol::LogEntry,
};

// ── Runner Commands ────────────────────────────────────────────────────────

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
        ordinance_id: Option<DecreeId>,
        trigger_time: std::time::Instant,
        dry_run: bool,
    },
}

// ── Singleton Queue ──────────────────────────────────────────────────────────

lazy_static::lazy_static! {
    /// A global lock to ensure only one sequence can execute at a time.
    static ref QUEUE_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

// ── Interpolation ────────────────────────────────────────────────────────────

lazy_static::lazy_static! {
    /// Regex to match ${env.KEY} patterns.
    static ref ENV_RE: Regex = Regex::new(r"\$\{env\.([^}]+)\}").unwrap();
}

fn interpolate_str(text: &str, ctx: &EnvContext) -> String {
    // Replaces all occurrences of ${env.key} with values from the context.
    // If a key is unknown or the Signet Guard blocks it, the token is left as-is.
    ENV_RE.replace_all(text, |caps: &regex::Captures| {
        let key = &caps[1];
        if let Some(value) = ctx.resolve(key) {
            value.to_string()
        } else {
            caps[0].to_string()
        }
    }).into_owned()
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
            let src_str = interpolate_str(&source.to_string_lossy(), ctx);
            let dst_str = interpolate_str(&destination.to_string_lossy(), ctx);
            *source = src_str.into();
            *destination = dst_str.into();
        }
        ActionType::InscribeDelete { target } => {
            let tgt_str = interpolate_str(&target.to_string_lossy(), ctx);
            *target = tgt_str.into();
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

// ── Windows Idle Detection ───────────────────────────────────────────────────

/// Returns the number of seconds since the user last touched a keyboard or mouse.
///
/// Uses `GetLastInputInfo` + `GetTickCount` from the Win32 API.
/// Logged by the Runner before executing a sequence as an observability data point.
#[cfg(windows)]
fn get_idle_secs() -> u64 {
    use windows::Win32::{
        System::SystemInformation::GetTickCount,
        UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
    };
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if GetLastInputInfo(&mut lii).as_bool() {
            let now = GetTickCount();
            // wrapping_sub handles the u32 DWORD tick counter rollover (~49 days).
            (now.wrapping_sub(lii.dwTime) / 1000) as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn get_idle_secs() -> u64 {
    0
}

// ── Runner Task ──────────────────────────────────────────────────────────────

/// Spawn the long-running background Runner task.
///
/// This task owns `The Hand` and processes `ExecCmd` requests one at a time.
pub fn spawn(
    mut rx: mpsc::Receiver<ExecCmd>,
    screen_width: i32,
    screen_height: i32,
    filter: ArbiterFilter,
) {
    tokio::spawn(async move {
        info!("Runner task started");

        // The Hand is owned locally by this task and only used while holding QUEUE_LOCK
        let mut hand = HardwareBridge::new(screen_width, screen_height);

        while let Some(cmd) = rx.recv().await {
            let ExecCmd::Run {
                nodes,
                context,
                mut abort_rx,
                event_tx,
                trusted_roots,
                baton_allowed,
                ordinance_id,
                trigger_time,
                dry_run,
            } = cmd;

            info!("Runner: acquiring queue lock");
            let _guard = QUEUE_LOCK.lock().await;
            info!("Runner: lock acquired, checking hibernation guard");

            // The Hibernation Guard: Discard stale events > 5s
            if trigger_time.elapsed().as_secs() > 5 {
                warn!("Runner: Hibernation Guard triggered — dropping stale event (age > 5s)");
                let _ = event_tx.send(RunEvent::Done).await;
                continue; // bypass processing
            }

            // Idle Telemetry: Log how long the user has been idle before
            // starting. Informational only.
            let idle = get_idle_secs();
            info!(idle_secs = idle, "Runner: user idle time at sequence start");

            let _ = event_tx.send(RunEvent::Log(LogEntry {
                time: chrono::Utc::now().to_rfc3339(),
                tag: "HAND".into(),
                message: format!("Macro iteration started (Last User Input: {}s ago){}", idle, if dry_run { " [DRY RUN]" } else { "" }),
                is_error: false,
                ordinance_id: ordinance_id.as_ref().map(|id| id.0.clone()),
            })).await;

            let mut current_idx = 0;
            let total = nodes.len();

            for (idx, node) in nodes.iter().enumerate() {
                current_idx = idx;
                // Check for abort signal before every node
                if abort_rx.try_recv().is_ok() {
                    warn!("Runner: sequence aborted by yield");
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

                        // Pacing: Handle pre-action delay asynchronously
                        if action.delay_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(action.delay_ms)).await;
                        }

                        // Async Wait: Handle ActionType::Wait without blocking
                        if let ActionType::Wait(ms) = action.action_type {
                            if !dry_run {
                                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                            } else {
                                info!(ms, "DRY RUN: Would wait");
                            }
                            let _ = event_tx.send(RunEvent::Progress(idx)).await;
                            continue;
                        }

                        let exec_result = match &action.action_type {
                            // Somatic actions
                            ActionType::Click
                            | ActionType::DoubleClick
                            | ActionType::RightClick
                            | ActionType::Type(_)
                            | ActionType::Scroll(_)
                            | ActionType::Navigate(_) => {
                                if !dry_run {
                                    filter.inhibit_presence();
                                    let res = hand.execute(&action);
                                    filter.resume_presence();
                                    res
                                } else {
                                    info!(action = ?action.action_type, "DRY RUN: Would execute somatic action");
                                    Ok(())
                                }
                            }

                            // Inscribe actions
                            ActionType::InscribeMove {
                                source,
                                destination,
                            } => {
                                if !dry_run {
                                    filter.mark(destination);
                                    let r = inscribe::move_file(
                                        source,
                                        destination,
                                        &trusted_roots,
                                    )
                                    .map_err(|e| e.to_string());
                                    filter.unmark(destination);
                                    r
                                } else {
                                    info!(?source, ?destination, "DRY RUN: Would move file");
                                    Ok(())
                                }
                            }
                            ActionType::InscribeCopy {
                                source,
                                destination,
                            } => {
                                if !dry_run {
                                    filter.mark(destination);
                                    let r = inscribe::copy_file(
                                        source,
                                        destination,
                                        &trusted_roots,
                                    )
                                    .map(|_| ())
                                    .map_err(|e| e.to_string());
                                    filter.unmark(destination);
                                    r
                                } else {
                                    info!(?source, ?destination, "DRY RUN: Would copy file");
                                    Ok(())
                                }
                            }
                            ActionType::InscribeDelete { target } => {
                                if !dry_run {
                                    inscribe::delete_file(target, &trusted_roots)
                                        .map_err(|e| e.to_string())
                                } else {
                                    info!(?target, "DRY RUN: Would delete file");
                                    Ok(())
                                }
                            }

                            // Shell actions
                            ActionType::Shell {
                                command,
                                args,
                                detached,
                            } => {
                                if !dry_run {
                                    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                                    if *detached {
                                        shell::spawn_detached(command, command, &arg_refs, &baton_allowed)
                                            .map_err(|e| e.to_string())
                                    } else {
                                        shell::run(command, command, &arg_refs, &baton_allowed)
                                            .map(|_| ())
                                            .map_err(|e| e.to_string())
                                    }
                                } else {
                                    info!(%command, ?args, detached = *detached, "DRY RUN: Would execute shell command");
                                    Ok(())
                                }
                            }
                            _ => Ok(()),
                        };

                        if let Err(e) = exec_result {
                            error!(%e, id = %node.id, "Runner: action failed");
                            let _ = event_tx.send(RunEvent::Panic(format!("Step '{}' failed: {}", node.label, e))).await;
                            break;
                        }
                    }
                    Err(e) => {
                        error!(%e, id = %node.id, "Runner: failed to parse JSON action");
                        let _ = event_tx.send(RunEvent::Log(LogEntry {
                            time: chrono::Utc::now().to_rfc3339(),
                            tag: "HAND".into(),
                            message: format!("Corrupt Action data in step '{}'", node.label),
                            is_error: true,
                            ordinance_id: ordinance_id.as_ref().map(|id| id.0.clone()),
                        })).await;
                        let _ = event_tx.send(RunEvent::Panic("Engine halt: Malformed ordinance data".into())).await;
                        break;
                    }
                }

                let _ = event_tx.send(RunEvent::Progress(idx)).await;
            }

            if current_idx == total - 1 || total == 0 {
                let _ = event_tx.send(RunEvent::Done).await;
            }
        }
    });
}
