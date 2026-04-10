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

use crate::ordinance::{push_log, ExecData, LogEntry, NodeKind, OrdNode, RunEvent, Summons};

#[cfg(feature = "presence")]
use crate::presence::PresenceSignal;

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
    pub registry: HashMap<String, Vec<OrdNode>>,

    // Held during an active sequence to allow interruption.
    active_abort: Option<oneshot::Sender<()>>,
}

impl Atlas {
    pub fn new() -> Self {
        let logs: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(vec![LogEntry {
            tag: "ATLAS".into(),
            message: "Engine boot sequence initiated.".into(),
            is_error: false,
        }]));
        Self {
            state: EngineState::Idle,
            engine_logs: logs,
            last_start: None,
            registry: HashMap::new(),
            active_abort: None,
        }
    }

    /// Register a sequence to a trigger key.
    pub fn register_ordinance(&mut self, summons_key: String, nodes: Vec<OrdNode>) {
        info!(%summons_key, "Atlas: registering ordinance");
        self.registry.insert(summons_key, nodes);
    }

    /// The main async event loop.
    pub async fn run(
        mut self,
        mut vigil_rx: mpsc::Receiver<Summons>,
        #[cfg(feature = "presence")] mut presence_rx: mpsc::Receiver<PresenceSignal>,
        mut run_event_rx: mpsc::Receiver<RunEvent>,
        exec_tx: mpsc::Sender<ExecData>,
        mut shutdown_rx: oneshot::Receiver<()>,
    ) {
        info!("Atlas: run loop started");

        loop {
            tokio::select! {
                // 1. Process Shutdown
                _ = &mut shutdown_rx => {
                    info!("Atlas: shutting down");
                    if let Some(tx) = self.active_abort.take() {
                        let _ = tx.send(());
                    }
                    break;
                }

                // 2. Process incoming Triggers (Summons)
                Some(summons) = vigil_rx.recv() => {
                    if self.state == EngineState::Idle {
                        let key = summons.to_registry_key();
                        if let Some(nodes) = self.registry.get(&key).cloned() {
                            info!(%key, "Atlas: summons matched, dispatching sequence");
                            push_log(&self.engine_logs, "ATLAS", &format!("Summons matched: {}", key), false);

                            self.state = EngineState::Executing;
                            self.last_start = Some(Instant::now());

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
                                nodes,
                                context,
                                abort_rx,
                            };

                            if let Err(e) = exec_tx.send(exec_data).await {
                                error!(%e, "Atlas: failed to dispatch to Executor");
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
                    { std::future::pending::<Option<()>>().await }
                } => {
                    if let Some(_signal) = res {
                        if self.state == EngineState::Executing {
                            info!("Atlas: human presence detected, yielding");
                            self.yield_to_presence();
                        }
                    }
                }

                // 4. Process Executor Status updates
                Some(event) = run_event_rx.recv() => {
                    self.handle_run_event(event);
                }
            }
        }
    }

    fn yield_to_presence(&mut self) {
        if let Some(tx) = self.active_abort.take() {
            let _ = tx.send(());
        }
        self.state = EngineState::Yielded;
        push_log(
            &self.engine_logs,
            "PRESN",
            "Human presence detected — yielding.",
            false,
        );
        warn!("Atlas yielded to human presence");
    }

    fn handle_run_event(&mut self, event: RunEvent) {
        match event {
            RunEvent::Log(entry) => {
                if let Ok(mut logs) = self.engine_logs.lock() {
                    logs.push(entry);
                }
            }
            RunEvent::Progress(idx) => {
                debug!(idx, "Atlas: node execution complete");
            }
            RunEvent::Panic(msg) => {
                push_log(&self.engine_logs, "ATLAS", &msg, true);
                error!(%msg, "Atlas entered Faulted state");
                self.state = EngineState::Faulted;
                self.active_abort = None;
            }
            RunEvent::Done => {
                push_log(&self.engine_logs, "ATLAS", "Sequence complete.", false);
                info!("Atlas sequence complete — returning to Idle");
                self.state = EngineState::Idle;
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

// ── Graph Compiler & IO ───────────────────────────────────────────────────────

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
