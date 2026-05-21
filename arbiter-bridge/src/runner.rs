use regex::Regex;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info, warn};

use crate::{hand::HardwareBridge, inscribe, shell};
use arbiter_core::{
    decree::{Action, ActionType, DecreeId, DecreeNode, EnvContext, NodeKind, NodeState, RunEvent},
    filter::ArbiterFilter,
    protocol::LogEntry,
};

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("Hand interaction failed: {0}")]
    HandError(String),
    #[error(transparent)]
    InscribeError(#[from] crate::inscribe::InscribeError),
    #[error(transparent)]
    ShellError(#[from] crate::shell::ShellError),
}

pub enum ExecCmd {
    Run {
        nodes: Vec<DecreeNode>,
        context: EnvContext,
        abort_rx: oneshot::Receiver<()>,
        event_tx: mpsc::Sender<RunEvent>,
        // Signet contextual data
        trusted_roots: Vec<String>,
        baton_allowed: HashSet<String>,
        decree_id: Option<DecreeId>,
        trigger_time: std::time::Instant,
        dry_run: bool,
    },
}

lazy_static::lazy_static! {
    static ref QUEUE_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

lazy_static::lazy_static! {
    static ref ENV_RE: Regex = Regex::new(r"\$\{env\.([^}]+)\}").unwrap();
}

fn interpolate_str(text: &str, ctx: &EnvContext, sanitize: bool) -> String {
    ENV_RE
        .replace_all(text, |caps: &regex::Captures| {
            let key = &caps[1];
            if let Some(value) = ctx.resolve(key) {
                if sanitize {
                    sanitize_shell_arg(value)
                } else {
                    value.to_string()
                }
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

fn sanitize_shell_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' | '|' | ';' | '<' | '>' | '^' | '%' | '!' | '"' => {
                out.push(' '); // Replace with space or escape? Space is safer for simple names.
            }
            _ => out.push(c),
        }
    }
    out
}

fn interpolate_action(action: &mut ActionType, ctx: &EnvContext) {
    match action {
        ActionType::Type(ref mut s) | ActionType::Navigate(ref mut s) => {
            *s = interpolate_str(s, ctx, false);
        }
        ActionType::InscribeMove {
            source,
            destination,
        }
        | ActionType::InscribeCopy {
            source,
            destination,
        } => {
            let src_str = interpolate_str(&source.to_string_lossy(), ctx, false);
            let dst_str = interpolate_str(&destination.to_string_lossy(), ctx, false);
            *source = src_str.into();
            *destination = dst_str.into();
        }
        ActionType::InscribeDelete { target } => {
            let tgt_str = interpolate_str(&target.to_string_lossy(), ctx, false);
            *target = tgt_str.into();
        }
        ActionType::Shell { command, args, .. } => {
            *command = interpolate_str(command, ctx, false);
            for arg in args.iter_mut() {
                *arg = interpolate_str(arg, ctx, true);
            }
        }
        ActionType::Click
        | ActionType::DoubleClick
        | ActionType::RightClick
        | ActionType::Scroll(_)
        | ActionType::Wait(_) => {}
    }
}

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
        if GetLastInputInfo(&raw mut lii).as_bool() {
            let now = GetTickCount();
            // wrapping_sub handles the u32 DWORD tick counter rollover (~49 days).
            u64::from(now.wrapping_sub(lii.dwTime) / 1000)
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn get_idle_secs() -> u64 {
    0
}

pub fn spawn(
    mut rx: mpsc::Receiver<ExecCmd>,
    screen_width: i32,
    screen_height: i32,
    filter: ArbiterFilter,
) {
    tokio::spawn(async move {
        info!("Runner task started");

        let mut hand = HardwareBridge::new(screen_width, screen_height);

        while let Some(cmd) = rx.recv().await {
            let ExecCmd::Run {
                nodes,
                context,
                abort_rx,
                event_tx,
                trusted_roots,
                baton_allowed,
                decree_id,
                trigger_time,
                dry_run,
            } = cmd;

            info!("Runner: acquiring queue lock");
            let _guard = QUEUE_LOCK.lock().await;
            info!("Runner: lock acquired, checking hibernation guard");

            if trigger_time.elapsed().as_secs() > 5 {
                warn!("Runner: Hibernation Guard triggered — dropping stale event (age > 5s)");
                let _ = event_tx.send(RunEvent::Done).await;
                continue;
            }

            let idle = get_idle_secs();
            info!(idle_secs = idle, "Runner: user idle time at sequence start");

            let _ = event_tx
                .send(RunEvent::Log(LogEntry {
                    time: chrono::Utc::now().to_rfc3339(),
                    tag: "HAND".into(),
                    message: if dry_run {
                        format!("[DRY-RUN] Macro iteration started (Last User Input: {idle}s ago)")
                    } else {
                        format!("Macro iteration started (Last User Input: {idle}s ago)")
                    },
                    is_error: false,
                    decree_id: decree_id.as_ref().map(|id| id.0.clone()),
                }))
                .await;

            let mut abort_rx = abort_rx; // make mutable to use in loop

            for (idx, node) in nodes.iter().enumerate() {
                if abort_rx.try_recv().is_ok() {
                    warn!("Runner: sequence aborted by yield");
                    break;
                }

                if node.kind() != NodeKind::Action {
                    continue;
                }

                let mut action = if let NodeState::Action {
                    action_type,
                    point,
                    delay_ms,
                } = &node.state
                {
                    Action {
                        action_type: action_type.clone(),
                        point: point.clone(),
                        delay_ms: *delay_ms,
                    }
                } else {
                    error!(%node.id, "Runner: Expected Action state, found something else");
                    let _ = event_tx
                        .send(RunEvent::Panic("Engine halt: Malformed decree data".into()))
                        .await;
                    break;
                };

                interpolate_action(&mut action.action_type, &context);

                if action.delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(action.delay_ms)).await;
                }

                if let ActionType::Wait(ms) = action.action_type {
                    if dry_run {
                        info!("[DRY-RUN] Would wait for {ms} ms (execution bypassed)");
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    }
                    let _ = event_tx.send(RunEvent::Progress(idx)).await;
                    continue;
                }

                let exec_result: Result<(), RunnerError> = match &action.action_type {
                    ActionType::Click
                    | ActionType::DoubleClick
                    | ActionType::RightClick
                    | ActionType::Type(_)
                    | ActionType::Scroll(_)
                    | ActionType::Navigate(_) => {
                        if dry_run {
                            info!(
                                action = ?action.action_type,
                                point = ?action.point,
                               "[DRY-RUN] Would execute synthetic action (execution bypassed)"
                            );
                            Ok(())
                        } else {
                            filter.inhibit_presence();
                            let res = hand.execute(&action).await.map_err(RunnerError::HandError);
                            filter.resume_presence();
                            res
                        }
                    }

                    ActionType::InscribeMove {
                        source,
                        destination,
                    } => {
                        if dry_run {
                            info!(
                                ?source,
                                ?destination,
                                "[DRY-RUN] Would move file (execution bypassed)"
                            );
                            Ok(())
                        } else {
                            let r = inscribe::move_file(source, destination, &trusted_roots).await;
                            if let Ok(ref final_dst) = r {
                                filter.mark(final_dst);
                            }
                            r.map(|_| ()).map_err(RunnerError::from)
                        }
                    }
                    ActionType::InscribeCopy {
                        source,
                        destination,
                    } => {
                        if dry_run {
                            info!(
                                ?source,
                                ?destination,
                                "[DRY-RUN] Would copy file (execution bypassed)"
                            );
                            Ok(())
                        } else {
                            let r = inscribe::copy_file(source, destination, &trusted_roots).await;
                            if let Ok((ref final_dst, _)) = r {
                                filter.mark(final_dst);
                            }
                            r.map(|_| ()).map_err(RunnerError::from)
                        }
                    }
                    ActionType::InscribeDelete { target } => {
                        if dry_run {
                            info!(?target, "[DRY-RUN] Would delete file (execution bypassed)");
                            Ok(())
                        } else {
                            inscribe::delete_file(target, &trusted_roots)
                                .await
                                .map_err(RunnerError::from)
                        }
                    }

                    ActionType::Shell {
                        command,
                        args,
                        detached,
                    } => {
                        if dry_run {
                            info!(
                                %command,
                                ?args,
                                detached = detached,
                                "[DRY-RUN] Would execute shell command (execution bypassed)"
                            );
                            Ok(())
                        } else {
                            let arg_refs: Vec<&str> =
                                args.iter().map(std::string::String::as_str).collect();
                            if *detached {
                                shell::spawn_detached(
                                    command.as_str(),
                                    command.as_str(),
                                    &arg_refs,
                                    &baton_allowed,
                                )
                                .await
                                .map_err(RunnerError::from)
                            } else {
                                shell::run(
                                    command.as_str(),
                                    command.as_str(),
                                    &arg_refs,
                                    &baton_allowed,
                                )
                                .await
                                .map(|_| ())
                                .map_err(RunnerError::from)
                            }
                        }
                    }
                    _ => Ok(()),
                };

                if let Err(e) = exec_result {
                    error!(%e, id = %node.id, "Runner: action failed");
                    let _ = event_tx
                        .send(RunEvent::Panic(format!(
                            "Step '{label}' failed: {e}",
                            label = node.label
                        )))
                        .await;
                    break;
                }

                let _ = event_tx.send(RunEvent::Progress(idx)).await;
            }

            // Always signal Done so the Atlas doesn't get stuck in Yielded state.
            let _ = event_tx.send(RunEvent::Done).await;
        }
    });
}
