//! atlas.rs — The Atlas: the core FSM orchestrator.
//!
//! Responsibilities:
//!   - Owns the engine's `EngineState` machine.
//!   - Drives sequence execution via an async run loop.
//!   - Maintains the Ordinance registry (Summons -> Sequence).
//!   - Emits `RunEvent`s to connected consumers and handles UI log pushes.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::ordinance::{DecreeId, EnvContext, ExecData, NodeId, NodeKind, OrdNode, Ordinance, PresenceConfig, RunEvent, Summons};
use crate::protocol::{ForgeCommand, LogEntry};

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
    pub active_ordinance_id: Option<DecreeId>,
    
    /// Tracks process names that already have an active watcher task.
    pub watched_processes: HashSet<String>,

    /// Tracks active Ward watchers by their Ward ID to allow stopping/restarting them.
    pub active_watchers: HashMap<String, tokio::sync::broadcast::Sender<()>>,

    // Held during an active sequence to allow interruption.
    active_abort: Option<oneshot::Sender<()>>,
}

impl Atlas {
    pub fn new() -> Self {
        let logs: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(vec![LogEntry {
            time: chrono::Utc::now().to_rfc3339(),
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
            watched_processes: HashSet::new(),
            active_watchers: HashMap::new(),
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
        vigil_tx: mpsc::Sender<Summons>,
        #[cfg_attr(not(feature = "presence"), allow(unused_variables))]
        presence_rx: &mut mpsc::Receiver<PresenceSignalInner>,
        run_event_rx: &mut mpsc::Receiver<RunEvent>,
        run_tx: mpsc::Sender<ExecData>,
        reset_rx: &mut mpsc::Receiver<()>,
        forge_cmd_rx: &mut mpsc::Receiver<ForgeCommand>,
        shutdown_rx: &mut oneshot::Receiver<()>,
        log_broadcast: tokio::sync::broadcast::Sender<LogEntry>,
        filter: crate::filter::ArbiterFilter,
    ) {
        info!("Atlas: run loop started");

        loop {
            tokio::select! {
                // 1. Process Shutdown
                _ = &mut *shutdown_rx => {
                    info!("Atlas: shutting down");
                    // Stop all watchers
                    for (id, tx) in self.active_watchers.drain() {
                        debug!(%id, "Atlas: stopping watcher on shutdown");
                        let _ = tx.send(());
                    }
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
                            time: chrono::Utc::now().to_rfc3339(),
                            tag: "ATLAS".into(),
                            message: "Engine fault cleared manually.".into(),
                            is_error: false,
                            ordinance_id: None,
                        });
                    }
                }

                // ── Process Forge Commands ──
                Some(cmd) = forge_cmd_rx.recv() => {
                    match cmd {
                        ForgeCommand::SaveDecree(def) => {
                            info!(id = %def.id, "Atlas: received SaveDecree command");

                            // 1. Update the Ledger on disk
                            let mut ledger = crate::ledger::load().unwrap_or_else(|e| {
                                error!("Atlas: failed to load ledger for save: {}", e);
                                crate::ledger::ArbiterLedger::default()
                            });

                            // Update or insert
                            if let Some(existing) = ledger.ordinances.iter_mut().find(|o| o.id == def.id) {
                                *existing = def.clone();
                            } else {
                                ledger.ordinances.push(def.clone());
                            }
                            let _ = crate::ledger::save(&ledger);

                            // 2. Hot-reload logic
                            let mut context = EnvContext::new();
                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            context.insert("timestamp", &now_unix.to_string());
                            context.insert("timestamp_local", &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string());

                            let summons = match &def.summons {
                                crate::ledger::SummonsDef::FileCreated { ward_id, pattern, recursive } => {
                                    // 2a. Ensure the Ward exists and is up to date
                                    let mut ward_exists = false;
                                    if let Some(w) = ledger.wards.iter_mut().find(|w| w.path.to_string_lossy() == *ward_id) {
                                        ward_exists = true;
                                        if w.recursive != *recursive {
                                            info!(path = %ward_id, from = w.recursive, to = *recursive, "Atlas: updating Ward recursion level (Allowed/Denied)");
                                            w.recursive = *recursive;
                                            
                                            // Stop old watcher and spawn new one with correct mode
                                            if let Some(stop_tx) = self.active_watchers.get(ward_id) {
                                                let _ = stop_tx.send(());
                                            }
                                            let new_stop_tx = crate::vigil::fs::spawn_watcher(w.clone(), filter.clone(), vigil_tx.clone());
                                            self.active_watchers.insert(ward_id.clone(), new_stop_tx);
                                        }
                                    }

                                    if !ward_exists && !ward_id.is_empty() {
                                        info!(path = %ward_id, "Atlas: path not found in Wards, creating new entry");
                                        let new_ward = crate::ordinance::WardConfig {
                                            id: ward_id.clone(),
                                            path: std::path::PathBuf::from(ward_id),
                                            pattern: "*".into(),
                                            layer: crate::ordinance::WardLayer::Surface,
                                            recursive: *recursive,
                                        };
                                        ledger.wards.push(new_ward.clone());
                                        // Save again to persist the new ward
                                        let _ = crate::ledger::save(&ledger);
                                        let stop_tx = crate::vigil::fs::spawn_watcher(new_ward, filter.clone(), vigil_tx.clone());
                                        self.active_watchers.insert(ward_id.clone(), stop_tx);
                                    }

                                    let ward = ledger.wards.iter().find(|w| w.path.to_string_lossy() == *ward_id);
                                    if let Some(w) = ward {
                                        Summons::FileCreated {
                                            watch_path: w.path.clone(),
                                            pattern: pattern.clone(),
                                            context,
                                        }
                                    } else {
                                        warn!(id = %def.id, ward_id, "Atlas: Ward not found for dynamic registration");
                                        continue;
                                    }
                                }
                                crate::ledger::SummonsDef::Hotkey { combo } => {
                                    let _ = crate::vigil::keys::register_hotkey(combo.clone(), vigil_tx.clone());
                                    Summons::Hotkey {
                                        combo: combo.clone(),
                                        context,
                                    }
                                }
                                crate::ledger::SummonsDef::ProcessAppeared { name } => {
                                    if !self.watched_processes.contains(name) {
                                        info!(%name, "Atlas: spawning new process watcher");
                                        crate::vigil::sys::spawn_watcher(name.clone(), vigil_tx.clone());
                                        self.watched_processes.insert(name.clone());
                                    } else {
                                        debug!(%name, "Atlas: process watcher already active, skipping spawn");
                                    }
                                    Summons::ProcessAppeared {
                                        name: name.clone(),
                                        context,
                                    }
                                }
                                crate::ledger::SummonsDef::Manual => Summons::Manual {
                                    context,
                                },
                            };

                            self.register_ordinance(
                                summons.to_registry_key(),
                                Ordinance {
                                    nodes: def.nodes,
                                    presence_config: def.presence_config,
                                },
                            );

                            let _ = log_broadcast.send(LogEntry {
                                time: chrono::Utc::now().to_rfc3339(),
                                tag: "ATLAS".into(),
                                message: format!("Decree '{}' registered and saved.", def.label),
                                is_error: false,
                                ordinance_id: Some(def.id.0.clone()),
                            });
                        }
                        ForgeCommand::ReloadWards => {
                            info!("Atlas: reloading Signet configuration (Live)");
                            crate::signet::reload_cache();
                            let _ = log_broadcast.send(LogEntry {
                                time: chrono::Utc::now().to_rfc3339(),
                                tag: "SIGNT".into(),
                                message: "Signet configuration reloaded from vault.".into(),
                                is_error: false,
                                ordinance_id: None,
                            });
                        }
                        ForgeCommand::ManualRun { summons_key } => {
                            if self.state == EngineState::Idle {
                                info!(%summons_key, "Atlas: received ManualRun command");
                                if let Some(ord) = self.registry.get(&summons_key).cloned() {
                                    let mut context = EnvContext::new();
                                    context.insert("trigger_mode", "Manual");
                                    self.dispatch_ordinance(summons_key, ord, context, &run_tx, &log_broadcast).await;
                                } else {
                                    warn!(%summons_key, "Atlas: ManualRun failed — ordinance not found");
                                }
                            } else {
                                debug!("Atlas: ignoring ManualRun, engine is busy");
                            }
                        }
                    }
                }

                // 2. Process incoming Triggers (Summons)
                Some(summons) = vigil_rx.recv() => {
                    if self.state == EngineState::Idle {
                        let mut key = summons.to_registry_key();
                        let mut ordinance = self.registry.get(&key).cloned();

                        // ── Fuzzy Matching for File Events ──
                        if ordinance.is_none() {
                            if let Summons::FileCreated { watch_path, .. } = &summons {
                                let filename = match &summons {
                                    Summons::FileCreated { context, .. } => context.variables.get("file_name").cloned().unwrap_or_default(),
                                    _ => String::new(),
                                };
                                
                                if !filename.is_empty() {
                                    let path_prefix = format!("FileCreated|{}|", watch_path.display());
                                    
                                    for (reg_key, reg_ord) in &self.registry {
                                        if reg_key.starts_with(&path_prefix) {
                                            let pattern = &reg_key[path_prefix.len()..];
                                            if let Ok(matcher) = globset::GlobBuilder::new(pattern).case_insensitive(true).build() {
                                                if matcher.compile_matcher().is_match(&filename) {
                                                    debug!(%reg_key, %filename, "Atlas: fuzzy summons match found");
                                                    key = reg_key.clone();
                                                    ordinance = Some(reg_ord.clone());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(ordinance) = ordinance {
                            // Extract context
                            let context = match summons {
                                #[cfg(feature = "vigil-fs")]
                                Summons::FileCreated { context, .. } => context,
                                #[cfg(feature = "vigil-keys")]
                                Summons::Hotkey { context, .. } => context,
                                Summons::ProcessAppeared { context, .. } => context,
                                Summons::Manual { context, .. } => context,
                            };

                            self.dispatch_ordinance(key, ordinance, context, &run_tx, &log_broadcast).await;
                        } else {
                            debug!(%key, "Atlas: unassigned Summons received, ignoring");
                        }
                    } else {
                        debug!("Atlas: ignoring Summons, Engine is busy");
                    }

                    // Periodic Cleanup of dead watchers
                    self.active_watchers.retain(|_, tx| tx.receiver_count() > 0);
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
                            #[cfg(feature = "presence")]
                            {
                                use crate::presence::PresenceSignal;
                                match signal {
                                    PresenceSignal::MouseInput if self.active_presence_config.ignore_mouse => continue,
                                    PresenceSignal::KeyboardInput if self.active_presence_config.ignore_keyboard => continue,
                                    _ => {}
                                }
                            }

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

    async fn dispatch_ordinance(
        &mut self,
        key: String,
        ordinance: Ordinance,
        context: EnvContext,
        run_tx: &mpsc::Sender<ExecData>,
        log_broadcast: &tokio::sync::broadcast::Sender<LogEntry>,
    ) {
        info!(%key, "Atlas: dispatching sequence");
        self.active_ordinance_id = Some(DecreeId(key.clone()));

        let msg = format!("Summons matched: {}", key);
        push_log(&self.engine_logs, "ATLAS", &msg, false, self.active_ordinance_id.as_ref().map(|id| id.0.clone()));
        let _ = log_broadcast.send(LogEntry { 
            time: chrono::Utc::now().to_rfc3339(),
            tag: "ATLAS".into(), 
            message: msg, 
            is_error: false, 
            ordinance_id: self.active_ordinance_id.as_ref().map(|id| id.0.clone())
        });

        self.state = EngineState::Executing;
        self.last_start = Some(Instant::now());
        self.active_presence_config = ordinance.presence_config.clone();

        let (abort_tx, abort_rx) = oneshot::channel();
        self.active_abort = Some(abort_tx);

        // ── Recursion Safety ──
        if let Some(p) = context.variables.get("file_path") {
            let component_count = p.split(|c| c == '/' || c == '\\').count();
            if component_count > 20 {
                error!(%p, "Atlas: MAX_RECURSION_DEPTH exceeded, aborting sequence to prevent path explosion");
                self.state = EngineState::Idle;
                self.active_ordinance_id = None;
                return;
            }
        }

        let exec_data = ExecData {
            nodes: ordinance.nodes,
            context,
            presence_config: ordinance.presence_config,
            ordinance_id: self.active_ordinance_id.clone(),
            trigger_time: Instant::now(),
            abort_rx,
        };

        if let Err(e) = run_tx.send(exec_data).await {
            error!(%e, "Atlas: failed to dispatch to Runner");
            self.state = EngineState::Faulted;
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
            self.active_ordinance_id.as_ref().map(|id| id.0.clone()),
        );
        let _ = log_broadcast.send(LogEntry { 
            time: chrono::Utc::now().to_rfc3339(),
            tag: "PRESN".into(), 
            message: msg.into(), 
            is_error: false,
            ordinance_id: self.active_ordinance_id.as_ref().map(|id| id.0.clone()),
        });
        warn!("Atlas yielded to human presence");
    }

    fn handle_run_event(&mut self, event: RunEvent, log_broadcast: &tokio::sync::broadcast::Sender<LogEntry>) {
        match event {
            RunEvent::Log(mut entry) => {
                if entry.time.is_empty() {
                    entry.time = chrono::Utc::now().to_rfc3339();
                }
                let _ = log_broadcast.send(entry.clone());
                if let Ok(mut logs) = self.engine_logs.lock() {
                    logs.push(entry);
                }
            }
            RunEvent::Progress(idx) => {
                debug!(idx, "Atlas: node execution complete");
            }
            RunEvent::Panic(msg) => {
                push_log(&self.engine_logs, "ATLAS", &msg, true, self.active_ordinance_id.as_ref().map(|id| id.0.clone()));
                let _ = log_broadcast.send(LogEntry { 
                    time: chrono::Utc::now().to_rfc3339(),
                    tag: "ATLAS".into(), 
                    message: msg.clone(), 
                    is_error: true,
                    ordinance_id: self.active_ordinance_id.as_ref().map(|id| id.0.clone()),
                });
                error!(%msg, "Atlas entered Faulted state");
                self.state = EngineState::Faulted;
                self.active_abort = None;
            }
            RunEvent::Done => {
                let msg = "Sequence complete.";
                push_log(&self.engine_logs, "ATLAS", msg, false, self.active_ordinance_id.as_ref().map(|id| id.0.clone()));
                let _ = log_broadcast.send(LogEntry { 
                    time: chrono::Utc::now().to_rfc3339(),
                    tag: "ATLAS".into(), 
                    message: msg.into(), 
                    is_error: false, 
                    ordinance_id: self.active_ordinance_id.as_ref().map(|id| id.0.clone()),
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

pub fn compile_sequence(nodes_map: &HashMap<NodeId, OrdNode>) -> Option<Vec<OrdNode>> {
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

/// Helper: push a log entry into a shared log buffer, capping at 1 000 lines.
pub fn push_log(
    logs: &Arc<Mutex<Vec<LogEntry>>>,
    tag: &str,
    msg: &str,
    is_error: bool,
    ordinance_id: Option<String>,
) {
    if let Ok(mut v) = logs.lock() {
        if v.len() >= 1_000 {
            v.remove(0);
        }
        v.push(LogEntry {
            time: chrono::Utc::now().to_rfc3339(),
            tag: tag.into(),
            message: msg.into(),
            is_error,
            ordinance_id,
        });
    }
}
