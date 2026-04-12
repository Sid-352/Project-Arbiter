//! atlas.rs — The Atlas: the core FSM orchestrator.
//!
//! Responsibilities:
//!   - Owns the engine's `EngineState` machine.
//!   - Drives sequence execution via an async run loop.
//!   - Maintains the Ordinance registry (Summons -> Sequence).
//!   - Emits `RunEvent`s to connected consumers and handles UI log pushes.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::ordinance::{push_log, ExecData, LogEntry, NodeKind, OrdNode, Ordinance, PresenceConfig, RunEvent, Summons};

#[cfg(feature = "presence")]
use crate::presence::PresenceSignal;

#[cfg(feature = "presence")]
type PresenceSignalInner = PresenceSignal;
#[cfg(not(feature = "presence"))]
type PresenceSignalInner = ();

// ── Engine State ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    Executing,
    Yielded,
    Faulted,
}

// ── Atlas ─────────────────────────────────────────────────────────────────────

/// The Atlas: owns engine state, registry, and drives sequence execution.
pub struct Atlas {
    pub state: EngineState,
    pub engine_logs: Arc<Mutex<Vec<LogEntry>>>,
    pub last_start: Option<Instant>,
    pub registry: HashMap<String, Ordinance>,
    pub active_presence_config: PresenceConfig,
    pub active_ordinance_id: Option<String>,

    // Held during an active sequence to allow interruption.
    active_abort: Option<oneshot::Sender<()>>,
}

impl Atlas {
    pub fn new() -> Self {
        let logs: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(vec![LogEntry {
            tag: "ATLAS".into(),
            message: "Engine boot sequence initiated.".into(),
            is_error: false,
            ordinance_id: None,
        }]));
        Self {
            state: EngineState::Idle,
            engine_logs: logs,
            last_start: None,
            registry: HashMap::new(),
            active_presence_config: PresenceConfig::default(),
            active_ordinance_id: None,
            active_abort: None,
        }
    }

    /// Register a sequence to a trigger key.
    pub fn register_ordinance(&mut self, summons_key: String, ordinance: Ordinance) {
        info!(%summons_key, "Atlas: registering ordinance");
        self.registry.insert(summons_key, ordinance);
    }

    /// The main async event loop.
    pub async fn run(
        &mut self,
        vigil_rx: &mut mpsc::Receiver<Summons>,
        #[cfg_attr(not(feature = "presence"), allow(unused_variables))]
        presence_rx: &mut mpsc::Receiver<PresenceSignalInner>,
        run_event_rx: &mut mpsc::Receiver<RunEvent>,
        run_tx: mpsc::Sender<ExecData>,
        reset_rx: &mut mpsc::Receiver<()>,
        shutdown_rx: &mut oneshot::Receiver<()>,
        log_broadcast: tokio::sync::broadcast::Sender<LogEntry>,
    ) {
        info!("Atlas: run loop started");

        loop {
            tokio::select! {
                // 1. Process Shutdown
                _ = &mut *shutdown_rx => {
                    info!("Atlas: shutting down");
                    if let Some(tx) = self.active_abort.take() {
                        let _ = tx.send(());
                    }
                    break;
                }

                // ── Process Manual Reset ──
                Some(_) = reset_rx.recv() => {
                    if self.state == EngineState::Faulted {
                        info!("Atlas: reset signal received, clearing Faulted state");
                        self.state = EngineState::Idle;
                        let _ = log_broadcast.send(LogEntry {
                            tag: "ATLAS".into(),
                            message: "Engine fault cleared manually.".into(),
                            is_error: false,
                            ordinance_id: None,
                        });
                    }
                }

                // 2. Process incoming Triggers (Summons)
                Some(summons) = vigil_rx.recv() => {
                    if self.state == EngineState::Idle {
                        let key = summons.to_registry_key();
                        if let Some(ordinance) = self.registry.get(&key).cloned() {
                            info!(%key, "Atlas: summons matched, dispatching sequence");
                            self.active_ordinance_id = Some(key.clone());

                            let msg = format!("Summons matched: {}", key);
                            push_log(&self.engine_logs, "ATLAS", &msg, false, self.active_ordinance_id.clone());
                            let _ = log_broadcast.send(LogEntry { 
                                tag: "ATLAS".into(), 
                                message: msg, 
                                is_error: false, 
                                ordinance_id: self.active_ordinance_id.clone() 
                            });

                            self.state = EngineState::Executing;
                            self.last_start = Some(Instant::now());
                            self.active_presence_config = ordinance.presence_config.clone();

                            let (abort_tx, abort_rx) = oneshot::channel();
                            self.active_abort = Some(abort_tx);

                            // Extract context
                            let context = match summons {
                                #[cfg(feature = "vigil-fs")]
                                Summons::FileCreated { context, .. } => context,
                                #[cfg(feature = "vigil-keys")]
                                Summons::Hotkey { context, .. } => context,
                                Summons::ProcessAppeared { context, .. } => context,
                                Summons::Manual { context, .. } => context,
                            };

                            let exec_data = ExecData {
                                nodes: ordinance.nodes,
                                context,
                                presence_config: ordinance.presence_config,
                                ordinance_id: Some(key),
                                abort_rx,
                            };

                            if let Err(e) = run_tx.send(exec_data).await {
                                error!(%e, "Atlas: failed to dispatch to Runner");
                                self.state = EngineState::Faulted;
                            }
                        } else {
                            debug!(%key, "Atlas: unassigned Summons received, ignoring");
                        }
                    } else {
                        debug!("Atlas: ignoring Summons, Engine is busy");
                    }
                }

                // 3. Process Human Yield (Presence)
                res = async {
                    #[cfg(feature = "presence")]
                    { presence_rx.recv().await }
                    #[cfg(not(feature = "presence"))]
                    { std::future::pending::<Option<()>>().await; None::<PresenceSignalInner> }
                } => {
                    #[allow(unused_variables)]
                    if let Some(signal) = res {
                        if self.state == EngineState::Executing {
                            // Sensitivity Filter (Scope-bound)
                            #[cfg(feature = "presence")]
                            {
                                use crate::presence::PresenceSignal;
                                match signal {
                                    PresenceSignal::MouseInput if self.active_presence_config.ignore_mouse => continue,
                                    PresenceSignal::KeyboardInput if self.active_presence_config.ignore_keyboard => continue,
                                    _ => {}
                                }
                            }

                            // Grace Period: Ignore presence for 1500ms after summons
                            if let Some(start) = self.last_start {
                                if start.elapsed().as_millis() < 1500 {
                                    debug!("Atlas: ignoring presence during 1500ms grace period");
                                    continue;
                                }
                            }
                            info!("Atlas: human presence detected, yielding");
                            self.yield_to_presence(&log_broadcast);
                        }
                    }
                }

                // 4. Process Runner Status updates
                Some(event) = run_event_rx.recv() => {
                    self.handle_run_event(event, &log_broadcast);
                }
            }
        }
    }

    fn yield_to_presence(&mut self, log_broadcast: &tokio::sync::broadcast::Sender<LogEntry>) {
        if let Some(tx) = self.active_abort.take() {
            let _ = tx.send(());
        }
        self.state = EngineState::Yielded;
        let msg = "Human presence detected — yielding.";
        push_log(
            &self.engine_logs,
            "PRESN",
            msg,
            false,
            self.active_ordinance_id.clone(),
        );
        let _ = log_broadcast.send(LogEntry { 
            tag: "PRESN".into(), 
            message: msg.into(), 
            is_error: false,
            ordinance_id: self.active_ordinance_id.clone(),
        });
        warn!("Atlas yielded to human presence");
    }

    fn handle_run_event(&mut self, event: RunEvent, log_broadcast: &tokio::sync::broadcast::Sender<LogEntry>) {
        match event {
            RunEvent::Log(entry) => {
                let _ = log_broadcast.send(entry.clone());
                if let Ok(mut logs) = self.engine_logs.lock() {
                    logs.push(entry);
                }
            }
            RunEvent::Progress(idx) => {
                debug!(idx, "Atlas: node execution complete");
            }
            RunEvent::Panic(msg) => {
                push_log(&self.engine_logs, "ATLAS", &msg, true, self.active_ordinance_id.clone());
                let _ = log_broadcast.send(LogEntry { 
                    tag: "ATLAS".into(), 
                    message: msg.clone(), 
                    is_error: true,
                    ordinance_id: self.active_ordinance_id.clone(),
                });
                error!(%msg, "Atlas entered Faulted state");
                self.state = EngineState::Faulted;
                self.active_abort = None;
            }
            RunEvent::Done => {
                let msg = "Sequence complete.";
                push_log(&self.engine_logs, "ATLAS", msg, false, self.active_ordinance_id.clone());
                let _ = log_broadcast.send(LogEntry { 
                    tag: "ATLAS".into(), 
                    message: msg.into(), 
                    is_error: false,
                    ordinance_id: self.active_ordinance_id.clone(),
                });
                info!("Atlas sequence complete — returning to Idle");
                self.state = EngineState::Idle;
                self.active_ordinance_id = None;
                self.active_abort = None;
            }
        }
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

// ── Graph Compiler ──────────────────────────────────────────────────────────

pub fn compile_sequence(nodes_map: &HashMap<String, OrdNode>) -> Option<Vec<OrdNode>> {
    let entry = nodes_map.values().find(|n| n.kind == NodeKind::Entry)?;

    let mut sequence = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    let mut visited = std::collections::HashSet::new();

    queue.push_back(entry.id.clone());

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(node) = nodes_map.get(&id) {
            if node.kind != NodeKind::Entry {
                sequence.push(node.clone());
            }
            for next_id in node.next_nodes.values() {
                if !visited.contains(next_id) {
                    queue.push_back(next_id.clone());
                }
            }
        }
    }

    Some(sequence)
}
