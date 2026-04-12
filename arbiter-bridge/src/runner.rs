//! runner.rs — The Runner: background orchestration task.
//!
//! Owns The Hand, interfaces with The Inscribe and The Baton, and
//! processes instructions sequentially under a Singleton Queue Lock.

use std::{collections::HashSet, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::{hand::HardwareBridge, inscribe, shell};
use arbiter_core::{
    filter::ArbiterFilter,
    ordinance::{Action, ActionType, EnvContext, NodeKind, OrdNode, RunEvent},
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
    },
}

// ── Singleton Queue ──────────────────────────────────────────────────────────

lazy_static::lazy_static! {
    /// A global lock to ensure only one sequence can execute at a time.
    static ref QUEUE_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

// ── Interpolation ────────────────────────────────────────────────────────────

fn interpolate_str(text: &str, ctx: &EnvContext) -> String {
    // Two-pass approach:
    //   1. Find every ${env.<key>} token present in the text.
    //   2. Resolve each key through ctx.resolve(), which checks the static
    //      variables map first, then triggers lazy computation (SHA-256, MIME)
    //      if the key is a content variable and integrity_scan is true.
    //
    // This activates the OnceLock resolver chain built in ordinance.rs and
    // enforces the Signet Guard — surface-only Wards return None for deep vars,
    // leaving the token unreplaced (safe no-op).
    let mut result = text.to_string();

    // Collect unique env keys referenced in this string to avoid repeated scans.
    let mut start = 0;
    while let Some(open) = result[start..].find("${env.") {
        let open_abs = start + open;
        if let Some(close) = result[open_abs..].find('}') {
            let close_abs = open_abs + close;
            // The key sits between "${env." (6 chars) and '}'
            let key = result[open_abs + 6..close_abs].to_string();
            let token = format!("${{env.{key}}}");
            if let Some(value) = ctx.resolve(&key) {
                result = result.replacen(&token, value, 1);
                // After replacement the string may be shorter; re-scan from
                // where the replacement ended rather than after the token.
                start = open_abs + value.len();
            } else {
                // Key unknown or Signet Guard blocked it — leave token as-is
                // and advance past it so we don't loop forever.
                start = close_abs + 1;
            }
        } else {
            break; // Malformed token — stop scanning.
        }
    }
    result
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

/// Stub for non-Windows platforms — always reports 0 idle seconds.
#[cfg(not(windows))]
fn get_idle_secs() -> u64 {
    0
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

// ── Execution Task ───────────────────────────────────────────────────────────

/// Spawns the runner background task.
pub fn spawn_runner(
    mut cmd_rx: mpsc::Receiver<ExecCmd>,
    screen_width: i32,
    screen_height: i32,
    filter: ArbiterFilter,
) {
    tokio::spawn(async move {
        info!("Runner task started");

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
                    info!("Runner: acquiring queue lock");
                    let _guard = QUEUE_LOCK.lock().await;
                    info!("Runner: lock acquired, checking hibernation guard");

                    // The Hibernation Guard: Discard stale events > 5s
                    if let Some(ts_str) = context.variables.get("timestamp") {
                        if let Ok(ts) = ts_str.parse::<u64>() {
                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                            if now > ts + 5 {
                                warn!("Runner: Hibernation Guard triggered — dropping stale event (age > 5s)");
                                let _ = event_tx.send(RunEvent::Done).await;
                                continue; // bypass processing
                            }
                        }
                    }

                    // Idle Telemetry: Log how long the user has been idle before
                    // starting. Informational only — the Presence system handles
                    // active-user abortion. Wrapped in cfg(windows) internally.
                    let idle = get_idle_secs();
                    info!(idle_secs = idle, "Runner: user idle time at sequence start");


                    for (idx, node) in nodes.iter().enumerate() {
                        // Check for abort signal before every node
                        if abort_rx.try_recv().is_ok() {
                            warn!("Runner: sequence aborted by yield");
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

                                // Pacing: Handle pre-action delay asynchronously
                                if action.delay_ms > 0 {
                                    tokio::time::sleep(std::time::Duration::from_millis(action.delay_ms)).await;
                                }

                                // Async Wait: Handle ActionType::Wait without blocking
                                if let ActionType::Wait(ms) = action.action_type {
                                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                                    let _ = event_tx.send(RunEvent::Progress(idx)).await;
                                    continue;
                                }

                                let exec_result = match action.action_type {
                                    // Somatic actions
                                    ActionType::Click
                                    | ActionType::DoubleClick
                                    | ActionType::RightClick
                                    | ActionType::Type(_)
                                    | ActionType::Scroll(_)
                                    | ActionType::Navigate(_) => {
                                        filter.inhibit_presence();
                                        let res = hand.execute(&action);
                                        filter.resume_presence();
                                        res
                                    }

                                    // Inscribe actions
                                    ActionType::InscribeMove {
                                        source,
                                        destination,
                                    } => {
                                        filter.mark(&destination);
                                        let r = inscribe::move_file(
                                            source,
                                            &destination,
                                            &trusted_roots,
                                        )
                                        .map_err(|e| e.to_string());
                                        filter.unmark(&destination);
                                        r
                                    }
                                    ActionType::InscribeCopy {
                                        source,
                                        destination,
                                    } => {
                                        filter.mark(&destination);
                                        let r = inscribe::copy_file(
                                            source,
                                            &destination,
                                            &trusted_roots,
                                        )
                                        .map(|_| ())
                                        .map_err(|e| e.to_string());
                                        filter.unmark(&destination);
                                        r
                                    }
                                    ActionType::InscribeDelete { target } => {
                                        inscribe::delete_file(target, &trusted_roots)
                                            .map_err(|e| e.to_string())
                                    }

                                    // Shell actions
                                    ActionType::Shell {
                                        command,
                                        args,
                                        detached,
                                    } => {
                                        let arg_refs: Vec<&str> =
                                            args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
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
                                    ActionType::Wait(_) => unreachable!("Wait handled async above"),
                                };

                                if let Err(e) = exec_result {
                                    error!(%e, "Runner: action failed");
                                    let _ = event_tx.send(RunEvent::Panic(e)).await;
                                    break;
                                }

                                let _ = event_tx.send(RunEvent::Progress(idx)).await;
                            }
                            Err(e) => {
                                error!(%e, id = %node.id, "Runner: failed to parse JSON action");
                                let _ = event_tx
                                    .send(RunEvent::Panic(format!("Parse failure: {}", e)))
                                    .await;
                                break;
                            }
                        }
                    } // end for

                    info!("Runner: ordinance complete, releasing lock");
                    let _ = event_tx.send(RunEvent::Done).await;
                }
            }
        }

        info!("Runner task shutting down");
    });
}
