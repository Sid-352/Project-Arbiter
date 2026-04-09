//! ordinance.rs — The Vassal data contract.
//!
//! Defines all pure data types for triggers, actions, sequences, and
//! I/O messaging. No logic lives here — this is the shared vocabulary
//! used by The Atlas, The Vigil, and the UI terminal.

use std::{collections::HashMap, sync::{Arc, Mutex}};
use serde::{Deserialize, Serialize};

// ── Actions ──────────────────────────────────────────────────────────────────

/// A discrete hardware or system action the engine can perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActionType {
    /// Left mouse click at the current / specified position.
    Click,
    /// Two rapid left clicks.
    DoubleClick,
    /// Right mouse click.
    RightClick,
    /// Type a string through the OS keyboard pipeline.
    Type(String),
    /// Vertical scroll by `i32` ticks (positive = down).
    Scroll(i32),
    /// OS-native navigation keystroke (e.g., Win+S, Alt+Tab).
    /// The string is passed directly to enigo's key sequence parser.
    Navigate(String),
    /// No-op pause for `u64` milliseconds.
    Wait(u64),
}

/// An absolute screen coordinate validated by The Hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A resolved, executable action with optional target coordinates and delay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub action_type: ActionType,
    /// Screen target — `None` for keyboard-only or Wait actions.
    pub point: Option<Point>,
    /// Pre-execution delay in milliseconds (The Queue's pacing gate).
    pub delay_ms: u64,
}

// ── Ordinance Nodes (Sequence Graph) ─────────────────────────────────────────

/// The kind of node in an Ordinance sequence graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Entry point — every sequence must have exactly one.
    Entry,
    /// A hardware action step.
    Action,
    /// A Summons trigger node (condition that must be true to proceed).
    Trigger,
}

/// A single node in a compiled Ordinance sequence.
///
/// Derived from the graph editor's blueprint, stripped of all visual
/// perception fields that existed in the Lithos era.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrdNode {
    /// Stable UUID for this node, used for graph wiring.
    pub id: String,
    /// Human-readable label shown in the editor.
    pub label: String,
    /// The action this node executes (serialised as a string key for the editor).
    pub internal_state: String,
    pub kind: NodeKind,
    /// Adjacency map: output-port-name → next node UUID.
    #[serde(default)]
    pub next_nodes: HashMap<String, String>,
}

// ── Summons (Triggers) ────────────────────────────────────────────────────────

/// The specific signal that starts or gates a sequence — The Summons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Summons {
    /// A file matching `glob` finished writing inside `watch_path`.
    #[cfg(feature = "vigil-fs")]
    FileCreated { watch_path: String, glob: String },
    /// A user-defined global hotkey combination.
    #[cfg(feature = "vigil-keys")]
    Hotkey { combo: String },
    /// A named process appeared in the process list.
    ProcessAppeared { name: String },
    /// Manual trigger (used for testing and UI-triggered runs).
    Manual,
}

// ── Run-time Events ───────────────────────────────────────────────────────────

/// Events emitted by the Atlas FSM to any listening consumers (UI, logger).
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// A log line to be displayed in the Terminal of Commands.
    Log(LogEntry),
    /// The FSM advanced to node at index `usize`.
    Progress(usize),
    /// A non-recoverable fault — engine halted.
    Panic(String),
    /// Sequence completed normally.
    Done,
}

/// A single structured log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Short category tag shown in the terminal (e.g. "ATLAS", "VIGIL", "HAND").
    pub tag: String,
    pub message: String,
    pub is_error: bool,
}

/// Helper: push a log entry into a shared log buffer, capping at 1 000 lines.
pub fn push_log(logs: &Arc<Mutex<Vec<LogEntry>>>, tag: &str, msg: &str, is_error: bool) {
    if let Ok(mut v) = logs.lock() {
        if v.len() >= 1_000 {
            v.remove(0);
        }
        v.push(LogEntry { tag: tag.into(), message: msg.into(), is_error });
    }
}

// ── I/O Commands (Ordinance persistence) ─────────────────────────────────────

/// Commands sent from the UI or engine to the I/O worker thread.
#[derive(Debug)]
pub enum IoCommand {
    /// Serialise and persist the current sequence graph.
    SaveGraph(String),
    /// Load the persisted sequence graph from disk.
    LoadGraph,
}

/// Responses from the I/O worker thread.
#[derive(Debug)]
pub enum IoResult {
    SaveSuccess,
    LoadSuccess(serde_json::Value),
    Error(String),
}
