use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::OnceLock, time::Instant};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
/// Unique identifier for a decree.
pub struct DecreeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
/// Unique identifier for a decree node.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Represents supported automation actions.
pub enum ActionType {
    Click,
    DoubleClick,
    RightClick,
    Type(String),
    Scroll(i32),
    Navigate(String),
    Wait(u64),
    InscribeMove {
        source: PathBuf,
        destination: PathBuf,
    },
    InscribeCopy {
        source: PathBuf,
        destination: PathBuf,
    },
    InscribeDelete {
        target: PathBuf,
    },
    Shell {
        command: String,
        args: Vec<String>,
        detached: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Represents screen coordinates.
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Represents an executable automation action.
pub struct Action {
    pub action_type: ActionType,
    pub point: Option<Point>,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Configuration for user presence detection.
pub struct PresenceConfig {
    pub ignore_mouse: bool,
    pub ignore_keyboard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
/// Configuration for filesystem monitoring wards.
pub enum WardLayer {
    #[default]
    Surface,
    Analytical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Configuration for filesystem monitoring wards.
pub struct WardConfig {
    pub id: String,
    pub path: PathBuf,
    pub pattern: String,
    pub layer: WardLayer,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Represents a workflow decree definition.
pub struct Decree {
    pub nodes: Vec<DecreeNode>,
    pub presence_config: PresenceConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Defines the type of decree node.
pub enum NodeKind {
    Entry,
    Action,
    Trigger,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
/// Represents the runtime state of a decree node.
pub enum NodeState {
    #[serde(rename = "Action")]
    Action {
        action_type: ActionType,
        point: Option<Point>,
        delay_ms: u64,
    },
    #[serde(rename = "Entry")]
    Empty,
}

#[derive(Debug, Clone)]
/// Represents a node inside a decree workflow.
pub struct DecreeNode {
    pub id: NodeId,

    pub label: String,
    pub state: NodeState,
    pub next_nodes: HashMap<String, NodeId>,
}

#[derive(Deserialize)]
struct RawDecreeNode {
    id: NodeId,
    label: String,
    kind: String,
    action_type: Option<ActionType>,
    point: Option<Point>,
    #[serde(default)]
    delay_ms: u64,
    #[serde(default)]
    next_nodes: HashMap<String, NodeId>,
}

impl<'de> serde::Deserialize<'de> for DecreeNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawDecreeNode::deserialize(deserializer)?;
        let state = match raw.kind.as_str() {
            "Action" => {
                let action_type = raw.action_type.ok_or_else(|| {
                    serde::de::Error::custom("Missing 'action_type' for Action node")
                })?;
                NodeState::Action {
                    action_type,
                    point: raw.point,
                    delay_ms: raw.delay_ms,
                }
            }
            "Entry" => NodeState::Empty,
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "Unknown node kind: {}",
                    raw.kind
                )))
            }
        };

        Ok(DecreeNode {
            id: raw.id,
            label: raw.label,
            state,
            next_nodes: raw.next_nodes,
        })
    }
}

impl serde::Serialize for DecreeNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let field_count = match &self.state {
            NodeState::Action { .. } => 7,
            NodeState::Empty => 4,
        };
        let mut s = serializer.serialize_struct("DecreeNode", field_count)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("label", &self.label)?;
        s.serialize_field("next_nodes", &self.next_nodes)?;

        match &self.state {
            NodeState::Action {
                action_type,
                point,
                delay_ms,
            } => {
                s.serialize_field("kind", "Action")?;
                s.serialize_field("action_type", action_type)?;
                s.serialize_field("point", point)?;
                s.serialize_field("delay_ms", delay_ms)?;
            }
            NodeState::Empty => {
                s.serialize_field("kind", "Entry")?;
            }
        }
        s.end()
    }
}

impl DecreeNode {
    pub fn kind(&self) -> NodeKind {
        match self.state {
            NodeState::Action { .. } => NodeKind::Action,
            NodeState::Empty => NodeKind::Entry,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Represents external triggers capable of invoking decree execution.
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
    /// Clipboard content changed.
    #[cfg(feature = "vigil-clipboard")]
    Clipboard { context: EnvContext },
    /// Manual trigger (used for testing and UI-triggered runs).
    Manual { context: EnvContext },
}

impl Summons {
    pub fn to_registry_key(&self) -> String {
        match self {
            #[cfg(feature = "vigil-fs")]
            Self::FileCreated {
                watch_path,
                pattern,
                ..
            } => format!("FileCreated|{}|{}", watch_path.display(), pattern),
            #[cfg(feature = "vigil-keys")]
            Self::Hotkey { combo, .. } => format!("Hotkey|{}", combo),
            #[cfg(feature = "vigil-clipboard")]
            Self::Clipboard { .. } => "Clipboard".to_string(),
            Self::ProcessAppeared { name, .. } => format!("ProcessAppeared|{}", name),
            Self::Manual { .. } => "Manual".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Defines supported runtime environment variables exposed to decrees.
pub enum EnvKey {
    // ── Layer 1: Surface (Always available for file triggers) ──
    FileDir,
    FilePath,
    FileName,
    FileExt,
    FileSize,
    FileSizeHuman,
    FileReadonly,
    FileHidden,
    FileCreatedUnix,
    FileCreatedIso,
    FileCreatedLocal,
    FileModifiedIso,
    FileModifiedLocal,
    FileOwner,
    FileIsLink,
    Timestamp,
    TimestampLocal,
    // ── Layer 2: Analytical (Gated by Integrity Ward) ──
    ContentSha256,
    ContentMd5,
    ContentMime,
    ContentEntropy,
    ImgDims,
    ImgAspect,
    ImgModel,
    ImgGps,
    TextLines,
    // ── Process Layer ──
    ProcessName,
    ProcessPid,
    // ── Hotkey Layer ──
    HotkeyCombo,
    // ── Clipboard Layer ──
    ClipboardContent,
}

impl EnvKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileDir => "file_dir",
            Self::FilePath => "file_path",
            Self::FileName => "file_name",
            Self::FileExt => "file_ext",
            Self::FileSize => "file_size",
            Self::FileSizeHuman => "file_size_human",
            Self::FileReadonly => "file_readonly",
            Self::FileHidden => "file_hidden",
            Self::FileCreatedUnix => "file_created_unix",
            Self::FileCreatedIso => "file_created_iso",
            Self::FileCreatedLocal => "file_created_local",
            Self::FileModifiedIso => "file_modified_iso",
            Self::FileModifiedLocal => "file_modified_local",
            Self::FileOwner => "file_owner",
            Self::FileIsLink => "file_is_link",
            Self::Timestamp => "timestamp",
            Self::TimestampLocal => "timestamp_local",
            Self::ContentSha256 => "content_sha256",
            Self::ContentMd5 => "content_md5",
            Self::ContentMime => "content_mime",
            Self::ContentEntropy => "content_entropy",
            Self::ImgDims => "img_dims",
            Self::ImgAspect => "img_aspect",
            Self::ImgModel => "img_model",
            Self::ImgGps => "img_gps",
            Self::TextLines => "text_lines",
            Self::ProcessName => "process_name",
            Self::ProcessPid => "process_pid",
            Self::HotkeyCombo => "hotkey_combo",
            Self::ClipboardContent => "clipboard_content",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "file_dir" => Some(Self::FileDir),
            "file_path" => Some(Self::FilePath),
            "file_name" => Some(Self::FileName),
            "file_ext" => Some(Self::FileExt),
            "file_size" => Some(Self::FileSize),
            "file_size_human" => Some(Self::FileSizeHuman),
            "file_readonly" => Some(Self::FileReadonly),
            "file_hidden" => Some(Self::FileHidden),
            "file_created_unix" => Some(Self::FileCreatedUnix),
            "file_created_iso" => Some(Self::FileCreatedIso),
            "file_created_local" => Some(Self::FileCreatedLocal),
            "file_modified_iso" => Some(Self::FileModifiedIso),
            "file_modified_local" => Some(Self::FileModifiedLocal),
            "file_owner" => Some(Self::FileOwner),
            "file_is_link" => Some(Self::FileIsLink),
            "timestamp" => Some(Self::Timestamp),
            "timestamp_local" => Some(Self::TimestampLocal),
            "content_sha256" => Some(Self::ContentSha256),
            "content_md5" => Some(Self::ContentMd5),
            "content_mime" => Some(Self::ContentMime),
            "content_entropy" => Some(Self::ContentEntropy),
            "img_dims" => Some(Self::ImgDims),
            "img_aspect" => Some(Self::ImgAspect),
            "img_model" => Some(Self::ImgModel),
            "img_gps" => Some(Self::ImgGps),
            "text_lines" => Some(Self::TextLines),
            "process_name" => Some(Self::ProcessName),
            "process_pid" => Some(Self::ProcessPid),
            "hotkey_combo" => Some(Self::HotkeyCombo),
            "clipboard_content" => Some(Self::ClipboardContent),
            _ => None,
        }
    }

    pub fn is_analytical(&self) -> bool {
        matches!(
            self,
            Self::ContentSha256
                | Self::ContentMd5
                | Self::ContentMime
                | Self::ContentEntropy
                | Self::ImgDims
                | Self::ImgAspect
                | Self::ImgModel
                | Self::ImgGps
                | Self::TextLines
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
/// Stores environment variables and runtime metadata.
pub struct EnvContext {
    pub variables: HashMap<String, String>,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
    #[serde(skip)]
    pub integrity_scan: bool,
    #[serde(skip)]
    sha256_cache: OnceLock<Option<String>>,
    #[serde(skip)]
    mime_cache: OnceLock<Option<String>>,
    #[serde(skip)]
    md5_cache: OnceLock<Option<String>>,
    #[serde(skip)]
    entropy_cache: OnceLock<Option<String>>,
    #[serde(skip)]
    text_lines_cache: OnceLock<Option<String>>,
}

impl Default for EnvContext {
    fn default() -> Self {
        Self {
            variables: HashMap::new(),
            source_path: None,
            integrity_scan: false,
            sha256_cache: OnceLock::new(),
            mime_cache: OnceLock::new(),
            md5_cache: OnceLock::new(),
            entropy_cache: OnceLock::new(),
            text_lines_cache: OnceLock::new(),
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
            md5_cache: OnceLock::new(),
            entropy_cache: OnceLock::new(),
            text_lines_cache: OnceLock::new(),
        }
    }
}

impl EnvContext {
    pub fn new() -> Self {
        Self::default()
    }

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
            EnvKey::ContentSha256 => self
                .sha256_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_sha256))
                .as_deref(),
            EnvKey::ContentMime => self
                .mime_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_mime))
                .as_deref(),
            EnvKey::ContentMd5 => self
                .md5_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_md5))
                .as_deref(),
            EnvKey::ContentEntropy => self
                .entropy_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_entropy))
                .as_deref(),
            EnvKey::TextLines => self
                .text_lines_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_text_lines))
                .as_deref(),
            _ => None,
        }
    }
}

#[cfg(feature = "vigil-deep")]
fn compute_sha256(path: &PathBuf) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Some(format!("{:x}", hasher.finalize()))
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

#[cfg(feature = "vigil-deep")]
fn compute_md5(path: &PathBuf) -> Option<String> {
    use md5::{Digest, Md5};
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Md5::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Some(format!("{:x}", hasher.finalize()))
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_md5(_path: &PathBuf) -> Option<String> {
    None
}

/// Compute Shannon entropy H = -Σ p(x) * log2(p(x)) over the byte distribution.
/// Returns a 4-decimal-place string (max 8.0 for perfectly random data).
#[cfg(feature = "vigil-deep")]
fn compute_entropy(path: &PathBuf) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut freq = [0u64; 256];
    let mut buffer = [0u8; 8192];
    let mut total_len = 0u64;

    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        total_len += n as u64;
        for &b in &buffer[..n] {
            freq[b as usize] += 1;
        }
    }

    if total_len == 0 {
        return Some("0.0000".to_string());
    }

    let len_f = total_len as f64;
    let entropy: f64 = freq
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len_f;
            -p * p.log2()
        })
        .sum();
    Some(format!("{:.4}", entropy))
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_entropy(_path: &PathBuf) -> Option<String> {
    None
}

/// Count newline characters — a fast proxy for line count on text files.
#[cfg(feature = "vigil-deep")]
fn compute_text_lines(path: &PathBuf) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0u8; 8192];
    let mut count = 0usize;

    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        count += buffer[..n].iter().filter(|&&b| b == b'\n').count();
    }

    Some(count.to_string())
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_text_lines(_path: &PathBuf) -> Option<String> {
    None
}

#[derive(Debug, Clone)]
/// Represents execution events emitted by the runtime.
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
/// Contains runtime execution state passed into the orchestration engine.
pub struct ExecData {
    pub nodes: Vec<DecreeNode>,
    pub context: EnvContext,
    pub presence_config: PresenceConfig,
    pub decree_id: Option<DecreeId>,
    pub trigger_time: Instant,
    pub dry_run: bool,
    pub abort_rx: tokio::sync::oneshot::Receiver<()>,
}
