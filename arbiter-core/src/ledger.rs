//! ledger.rs — persistence engine for user-defined configuration.
//!
//! Handles loading and saving the decree registry and ward configurations
//! to `arbiter-data/ledger.json`.

use std::fs;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use tokio::sync::mpsc;

use crate::atlas::Atlas;
use crate::filter::ArbiterFilter;
use crate::decree::{DecreeId, EnvContext, DecreeNode, Decree, PresenceConfig, Summons, WardConfig};

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
pub fn load() -> Result<ArbiterLedger, String> {
    let path = crate::signet::data_dir().join("ledger.json");
    if !path.exists() {
        info!("Ledger: file not found, using default");
        return Ok(ArbiterLedger::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("Ledger: read failed: {e}"))?;
    let ledger: ArbiterLedger = serde_json::from_str(&content).map_err(|e| format!("Ledger: parse failed: {e}"))?;

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

    info!("Ledger: loaded version {}", ledger.version);
    Ok(ledger)
}

/// Save the Arbiter ledger to disk atomically.
pub fn save(ledger: &ArbiterLedger) -> Result<(), String> {
    let path = crate::signet::data_dir().join("ledger.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Ledger: failed to create data directory: {e}"))?;
    }

    let content = serde_json::to_string_pretty(ledger).map_err(|e| format!("Ledger: serialisation failed: {e}"))?;
    
    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, content).map_err(|e| format!("Ledger: write failed: {e}"))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("Ledger: rename failed: {e}"))?;

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

    // 1. Setup Wards (File System Watchers)
    for ward in &ledger.wards {
        let stop_tx = crate::vigil::fs::spawn_watcher(ward.clone(), filter.clone(), vigil_tx.clone());
        atlas.active_watchers.insert(ward.path.to_string_lossy().to_string(), stop_tx);
    }

    // 2. Register Decrees
    for def in &ledger.decrees {
        let summons = match &def.summons {
            SummonsDef::FileCreated { ward_id, pattern, recursive: _recursive } => {
                // Find the ward to get the path
                let ward = ledger.wards.iter().find(|w| {
                    w.path.to_string_lossy() == *ward_id
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
                if !atlas.active_watchers.contains_key(&format!("proc:{name}")) {
                    info!(%name, "Ledger: spawning new process watcher");
                    let shutdown_tx = crate::vigil::sys::spawn_watcher(name.clone(), vigil_tx.clone());
                    // Store the shutdown sender alongside fs watchers using a
                    // "proc:" prefix to avoid key collisions with Ward paths.
                    atlas.active_watchers.insert(format!("proc:{name}"), shutdown_tx);
                }
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
