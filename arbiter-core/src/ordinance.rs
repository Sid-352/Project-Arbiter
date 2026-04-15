//! ordinance.rs — The Arbiter data contract.
//!
//! Defines all pure data types for triggers, actions, sequences, and
//! I/O messaging. No logic lives here — this is the shared vocabulary
//! used by The Atlas, The Vigil, and the UI terminal.

use serde::{Deserialize, Serialize};
use tracing::warn;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{OnceLock},
    time::Instant,
};

// ── Strong ID Types ──────────────────────────────────────────────────────────

/// Unique identifier for an Ordinance (Decree).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecreeId(pub String);

impl From<&str> for DecreeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for DecreeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a Node within a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WardConfig {
    /// Unique identifier for this Ward.
    pub id: String,
    /// The absolute path of the directory to be watched.
    pub path: PathBuf,
    /// File pattern for filename matching (e.g. "*.zip"). Empty = match all.
    pub pattern: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrdNode {
    /// Stable UUID for this node, used for graph wiring.
    pub id: NodeId,
    /// Human-readable label shown in the editor.
    pub label: String,
    /// The action this node executes (serialised as a string key for the editor).
    pub internal_state: String,
    pub kind: NodeKind,
    /// Adjacency map: output-port-name → next node UUID.
    #[serde(default)]
    pub next_nodes: HashMap<String, NodeId>,
}

// ── Summons (Triggers) ────────────────────────────────────────────────────────

/// The specific signal that starts or gates a sequence — The Summons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Summons {
    /// A file matching `pattern` finished writing inside `watch_path`.
    #[cfg(feature = "vigil-fs")]
    FileCreated {
        watch_path: PathBuf,
        pattern: String,
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
                watch_path, pattern, ..
            } => format!("FileCreated|{}|{}", watch_path.display(), pattern),
            #[cfg(feature = "vigil-keys")]
            Self::Hotkey { combo, .. } => format!("Hotkey|{}", combo),
            Self::ProcessAppeared { name, .. } => format!("ProcessAppeared|{}", name),
            Self::Manual { .. } => "Manual".to_string(),
        }
    }
}

// ── Environment Keys ─────────────────────────────────────────────────────────

/// Structured keys for environment variables available in macro interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvKey {
    // ── Layer 1: Surface (Always available for file triggers) ──
    FilePath,
    FileName,
    FileExt,
    FileSize,
    FileCreated,
    // ── Layer 2: Analytical (Gated by Integrity Ward) ──
    ContentSha256,
    ContentMd5,
    ContentMime,
}

impl EnvKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FilePath => "file_path",
            Self::FileName => "file_name",
            Self::FileExt => "file_ext",
            Self::FileSize => "file_size",
            Self::FileCreated => "file_created",
            Self::ContentSha256 => "content_sha256",
            Self::ContentMd5 => "content_md5",
            Self::ContentMime => "content_mime",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "file_path" => Some(Self::FilePath),
            "file_name" => Some(Self::FileName),
            "file_ext" => Some(Self::FileExt),
            "file_size" => Some(Self::FileSize),
            "file_created" => Some(Self::FileCreated),
            "content_sha256" => Some(Self::ContentSha256),
            "content_md5" => Some(Self::ContentMd5),
            "content_mime" => Some(Self::ContentMime),
            _ => None,
        }
    }

    pub fn is_analytical(&self) -> bool {
        matches!(
            self,
            Self::ContentSha256 | Self::ContentMd5 | Self::ContentMime
        )
    }
}

// ── Environment Context ───────────────────────────────────────────────────────

/// The payload associated with a fired trigger.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnvContext {
    /// Eagerly-inserted static variables available to all ordinances.
    pub variables: HashMap<String, String>,

    /// The real `PathBuf` of the triggering file, used for lazy resolution.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,

    /// `true` when this Ward has Layer 2 (Analytical) access enabled.
    #[serde(skip)]
    pub integrity_scan: bool,

    /// Cached SHA-256 hex string.
    #[serde(skip)]
    sha256_cache: OnceLock<Option<String>>,

    /// Cached MIME type string.
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

    /// Insert a static key/value variable.
    pub fn insert(&mut self, key: &str, value: &str) {
        self.variables.insert(key.to_string(), value.to_string());
    }

    /// Resolve a variable by key, performing lazy computation if necessary.
    pub fn resolve(&self, key_str: &str) -> Option<&str> {
        if let Some(v) = self.variables.get(key_str) {
            return Some(v.as_str());
        }

        let key = EnvKey::from_str(key_str)?;

        if key.is_analytical() && !self.integrity_scan {
            warn!(key = %key_str, "Signet Guard: Analytical variable requested but Ward layer is insufficient");
            return None;
        }

        match key {
            EnvKey::ContentSha256 => {
                self.sha256_cache
                    .get_or_init(|| {
                        self.source_path
                            .as_ref()
                            .and_then(|p| compute_sha256(p))
                    })
                    .as_deref()
            }
            EnvKey::ContentMime => {
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

#[cfg(feature = "vigil-deep")]
fn compute_sha256(path: &PathBuf) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let hash = Sha256::digest(&bytes);
    Some(format!("{:x}", hash))
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_sha256(_path: &PathBuf) -> Option<String> {
    None
}

#[cfg(feature = "vigil-deep")]
fn compute_mime(path: &PathBuf) -> Option<String> {
    use std::io::Read;
    let mut buf = [0u8; 512];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    infer::get(&buf[..n]).map(|t| t.mime_type().to_string())
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_mime(_path: &PathBuf) -> Option<String> {
    None
}

// ── Run-time Events ───────────────────────────────────────────────────────────

/// Events emitted by the Atlas FSM to any listening consumers.
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// A log line to be displayed in the Terminal of Commands.
    Log(crate::protocol::LogEntry),
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
    pub ordinance_id: Option<DecreeId>,
    pub trigger_time: Instant,
    pub abort_rx: tokio::sync::oneshot::Receiver<()>,
}
