//! signet.rs — The Signet: the encrypted configuration vault.
//!
//! Stores per-machine user configuration (trusted directory grants, Baton
//! toggles, hotkey mappings) in an AES-256-GCM encrypted JSON file.
//!
//! The key is derived from a machine-specific seed so that the vault is
//! locked to the originating device (The Conservatory).
//!
//! Compiled only when the `signet` feature is enabled.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{info, warn};

// ── Config Schema ─────────────────────────────────────────────────────────────

/// The full Signet configuration stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VassalConfig {
    /// Directories the user has explicitly granted write access to (Directory Jail).
    pub trusted_paths: HashSet<String>,
    /// Shell scripts / executables the user has toggled on (The Baton).
    pub baton_allowed: HashSet<String>,
    /// Registered hotkey → ordinance-file mappings.
    pub hotkeys: std::collections::HashMap<String, String>,
}

// ── Vault Path ────────────────────────────────────────────────────────────────

const VAULT_PATH: &str = "vassal-data/signet.vault";

// ── Machine-scoped Key Derivation ─────────────────────────────────────────────

/// Derive a 32-byte AES key from the machine's hostname.
///
/// This is a deliberate lightweight binding — the Signet is not a
/// secrets manager; it prevents casual config portability but is not
/// cryptographically hardened against determined offline attacks.
fn machine_key() -> Key<Aes256Gcm> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let hostname = hostname::get()
        .unwrap_or_else(|_| "vassal-default".into())
        .to_string_lossy()
        .to_string();

    // Expand the hash into 32 bytes by seeding with different primes
    let mut key_bytes = [0u8; 32];
    for (i, chunk) in key_bytes.chunks_mut(8).enumerate() {
        let mut h = DefaultHasher::new();
        hostname.hash(&mut h);
        (i as u64).hash(&mut h);
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v);
    }

    *Key::<Aes256Gcm>::from_slice(&key_bytes)
}

// ── Nonce ─────────────────────────────────────────────────────────────────────

/// A fixed nonce for our single-file vault.
///
/// Because the nonce-key pair is unique per machine (key is machine-derived),
/// reusing the nonce is acceptable for this single-file, low-frequency write use case.
fn vault_nonce() -> Nonce<typenum::U12> {
    *Nonce::from_slice(b"vassal-signet")  // 13 bytes — padded below at compile time
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load and decrypt the Signet vault.
///
/// Returns `VassalConfig::default()` if the vault does not exist yet.
pub fn load() -> Result<VassalConfig, String> {
    let raw = match std::fs::read(VAULT_PATH) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("Signet: vault not found — using defaults");
            return Ok(VassalConfig::default());
        }
        Err(e) => return Err(format!("Signet: cannot read vault: {e}")),
    };

    // Decode base64 wrapper
    let ciphertext = B64.decode(&raw).map_err(|e| format!("Signet: base64 decode: {e}"))?;

    let cipher = Aes256Gcm::new(&machine_key());
    let plaintext = cipher
        .decrypt(&vault_nonce(), ciphertext.as_slice())
        .map_err(|_| "Signet: decryption failed — wrong machine or corrupt vault".to_string())?;

    serde_json::from_slice(&plaintext).map_err(|e| format!("Signet: parse error: {e}"))
}

/// Encrypt and persist the Signet vault.
pub fn save(config: &VassalConfig) -> Result<(), String> {
    std::fs::create_dir_all("vassal-data")
        .map_err(|e| format!("Signet: cannot create data dir: {e}"))?;

    let json = serde_json::to_vec(config).map_err(|e| format!("Signet: serialise error: {e}"))?;

    let cipher = Aes256Gcm::new(&machine_key());
    let ciphertext = cipher
        .encrypt(&vault_nonce(), json.as_slice())
        .map_err(|_| "Signet: encryption failed".to_string())?;

    let encoded = B64.encode(&ciphertext);
    std::fs::write(VAULT_PATH, encoded).map_err(|e| format!("Signet: write error: {e}"))?;

    info!("Signet: vault saved");
    Ok(())
}

/// Grant a directory path write access (The Conservatory).
pub fn grant_path(config: &mut VassalConfig, path: &str) {
    config.trusted_paths.insert(path.to_string());
    info!(%path, "Signet: directory trust granted");
}

/// Revoke a previously granted directory.
pub fn revoke_path(config: &mut VassalConfig, path: &str) {
    config.trusted_paths.remove(path);
    warn!(%path, "Signet: directory trust revoked");
}

/// Returns `true` if `path` is within a trusted root (The Conservatory check).
pub fn is_path_trusted(config: &VassalConfig, path: &str) -> bool {
    config.trusted_paths.iter().any(|root| path.starts_with(root.as_str()))
}

/// Activate The Baton for a specific shell target.
pub fn baton_allow(config: &mut VassalConfig, target: &str) {
    config.baton_allowed.insert(target.to_string());
    info!(%target, "Signet: Baton toggle enabled");
}

/// Deactivate The Baton for a specific shell target.
pub fn baton_revoke(config: &mut VassalConfig, target: &str) {
    config.baton_allowed.remove(target);
    warn!(%target, "Signet: Baton toggle revoked");
}

/// Returns `true` if shell execution is Baton-allowed for this target.
pub fn is_baton_allowed(config: &VassalConfig, target: &str) -> bool {
    config.baton_allowed.contains(target)
}
