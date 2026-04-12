//! ordinance.rs — The Arbiter data contract.
//!
//! Defines all pure data types for triggers, actions, sequences, and
//! I/O messaging. No logic lives here — this is the shared vocabulary
//! used by The Atlas, The Vigil, and the UI terminal.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

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
    // ── File Operations (Inscribe) ──────────────────────────────────────────
    /// Move a file from `source` to `destination`.
    InscribeMove { source: PathBuf, destination: PathBuf },
    /// Copy a file from `source` to `destination`.
    InscribeCopy { source: PathBuf, destination: PathBuf },
    /// Delete a target file.
    InscribeDelete { target: PathBuf },
    // ── Shell Execution (Baton) ─────────────────────────────────────────────
    /// Execute a shell command.
    Shell {
        command: String,
        args: Vec<String>,
        detached: bool,
    },
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

// ── Presence Configuration ────────────────────────────────────────────────────

/// Configuration for human presence detection during sequence execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceConfig {
    /// If true, mouse movement/clicks will not trigger a yield.
    pub ignore_mouse: bool,
    /// If true, keyboard input will not trigger a yield.
    pub ignore_keyboard: bool,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            ignore_mouse: false,
            ignore_keyboard: false,
        }
    }
}

// ── Ward Configuration (Signet Authority Model) ───────────────────────────────

/// The permission tier granted to a watched directory.
///
/// Controls how deep the Vigil can reach into files that appear within the Ward.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum WardLayer {
    /// **Layer 1 — Surface Access:** OS-level metadata only (name, size, timestamps).
    /// No file handles are opened. Default for all Wards.
    #[default]
    Surface,
    /// **Layer 2 — Analytical Access:** Full content read — MIME/magic bytes, SHA256
    /// hash, entropy. Must be explicitly granted by the user for a specific Ward.
    Analytical,
}

/// Runtime configuration for a monitored Ward (watched directory).
///
/// Constructed by `main.rs` and passed into `vigil::fs::spawn_watcher`.
/// The Vigil itself makes no policy decisions — it only fires events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WardConfig {
    /// The absolute path of the directory to be watched.
    pub path: PathBuf,
    /// Glob pattern for filename matching (e.g. `"*.zip"`). Empty = match all.
    pub glob: String,
    /// The permission layer granted to this Ward.
    pub layer: WardLayer,
}

// ── Ordinance Nodes (Sequence Graph) ─────────────────────────────────────────

/// A full Ordinance definition: the nodes and its execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ordinance {
    pub nodes: Vec<OrdNode>,
    pub presence_config: PresenceConfig,
}

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
    FileCreated {
        watch_path: PathBuf,
        glob: String,
        context: EnvContext,
    },
    /// A user-defined global hotkey combination.
    #[cfg(feature = "vigil-keys")]
    Hotkey { combo: String, context: EnvContext },
    /// A named process appeared in the process list.
    ProcessAppeared { name: String, context: EnvContext },
    /// Manual trigger (used for testing and UI-triggered runs).
    Manual { context: EnvContext },
}

impl Summons {
    pub fn to_registry_key(&self) -> String {
        match self {
            #[cfg(feature = "vigil-fs")]
            Self::FileCreated {
                watch_path, glob, ..
            } => format!("FileCreated|{}|{}", watch_path.display(), glob),
            #[cfg(feature = "vigil-keys")]
            Self::Hotkey { combo, .. } => format!("Hotkey|{}", combo),
            Self::ProcessAppeared { name, .. } => format!("ProcessAppeared|{}", name),
            Self::Manual { .. } => "Manual".to_string(),
        }
    }
}

// ── Environment Context ───────────────────────────────────────────────────────

/// The payload associated with a fired trigger.
///
/// Provides variables for string interpolation (e.g., `${env.file_path}`).
///
/// **Static variables** (file name, timestamp, etc.) are inserted eagerly by the
/// Vigil at fire time via [`EnvContext::insert`].
///
/// **Lazy variables** (SHA256, MIME type) are computed on first access via
/// [`EnvContext::resolve`] and cached using [`OnceLock`] — the file is read
/// at most once per context lifetime regardless of how many times a macro
/// requests the same hash.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnvContext {
    /// Eagerly-inserted static variables available to all ordinances.
    pub variables: HashMap<String, String>,

    /// The real `PathBuf` of the triggering file, used for lazy resolution.
    /// Skipped during serialisation — re-populated from `file_path` on load if needed.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,

    /// `true` when this Ward has Layer 2 (Analytical) access enabled.
    /// The Signet Guard in `resolve()` consults this before performing any
    /// content read; returns `None` if the required layer is not granted.
    #[serde(skip)]
    pub integrity_scan: bool,

    /// Cached SHA-256 hex string. Computed once on first request, then frozen.
    /// `None` = not yet computed *or* computation failed / not permitted.
    #[serde(skip)]
    sha256_cache: OnceLock<Option<String>>,

    /// Cached MIME type string (e.g. `"application/zip"`). Same lazy semantics.
    #[serde(skip)]
    mime_cache: OnceLock<Option<String>>,
}

impl Default for EnvContext {
    fn default() -> Self {
        Self {
            variables: HashMap::new(),
            source_path: None,
            integrity_scan: false,
            sha256_cache: OnceLock::new(),
            mime_cache: OnceLock::new(),
        }
    }
}

impl Clone for EnvContext {
    /// Clones the static `variables` map and `source_path`/`integrity_scan` flags.
    /// The `OnceLock` caches are intentionally reset on clone so each clone
    /// independently re-computes if needed (avoids cross-clone aliasing).
    fn clone(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            source_path: self.source_path.clone(),
            integrity_scan: self.integrity_scan,
            sha256_cache: OnceLock::new(),
            mime_cache: OnceLock::new(),
        }
    }
}

impl EnvContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a static key/value variable (eagerly available to all ordinances).
    pub fn insert(&mut self, key: &str, value: &str) {
        self.variables.insert(key.to_string(), value.to_string());
    }

    /// Resolve a variable by key, performing lazy computation if necessary.
    ///
    /// **Resolution order:**
    /// 1. Static `variables` map (always available).
    /// 2. Lazy content variables — only if `integrity_scan` is `true`.
    ///    If the Ward is Surface-only, these return `None` (Signet Guard).
    ///
    /// Returns `None` if the key is unknown, the Ward layer is insufficient,
    /// or the underlying computation failed (e.g. I/O error).
    pub fn resolve(&self, key: &str) -> Option<&str> {
        // ── 1. Static map ────────────────────────────────────────────────────
        if let Some(v) = self.variables.get(key) {
            return Some(v.as_str());
        }

        // ── 2. Lazy / content-derived keys (Signet Guard) ────────────────────
        match key {
            "content_sha256" => {
                if !self.integrity_scan {
                    // Signet Guard: Layer 2 not granted for this Ward.
                    return None;
                }
                self.sha256_cache
                    .get_or_init(|| {
                        self.source_path
                            .as_ref()
                            .and_then(|p| compute_sha256(p))
                    })
                    .as_deref()
            }
            "content_mime" => {
                if !self.integrity_scan {
                    return None;
                }
                self.mime_cache
                    .get_or_init(|| {
                        self.source_path
                            .as_ref()
                            .and_then(|p| compute_mime(p))
                    })
                    .as_deref()
            }
            _ => None,
        }
    }
}

// ── Lazy Content Helpers (vigil-deep) ─────────────────────────────────────────

/// Compute the SHA-256 hex digest of the file at `path`.
///
/// Reads the entire file into memory. For very large files the caller should
/// ensure this is invoked from a blocking context (the Runner already does so
/// inside `tokio::task::spawn_blocking` where necessary).
///
/// Returns `None` on any I/O error.
#[cfg(feature = "vigil-deep")]
fn compute_sha256(path: &PathBuf) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let hash = Sha256::digest(&bytes);
    Some(format!("{:x}", hash))
}

/// Stub when `vigil-deep` is not compiled in — always returns `None`.
#[cfg(not(feature = "vigil-deep"))]
fn compute_sha256(_path: &PathBuf) -> Option<String> {
    None
}

/// Detect the MIME type of `path` by inspecting its magic bytes.
///
/// Returns `None` on I/O error or unknown format.
#[cfg(feature = "vigil-deep")]
fn compute_mime(path: &PathBuf) -> Option<String> {
    // Read just the first 512 bytes — sufficient for magic-byte detection.
    use std::io::Read;
    let mut buf = [0u8; 512];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    infer::get(&buf[..n]).map(|t| t.mime_type().to_string())
}

/// Stub when `vigil-deep` is not compiled in — always returns `None`.
#[cfg(not(feature = "vigil-deep"))]
fn compute_mime(_path: &PathBuf) -> Option<String> {
    None
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

// ── Orchestrator → Runner Payload ──────────────────────────────────────────

/// Payload sent from the Atlas to the mechanical Runner to start a sequence.
pub struct ExecData {
    pub nodes: Vec<OrdNode>,
    pub context: EnvContext,
    pub presence_config: PresenceConfig,
    pub ordinance_id: Option<String>,
    pub abort_rx: tokio::sync::oneshot::Receiver<()>,
}

/// A single structured log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Short category tag shown in the terminal (e.g. "ATLAS", "VIGIL", "HAND").
    pub tag: String,
    pub message: String,
    pub is_error: bool,
    /// The ID of the ordinance currently executing, if any.
    pub ordinance_id: Option<String>,
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
            tag: tag.into(),
            message: msg.into(),
            is_error,
            ordinance_id,
        });
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
