//! atlas.rs — The Atlas: the core FSM orchestrator.
//!
//! Responsibilities:
//!   - Owns the engine's `EngineState` machine.
//!   - Drives sequence execution step-by-step.
//!   - Handles save/load of the Ordinance graph to disk.
//!   - Emits `RunEvent`s to any connected consumer (UI terminal, logger).
//!
//! The Atlas does NOT touch hardware (that is The Hand / vassal-bridge)
//! and does NOT watch for signals (that is The Vigil).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    sync::mpsc::{Receiver, SyncSender},
    time::Instant,
};
use tracing::{debug, error, info, warn};

use crate::ordinance::{
    IoCommand, IoResult, LogEntry, NodeKind, OrdNode, RunEvent, push_log,
};

// ── Engine State ──────────────────────────────────────────────────────────────

/// The Atlas FSM states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    /// Standing by — no sequence active.
    Idle,
    /// A sequence is actively executing.
    Executing,
    /// Execution yielded to human presence (Presence detected input).
    Yielded,
    /// A non-recoverable fault has occurred.
    Faulted,
}

// ── Run State ─────────────────────────────────────────────────────────────────

/// Live state of a running sequence, held by the Atlas during execution.
pub struct RunState {
    pub nodes_len: usize,
    pub current_index: usize,
    pub rx: Receiver<RunEvent>,
    pub abort_tx: std::sync::mpsc::Sender<()>,
}

// ── Atlas ─────────────────────────────────────────────────────────────────────

/// The Atlas: owns engine state and drives sequence execution.
pub struct Atlas {
    pub state: EngineState,
    pub run_state: Option<RunState>,
    pub engine_logs: Arc<Mutex<Vec<LogEntry>>>,
    /// Timestamp of the last sequence start — used for stale-event guard.
    pub last_start: Option<Instant>,
}

impl Atlas {
    pub fn new() -> Self {
        let logs: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(vec![
            LogEntry {
                tag: "ATLAS".into(),
                message: "Engine boot sequence initiated.".into(),
                is_error: false,
            },
        ]));
        Self {
            state: EngineState::Idle,
            run_state: None,
            engine_logs: logs,
            last_start: None,
        }
    }

    /// Poll the active run state for new events. Call on every UI tick.
    pub fn poll_events(&mut self) {
        if let Some(rs) = &mut self.run_state {
            while let Ok(event) = rs.rx.try_recv() {
                match event {
                    RunEvent::Log(entry) => {
                        if let Ok(mut logs) = self.engine_logs.lock() {
                            logs.push(entry);
                        }
                    }
                    RunEvent::Progress(idx) => {
                        rs.current_index = idx;
                        debug!(idx, "Atlas advanced to node");
                    }
                    RunEvent::Panic(msg) => {
                        push_log(&self.engine_logs, "ATLAS", &msg, true);
                        error!(%msg, "Atlas entered Faulted state");
                        self.state = EngineState::Faulted;
                    }
                    RunEvent::Done => {
                        push_log(&self.engine_logs, "ATLAS", "Sequence complete.", false);
                        info!("Atlas sequence complete — returning to Idle");
                        self.run_state = None;
                        self.state = EngineState::Idle;
                        return;
                    }
                }
            }
        }
        // Guard: clean up if the sequence index overran node count
        if self.run_state.as_ref().map_or(false, |rs| rs.current_index >= rs.nodes_len) {
            self.run_state = None;
            self.state = EngineState::Idle;
        }
    }

    /// Yield control to the human — abort the active sequence non-destructively.
    pub fn yield_to_presence(&mut self) {
        if let Some(rs) = &self.run_state {
            let _ = rs.abort_tx.send(());
        }
        self.state = EngineState::Yielded;
        push_log(&self.engine_logs, "PRESN", "Human presence detected — yielding.", false);
        warn!("Atlas yielded to human presence");
    }

    /// Resume from a Yielded state back to Idle (sequence was already aborted).
    pub fn resume_from_yield(&mut self) {
        if self.state == EngineState::Yielded {
            self.state = EngineState::Idle;
            push_log(&self.engine_logs, "ATLAS", "Resumed from yield — Idle.", false);
        }
    }

    /// Clear the Faulted state so new sequences can be started.
    pub fn clear_fault(&mut self) {
        if self.state == EngineState::Faulted {
            self.state = EngineState::Idle;
            push_log(&self.engine_logs, "ATLAS", "Fault cleared — Idle.", false);
        }
    }

    /// Abort the running sequence immediately.
    pub fn stop(&mut self) {
        if let Some(rs) = &self.run_state {
            let _ = rs.abort_tx.send(());
        }
        self.run_state = None;
        self.state = EngineState::Idle;
        push_log(&self.engine_logs, "ATLAS", "Sequence halted by command.", true);
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

// ── I/O Worker ───────────────────────────────────────────────────────────────

/// Spawns the dedicated I/O thread for disk operations.
///
/// The Atlas sends `IoCommand`s; results come back as `IoResult`s.
/// This keeps all blocking file I/O off the engine hot-path.
pub fn spawn_io_thread(rx: Receiver<IoCommand>, tx: SyncSender<IoResult>) {
    std::thread::spawn(move || {
        const GRAPH_PATH: &str = "vassal-data/graph_state.json";

        while let Ok(cmd) = rx.recv() {
            match cmd {
                IoCommand::SaveGraph(json) => {
                    if let Err(e) = std::fs::create_dir_all("vassal-data") {
                        let _ = tx.send(IoResult::Error(format!("Cannot create data dir: {e}")));
                        continue;
                    }
                    match std::fs::write(GRAPH_PATH, &json) {
                        Ok(_) => { let _ = tx.send(IoResult::SaveSuccess); }
                        Err(e) => { let _ = tx.send(IoResult::Error(e.to_string())); }
                    }
                }
                IoCommand::LoadGraph => {
                    match std::fs::read_to_string(GRAPH_PATH) {
                        Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                            Ok(snap) => { let _ = tx.send(IoResult::LoadSuccess(snap)); }
                            Err(e) => { let _ = tx.send(IoResult::Error(format!("Corrupt save: {e}"))); }
                        },
                        Err(e) => { let _ = tx.send(IoResult::Error(format!("Cannot read save: {e}"))); }
                    }
                }
            }
        }
    });
}

// ── Graph Compiler ────────────────────────────────────────────────────────────

/// Walk the graph from the Entry node and emit a flat, ordered sequence of `OrdNode`s.
///
/// Returns `None` with an error if no Entry node is found.
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

// ── Serialisation Helpers ─────────────────────────────────────────────────────

/// Serialise the current node map into a pretty-printed JSON snapshot.
pub fn serialise_graph(nodes: &HashMap<String, OrdNode>) -> Result<String, String> {
    serde_json::to_string_pretty(nodes).map_err(|e| format!("Serialise error: {e}"))
}

/// Deserialise a JSON snapshot back into a node map.
pub fn deserialise_graph(json: &serde_json::Value) -> Result<HashMap<String, OrdNode>, String> {
    serde_json::from_value(json.clone()).map_err(|e| format!("Deserialise error: {e}"))
}
