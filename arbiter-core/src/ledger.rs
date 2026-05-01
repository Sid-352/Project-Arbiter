//! ledger.rs — persistence engine for user-defined configuration.
//!
//! Handles loading and saving the decree registry and ward configurations
//! to `arbiter-data/ledger.json`.

use std::collections::{HashMap, HashSet};
use std::fs;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use tokio::sync::mpsc;

use crate::atlas::Atlas;
use crate::filter::ArbiterFilter;
use crate::decree::{DecreeId, EnvContext, DecreeNode, Decree, PresenceConfig, Summons, WardConfig};

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("Ledger: read failed: {0}")]
    ReadFailed(String),
    #[error("Ledger: parse failed: {0}")]
    ParseFailed(String),
    #[error("Ledger: serialisation failed: {0}")]
    SerializationFailed(String),
    #[error("Ledger: write failed: {0}")]
    WriteFailed(String),
    #[error("Ledger: rename failed: {0}")]
    RenameFailed(String),
    #[error("Ledger: failed to create data directory: {0}")]
    DirCreationFailed(String),
}

impl From<std::io::Error> for LedgerError {
    fn from(e: std::io::Error) -> Self {
        LedgerError::ReadFailed(e.to_string())
    }
}

fn resolve_dynamic_paths(path: &str) -> String {
    let mut expanded = path.to_string();
    
    let upper = expanded.to_uppercase();
    if upper.contains("%USERPROFILE%\\DOWNLOADS") || upper.contains("%USERPROFILE%/DOWNLOADS") {
        if let Some(dl) = dirs::download_dir() {
            let re = regex::Regex::new(r"(?i)%USERPROFILE%[\\/]Downloads").unwrap();
            expanded = re.replace_all(&expanded, dl.to_string_lossy().as_ref()).to_string();
        }
    }
    
    if upper.contains("%USERPROFILE%\\DESKTOP") || upper.contains("%USERPROFILE%/DESKTOP") {
        if let Some(desktop) = dirs::desktop_dir() {
            let re = regex::Regex::new(r"(?i)%USERPROFILE%[\\/]Desktop").unwrap();
            expanded = re.replace_all(&expanded, desktop.to_string_lossy().as_ref()).to_string();
        }
    }

    if upper.contains("%USERPROFILE%\\DOCUMENTS") || upper.contains("%USERPROFILE%/DOCUMENTS") {
        if let Some(docs) = dirs::document_dir() {
            let re = regex::Regex::new(r"(?i)%USERPROFILE%[\\/]Documents").unwrap();
            expanded = re.replace_all(&expanded, docs.to_string_lossy().as_ref()).to_string();
        }
    }
    
    if upper.contains("%USERPROFILE%") {
        if let Some(home) = dirs::home_dir() {
            let re = regex::Regex::new(r"(?i)%USERPROFILE%").unwrap();
            expanded = re.replace_all(&expanded, home.to_string_lossy().as_ref()).to_string();
        }
    }
    
    if expanded.starts_with("~/") || expanded.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            expanded = expanded.replacen("~", &home.to_string_lossy(), 1);
        }
    }
    
    expanded
}

fn normalize_windows_path(path: &str) -> String {
    fn is_drive_root(p: &str) -> bool {
        let b = p.as_bytes();
        b.len() == 3 && b[1] == b':' && b[2] == b'\\'
    }

    let resolved = resolve_dynamic_paths(path);
    let mut out = resolved.trim().replace('/', "\\");
    while out.ends_with('\\') && !is_drive_root(&out) {
        out.pop();
    }
    out
}

fn normalize_ledger(ledger: &mut ArbiterLedger) -> bool {
    let mut changed = false;
    let mut seen_paths = HashSet::new();
    let mut id_to_path: HashMap<String, String> = HashMap::new();
    let mut path_to_path: HashMap<String, String> = HashMap::new();
    let mut normalized_wards = Vec::new();

    for ward in &ledger.wards {
        let normalized_path = normalize_windows_path(&ward.path.to_string_lossy());
        let normalized_id = normalize_windows_path(&ward.id);
        id_to_path.insert(ward.id.clone(), normalized_path.clone());
        id_to_path.insert(normalized_id.clone(), normalized_path.clone());
        path_to_path.insert(normalize_windows_path(&ward.path.to_string_lossy()), normalized_path.clone());

        if seen_paths.insert(normalized_path.clone()) {
            let mut ward_out = ward.clone();
            if ward_out.id != normalized_path {
                ward_out.id = normalized_path.clone();
                changed = true;
            }
            if ward_out.path.to_string_lossy() != normalized_path {
                ward_out.path = normalized_path.clone().into();
                changed = true;
            }
            normalized_wards.push(ward_out);
        } else {
            changed = true;
        }
    }

    for decree in &mut ledger.decrees {
        if let SummonsDef::FileCreated { ward_id, .. } = &mut decree.summons {
            let normalized = id_to_path
                .get(ward_id)
                .cloned()
                .or_else(|| path_to_path.get(&normalize_windows_path(ward_id)).cloned())
                .unwrap_or_else(|| normalize_windows_path(ward_id));
            if *ward_id != normalized {
                *ward_id = normalized;
                changed = true;
            }
        }
    }

    if ledger.wards.len() != normalized_wards.len() {
        changed = true;
    }
    ledger.wards = normalized_wards;
    changed
}

// ── Persistence Structures ───────────────────────────────────────────────────

/// The complete on-disk representation of a user's configuration.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ArbiterLedger {
    pub version: u32,
    pub wards: Vec<WardConfig>,
    pub decrees: Vec<DecreeDef>,
}

/// A named, serializable decree definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecreeDef {
    pub id: DecreeId,
    pub label: String,
    pub summons: SummonsDef,
    pub nodes: Vec<DecreeNode>,
    pub presence_config: PresenceConfig,
}

impl DecreeDef {
    /// Validates the structural integrity of the decree sequence.
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("Decree sequence is empty".into());
        }

        let mut has_entry = false;
        let mut node_ids = std::collections::HashSet::new();

        for node in &self.nodes {
            node_ids.insert(&node.id);
            if node.kind == crate::decree::NodeKind::Entry {
                has_entry = true;
            }
        }

        if !has_entry {
            return Err("Decree sequence is missing an Entry node".into());
        }

        // Check for orphaned transitions
        for node in &self.nodes {
            for (port, target_id) in &node.next_nodes {
                if !node_ids.contains(target_id) {
                    return Err(format!(
                        "Node '{}' transition '{}' points to non-existent node '{}'",
                        node.label, port, target_id
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Serializable trigger definition (mirrors Summons but without runtime fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum SummonsDef {
    FileCreated {
        ward_id: String,
        pattern: String,
        #[serde(default = "default_recursive")]
        recursive: bool,
    },
    Hotkey {
        combo: String,
    },
    ProcessAppeared {
        name: String,
    },
    Manual,
}

fn default_recursive() -> bool {
    true
}

// ── I/O Operations ───────────────────────────────────────────────────────────

/// Load the Arbiter ledger from disk.
pub fn load() -> Result<ArbiterLedger, LedgerError> {
    let path = crate::signet::data_dir().join("ledger.toml");
    if !path.exists() {
        info!("Ledger: file not found, using default");
        return Ok(ArbiterLedger::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| LedgerError::ReadFailed(e.to_string()))?;
    let mut ledger: ArbiterLedger = toml::from_str(&content).map_err(|e| LedgerError::ParseFailed(e.to_string()))?;
    let migrated = normalize_ledger(&mut ledger);

    // Warn if the on-disk format version doesn't match what this build expects.
    // This catches silent schema corruption after an upgrade.
    const CURRENT_VERSION: u32 = 1;
    if ledger.version != 0 && ledger.version != CURRENT_VERSION {
        warn!(
            on_disk = ledger.version,
            expected = CURRENT_VERSION,
            "Ledger: schema version mismatch — some fields may be missing or ignored"
        );
    }

    if migrated {
        if let Err(e) = save(&ledger) {
            warn!(%e, "Ledger: failed to persist migrated normalization changes");
        } else {
            info!("Ledger: normalized legacy path variants and duplicates");
        }
    }

    info!("Ledger: loaded version {}", ledger.version);
    Ok(ledger)
}

/// Save the Arbiter ledger to disk atomically.
pub fn save(ledger: &ArbiterLedger) -> Result<(), LedgerError> {
    let mut out = ArbiterLedger {
        version: ledger.version,
        wards: ledger.wards.clone(),
        decrees: ledger.decrees.clone(),
    };
    normalize_ledger(&mut out);

    let path = crate::signet::data_dir().join("ledger.toml");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| LedgerError::DirCreationFailed(e.to_string()))?;
    }

    let content = toml::to_string_pretty(&out).map_err(|e| LedgerError::SerializationFailed(e.to_string()))?;
    
    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, content).map_err(|e| LedgerError::WriteFailed(e.to_string()))?;
    fs::rename(&tmp_path, path).map_err(|e| LedgerError::RenameFailed(e.to_string()))?;

    info!("Ledger: configuration saved to disk");
    Ok(())
}

// ── Application ───────────────────────────────────────────────────────────────

/// Apply the ledger configuration to the running engine.
///
/// Wires loaded decrees into the Atlas and spawns watchers for Wards.
pub fn apply(
    ledger: &ArbiterLedger,
    atlas: &mut Atlas,
    vigil_tx: &mpsc::Sender<Summons>,
    filter: &ArbiterFilter,
) {
    info!("Ledger: applying configuration to engine");

    let mut unique_wards = std::collections::HashSet::new();
    let referenced_ward_ids: std::collections::HashSet<String> = ledger
        .decrees
        .iter()
        .filter_map(|d| match &d.summons {
            SummonsDef::FileCreated { ward_id, .. } => Some(normalize_windows_path(ward_id)),
            _ => None,
        })
        .collect();

    // 1. Setup Wards (File System Watchers)
    for ward in &ledger.wards {
        let normalized = normalize_windows_path(&ward.path.to_string_lossy());
        if !referenced_ward_ids.contains(&normalized) {
            debug!(path = %normalized, "Ledger: skipping unreferenced ward watcher");
            continue;
        }
        if !unique_wards.insert(normalized.clone()) {
            continue;
        }
        let mut normalized_ward = ward.clone();
        normalized_ward.id = normalized.clone();
        normalized_ward.path = std::path::PathBuf::from(&normalized);
        let stop_tx = crate::vigil::fs::spawn_watcher(normalized_ward, filter.clone(), vigil_tx.clone());
        atlas.active_watchers.insert(normalized, stop_tx);
    }

    // 2. Register Decrees
    for def in &ledger.decrees {
        let summons = match &def.summons {
            SummonsDef::FileCreated { ward_id, pattern, recursive: _recursive } => {
                let normalized_ward_id = normalize_windows_path(ward_id);
                // Find the ward to get the path
                let ward = ledger.wards.iter().find(|w| {
                    normalize_windows_path(&w.path.to_string_lossy()) == normalized_ward_id
                });

                if let Some(w) = ward {
                    Summons::FileCreated {
                        watch_path: w.path.clone(),
                        pattern: pattern.clone(),
                        context: EnvContext::new(),
                    }
                } else {
                    warn!(%def.id, ward_id, "Ledger: Decree ward not found, skipping");
                    continue;
                }
            }
            SummonsDef::Hotkey { combo } => {
                let _ = crate::vigil::keys::register_hotkey(combo.clone(), vigil_tx.clone());
                Summons::Hotkey {
                    combo: combo.clone(),
                    context: EnvContext::new(),
                }
            }
            SummonsDef::ProcessAppeared { name } => {
                atlas.active_watchers.entry(format!("proc:{name}")).or_insert_with(|| {
                    info!(%name, "Ledger: spawning new process watcher");
                    // Store the shutdown sender alongside fs watchers using a
                    // "proc:" prefix to avoid key collisions with Ward paths.
                    crate::vigil::sys::spawn_watcher(name.clone(), vigil_tx.clone())
                });
                Summons::ProcessAppeared {
                    name: name.clone(),
                    context: EnvContext::new(),
                }
            }
            SummonsDef::Manual => Summons::Manual {
                context: EnvContext::new(),
            },
        };

        let key = summons.to_registry_key();
        atlas.register_decree(
            key,
            Decree {
                nodes: def.nodes.clone(),
                presence_config: def.presence_config.clone(),
            },
        );
    }
}
