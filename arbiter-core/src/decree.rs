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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Represents screen coordinates.
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
                    "Unknown node kind: {kind}",
                    kind = raw.kind
                )))
            }
        };

        Ok(Self {
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
    pub const fn kind(&self) -> NodeKind {
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
            } => format!("FileCreated|{path}|{pattern}", path = watch_path.display()),
            #[cfg(feature = "vigil-keys")]
            Self::Hotkey { combo, .. } => format!("Hotkey|{combo}"),
            #[cfg(feature = "vigil-clipboard")]
            Self::Clipboard { .. } => "Clipboard".to_string(),
            Self::ProcessAppeared { name, .. } => format!("ProcessAppeared|{name}"),
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
    PdfPages,
    // ── Process Layer ──
    ProcessName,
    ProcessPid,
    // ── Hotkey Layer ──
    HotkeyCombo,
    // ── Clipboard Layer ──
    ClipboardContent,
}

impl EnvKey {
    pub const fn as_str(&self) -> &'static str {
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
            Self::PdfPages => "pdf_pages",
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
            "pdf_pages" => Some(Self::PdfPages),
            "process_name" => Some(Self::ProcessName),
            "process_pid" => Some(Self::ProcessPid),
            "hotkey_combo" => Some(Self::HotkeyCombo),
            "clipboard_content" => Some(Self::ClipboardContent),
            _ => None,
        }
    }

    pub const fn is_analytical(&self) -> bool {
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
                | Self::PdfPages
        )
    }
}

#[derive(Debug, Clone)]
struct ExifData {
    model: Option<String>,
    gps: Option<String>,
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
    #[serde(skip)]
    exif_cache: OnceLock<Option<ExifData>>,
    #[serde(skip)]
    pdf_pages_cache: OnceLock<Option<String>>,
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
            exif_cache: OnceLock::new(),
            pdf_pages_cache: OnceLock::new(),
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
            exif_cache: OnceLock::new(),
            pdf_pages_cache: OnceLock::new(),
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
            EnvKey::ImgModel => self
                .exif_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_exif_data))
                .as_ref()
                .and_then(|data| data.model.as_deref()),
            EnvKey::ImgGps => self
                .exif_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_exif_data))
                .as_ref()
                .and_then(|data| data.gps.as_deref()),
            EnvKey::PdfPages => self
                .pdf_pages_cache
                .get_or_init(|| self.source_path.as_ref().and_then(compute_pdf_pages))
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

    Some(format!("{hash:x}", hash = hasher.finalize()))
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_sha256(_path: &PathBuf) -> Option<String> {
    None
}

#[cfg(feature = "vigil-deep")]
fn compute_mime(path: &PathBuf) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; 8192];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if let Some(t) = infer::get(&buf) {
        return Some(t.mime_type().to_string());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let fallback = match ext.as_str() {
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "exe" => "application/vnd.microsoft.portable-executable",
        "dll" => "application/vnd.microsoft.portable-executable",
        "iso" => "application/x-iso9660-image",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    };

    Some(fallback.to_string())
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
    Some(format!("{entropy:.4}"))
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

#[cfg(feature = "vigil-deep")]
fn compute_exif_data(path: &PathBuf) -> Option<ExifData> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut offset = 2usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            offset += 1;
            continue;
        }

        let marker = bytes[offset + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }

        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length < 2 || offset + 2 + length > bytes.len() {
            break;
        }

        if marker == 0xE1
            && offset + 10 <= bytes.len()
            && &bytes[offset + 4..offset + 10] == b"Exif\0\0"
        {
            let exif = &bytes[offset + 10..offset + 2 + length];
            return parse_exif_tiff(exif);
        }

        offset += 2 + length;
    }

    None
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_exif_data(_path: &PathBuf) -> Option<ExifData> {
    None
}

#[cfg(feature = "vigil-deep")]
fn compute_pdf_pages(path: &PathBuf) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0u8; 8192];
    let mut read = file.read(&mut buffer).ok()?;
    if read < 5 || &buffer[..5] != b"%PDF-" {
        return None;
    }

    let pattern = b"/Type /Page";
    let tail_len = pattern.len().saturating_sub(1);
    let mut tail: Vec<u8> = Vec::new();
    let mut count = 0u64;

    loop {
        if read == 0 {
            break;
        }

        let mut scan_buf = Vec::with_capacity(tail.len() + read);
        scan_buf.extend_from_slice(&tail);
        scan_buf.extend_from_slice(&buffer[..read]);

        if scan_buf.len() >= pattern.len() {
            let scan_limit = scan_buf.len() - pattern.len();
            for i in 0..=scan_limit {
                if &scan_buf[i..i + pattern.len()] == pattern {
                    if scan_buf.get(i + pattern.len()) == Some(&b's') {
                        continue;
                    }
                    count += 1;
                }
            }
        }

        if scan_buf.len() >= tail_len {
            tail.clear();
            tail.extend_from_slice(&scan_buf[scan_buf.len() - tail_len..]);
        } else {
            tail = scan_buf;
        }

        read = file.read(&mut buffer).ok()?;
    }

    Some(count.to_string())
}

#[cfg(not(feature = "vigil-deep"))]
fn compute_pdf_pages(_path: &PathBuf) -> Option<String> {
    None
}

#[cfg(feature = "vigil-deep")]
fn parse_exif_tiff(exif: &[u8]) -> Option<ExifData> {
    if exif.len() < 8 {
        return None;
    }

    let le = match &exif[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let read_u16 = |data: &[u8], offset: usize| -> Option<u16> {
        let bytes = data.get(offset..offset + 2)?;
        Some(if le {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        })
    };

    let read_u32 = |data: &[u8], offset: usize| -> Option<u32> {
        let bytes = data.get(offset..offset + 4)?;
        Some(if le {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        })
    };

    let ifd0_offset = read_u32(exif, 4)? as usize;
    let ifd0_count = read_u16(exif, ifd0_offset)? as usize;
    let mut model: Option<String> = None;
    let mut gps_ifd_offset: Option<usize> = None;

    for i in 0..ifd0_count {
        let entry_offset = ifd0_offset + 2 + i * 12;
        let tag = read_u16(exif, entry_offset)?;
        let field_type = read_u16(exif, entry_offset + 2)?;
        let count = read_u32(exif, entry_offset + 4)? as usize;
        let value_offset = read_u32(exif, entry_offset + 8)? as usize;

        match tag {
            0x0110 => {
                if field_type == 2 && count > 0 {
                    let value = if count <= 4 {
                        let slice = exif.get(entry_offset + 8..entry_offset + 8 + count)?;
                        slice
                    } else {
                        exif.get(value_offset..value_offset + count)?
                    };
                    let text = String::from_utf8_lossy(value)
                        .trim_end_matches('\0')
                        .to_string();
                    if !text.is_empty() {
                        model = Some(text);
                    }
                }
            }
            0x8825 => {
                if field_type == 4 {
                    gps_ifd_offset = Some(value_offset);
                }
            }
            _ => {}
        }
    }

    let gps = gps_ifd_offset.and_then(|gps_offset| {
        let gps_count = read_u16(exif, gps_offset)? as usize;
        let mut lat_ref: Option<u8> = None;
        let mut lon_ref: Option<u8> = None;
        let mut lat: Option<(f64, f64, f64)> = None;
        let mut lon: Option<(f64, f64, f64)> = None;

        for i in 0..gps_count {
            let entry_offset = gps_offset + 2 + i * 12;
            let tag = read_u16(exif, entry_offset)?;
            let field_type = read_u16(exif, entry_offset + 2)?;
            let count = read_u32(exif, entry_offset + 4)? as usize;
            let value_offset = read_u32(exif, entry_offset + 8)? as usize;

            match tag {
                0x0001 => {
                    if field_type == 2 && count >= 2 {
                        lat_ref = exif.get(entry_offset + 8).copied();
                    }
                }
                0x0002 => {
                    if field_type == 5 && count >= 3 {
                        lat = read_gps_rationals(exif, value_offset, le);
                    }
                }
                0x0003 => {
                    if field_type == 2 && count >= 2 {
                        lon_ref = exif.get(entry_offset + 8).copied();
                    }
                }
                0x0004 => {
                    if field_type == 5 && count >= 3 {
                        lon = read_gps_rationals(exif, value_offset, le);
                    }
                }
                _ => {}
            }
        }

        let lat = lat?;
        let lon = lon?;
        let mut lat_val = lat.0 + lat.1 / 60.0 + lat.2 / 3600.0;
        let mut lon_val = lon.0 + lon.1 / 60.0 + lon.2 / 3600.0;

        if matches!(lat_ref, Some(b'S' | b's')) {
            lat_val = -lat_val;
        }
        if matches!(lon_ref, Some(b'W' | b'w')) {
            lon_val = -lon_val;
        }

        Some(format!("{lat:.6},{lon:.6}", lat = lat_val, lon = lon_val))
    });

    Some(ExifData { model, gps })
}

#[cfg(feature = "vigil-deep")]
fn read_gps_rationals(exif: &[u8], offset: usize, le: bool) -> Option<(f64, f64, f64)> {
    let mut read_u32 = |pos: usize| -> Option<u32> {
        let bytes = exif.get(pos..pos + 4)?;
        Some(if le {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        })
    };

    let mut to_f64 = |pos: usize| -> Option<f64> {
        let num = read_u32(pos)? as f64;
        let den = read_u32(pos + 4)? as f64;
        if den == 0.0 {
            return None;
        }
        Some(num / den)
    };

    let deg = to_f64(offset)?;
    let min = to_f64(offset + 8)?;
    let sec = to_f64(offset + 16)?;
    Some((deg, min, sec))
}

#[cfg(all(test, feature = "vigil-deep"))]
mod tests {
    use super::{compute_exif_data, compute_mime, compute_pdf_pages};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;

    fn write_temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        path.push(format!("arbiter_{name}_{stamp}"));
        let mut file = File::create(&path).expect("create temp file");
        file.write_all(bytes).expect("write temp file");
        path
    }

    fn build_exif_jpeg(model: &str) -> Vec<u8> {
        let model_bytes = format!("{model}\0").into_bytes();

        let mut tiff = vec![0u8; 8];
        tiff[0] = b'I';
        tiff[1] = b'I';
        tiff[2] = 0x2A;
        tiff[3] = 0x00;
        tiff[4..8].copy_from_slice(&8u32.to_le_bytes());

        let ifd0_offset = 8usize;
        let ifd0_size = 2 + 2 * 12 + 4;
        let model_offset = ifd0_offset + ifd0_size;
        let gps_ifd_offset = model_offset + model_bytes.len();
        let gps_ifd_size = 2 + 4 * 12 + 4;
        let gps_data_offset = gps_ifd_offset + gps_ifd_size;
        let lon_offset = gps_data_offset + 24;

        let total_size = gps_data_offset + 48;
        tiff.resize(total_size, 0u8);

        tiff[ifd0_offset..ifd0_offset + 2].copy_from_slice(&2u16.to_le_bytes());

        write_ifd_entry_le(&mut tiff, ifd0_offset + 2, 0x0110, 2, model_bytes.len() as u32, model_offset as u32);
        write_ifd_entry_le(&mut tiff, ifd0_offset + 14, 0x8825, 4, 1, gps_ifd_offset as u32);
        tiff[ifd0_offset + 26..ifd0_offset + 30].copy_from_slice(&0u32.to_le_bytes());

        tiff[model_offset..model_offset + model_bytes.len()].copy_from_slice(&model_bytes);

        tiff[gps_ifd_offset..gps_ifd_offset + 2].copy_from_slice(&4u16.to_le_bytes());
        write_ifd_entry_le(&mut tiff, gps_ifd_offset + 2, 0x0001, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0]));
        write_ifd_entry_le(&mut tiff, gps_ifd_offset + 14, 0x0002, 5, 3, gps_data_offset as u32);
        write_ifd_entry_le(&mut tiff, gps_ifd_offset + 26, 0x0003, 2, 2, u32::from_le_bytes([b'W', 0, 0, 0]));
        write_ifd_entry_le(&mut tiff, gps_ifd_offset + 38, 0x0004, 5, 3, lon_offset as u32);
        tiff[gps_ifd_offset + 50..gps_ifd_offset + 54].copy_from_slice(&0u32.to_le_bytes());

        write_rational_le(&mut tiff, gps_data_offset, 37, 1);
        write_rational_le(&mut tiff, gps_data_offset + 8, 48, 1);
        write_rational_le(&mut tiff, gps_data_offset + 16, 30, 1);
        write_rational_le(&mut tiff, lon_offset, 122, 1);
        write_rational_le(&mut tiff, lon_offset + 8, 24, 1);
        write_rational_le(&mut tiff, lon_offset + 16, 15, 1);

        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(&tiff);

        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        let len = (exif.len() + 2) as u16;
        jpeg.extend_from_slice(&len.to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    fn write_ifd_entry_le(buf: &mut [u8], offset: usize, tag: u16, field_type: u16, count: u32, value: u32) {
        buf[offset..offset + 2].copy_from_slice(&tag.to_le_bytes());
        buf[offset + 2..offset + 4].copy_from_slice(&field_type.to_le_bytes());
        buf[offset + 4..offset + 8].copy_from_slice(&count.to_le_bytes());
        buf[offset + 8..offset + 12].copy_from_slice(&value.to_le_bytes());
    }

    fn write_rational_le(buf: &mut [u8], offset: usize, num: u32, den: u32) {
        buf[offset..offset + 4].copy_from_slice(&num.to_le_bytes());
        buf[offset + 4..offset + 8].copy_from_slice(&den.to_le_bytes());
    }

    #[test]
    fn exif_extracts_model_and_gps() {
        let jpeg = build_exif_jpeg("TestCam 1");
        let path = write_temp_file("exif.jpg", &jpeg);
        let exif = compute_exif_data(&path).expect("exif data");
        assert_eq!(exif.model.as_deref(), Some("TestCam 1"));
        assert_eq!(exif.gps.as_deref(), Some("37.808333,-122.404167"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mime_falls_back_to_octet_stream() {
        let path = write_temp_file("mystery.bin", &[0, 1, 2, 3, 4, 5]);
        let mime = compute_mime(&path).expect("mime");
        assert_eq!(mime, "application/octet-stream");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn pdf_page_count_scans_pages() {
        let pdf = b"%PDF-1.4\n1 0 obj << /Type /Pages /Count 2 /Kids [2 0 R 3 0 R] >>\n2 0 obj << /Type /Page >>\n3 0 obj << /Type /Page >>\n%%EOF";
        let path = write_temp_file("sample.pdf", pdf);
        let pages = compute_pdf_pages(&path).expect("pages");
        assert_eq!(pages, "2");
        let _ = fs::remove_file(path);
    }
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
