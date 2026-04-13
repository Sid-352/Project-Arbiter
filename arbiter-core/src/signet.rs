//! signet.rs — The Signet: encrypted config vault.
//!
//! Manages the `ArbiterConfig` — a set of "Trusted Roots" (filesystem paths)
//! and "Baton Whitelists" (allowed shell commands).
//!
//! Responsibilities:
//!   - Load and save the encrypted configuration vault.
//!   - Verify if a path or command is trusted.
//!   - Manage security permissions for mechanical actions.

use std::{
    collections::HashSet,
    path::Path,
};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ── Arbiter Configuration ───────────────────────────────────────────────────

/// The serializable, encrypted configuration state for Arbiter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArbiterConfig {
    /// A list of directory paths Arbiter is allowed to write to.
    pub trusted_paths: HashSet<String>,
    /// A list of shell commands (binary names) Arbiter is allowed to spawn.
    pub baton_allowed: HashSet<String>,
}

// ── Vault Management ─────────────────────────────────────────────────────────

/// The relative path to the encrypted configuration file.
const VAULT_PATH: &str = "arbiter-data/signet.vault";

/// Load the Arbiter configuration from disk.
///
/// If the vault does not exist, returns a default configuration.
pub fn load() -> Result<ArbiterConfig, String> {
    let path = Path::new(VAULT_PATH);
    if !path.exists() {
        info!("Signet: vault not found, using default configuration");
        return Ok(ArbiterConfig::default());
    }

    let bytes = std::fs::read(path).map_err(|e| format!("Signet: failed to read vault: {e}"))?;
    
    // In a real implementation, we would decrypt `bytes` here using a key.
    // For this prototype, we'll use a placeholder decryption and then deserialize.
    let config: ArbiterConfig = serde_json::from_slice(&bytes).map_err(|e| format!("Signet: failed to deserialize vault: {e}"))?;

    Ok(config)
}

/// Save the Arbiter configuration to disk.
pub fn save(config: &ArbiterConfig) -> Result<(), String> {
    let path = Path::new(VAULT_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Signet: failed to create data directory: {e}"))?;
    }

    let bytes = serde_json::to_vec(config).map_err(|e| format!("Signet: failed to serialize config: {e}"))?;
    
    // In a real implementation, we would encrypt `bytes` here before writing.
    std::fs::write(path, bytes).map_err(|e| format!("Signet: failed to write vault: {e}"))?;

    info!("Signet: configuration saved to vault");
    Ok(())
}

// ── Permission Helpers ───────────────────────────────────────────────────────

/// Returns `true` if the given path is within a "Trusted Root".
pub fn is_path_trusted(config: &ArbiterConfig, path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    
    // Canonicalize target path (or its parent if it doesn't exist)
    let canon_target = if path.exists() {
        std::fs::canonicalize(path).ok()
    } else {
        path.parent().and_then(|p| std::fs::canonicalize(p).ok()).map(|p| p.join(path.file_name().unwrap_or_default()))
    }.unwrap_or_else(|| path.to_path_buf());

    let target_str = canon_target.to_string_lossy();

    for root in &config.trusted_paths {
        // Also canonicalize the root for a fair comparison
        let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| Path::new(root).to_path_buf());
        if target_str.starts_with(&canon_root.to_string_lossy().as_ref()) {
            return true;
        }
    }
    warn!(?path, "Signet: path rejected — not within a Trusted Root");
    false
}

/// Returns `true` if the given command is in the "Baton Whitelist".
pub fn is_command_allowed(config: &ArbiterConfig, command: &str) -> bool {
    if config.baton_allowed.contains(command) {
        return true;
    }
    warn!(%command, "Signet: command rejected — not in Baton Whitelist");
    false
}
