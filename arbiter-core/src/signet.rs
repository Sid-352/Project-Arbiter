//! signet.rs — encrypted config vault.
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
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use globset::{Glob, GlobMatcher};
use lazy_static::lazy_static;

// ── Arbiter Configuration ───────────────────────────────────────────────────

/// The serializable, encrypted configuration state for Arbiter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArbiterConfig {
    /// A list of directory paths Arbiter is allowed to write to.
    pub trusted_paths: HashSet<String>,
    /// A list of directory paths Arbiter is NOT allowed to trigger from.
    pub restricted_paths: HashSet<String>,
    /// A list of shell commands (binary names) Arbiter is allowed to spawn.
    pub baton_allowed: HashSet<String>,
    /// Whether the Arbiter service should launch on Windows startup.
    #[serde(default)]
    pub launch_on_startup: bool,
}

lazy_static! {
    /// Global cache of the Arbiter config to prevent excessive disk I/O in signal loops.
    static ref CONFIG_CACHE: Arc<RwLock<Option<ArbiterConfig>>> = Arc::new(RwLock::new(None));

    /// Pre-compiled glob matchers built from `ArbiterConfig::restricted_paths`.
    ///
    /// Rebuilt once per `save()` / `reload_cache()` call. After that, every
    /// Vigil event uses this cache and avoids the `Glob::new(...).compile_matcher()`
    /// call that previously ran on every single filesystem notification.
    static ref GLOB_CACHE: Arc<RwLock<Vec<GlobMatcher>>> = Arc::new(RwLock::new(Vec::new()));
}

// ── Data Directory ──────────────────────────────────────────────────────────

/// Returns the `arbiter-data` directory anchored to the directory that
/// contains the running executable.
///
/// Using `current_exe()` instead of a CWD-relative path means the data
/// directory is always found next to the binary, regardless of which working
/// directory the user (or a service manager) launches Arbiter from.
pub fn data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arbiter-data")
}

// ── DPAPI Encryption ─────────────────────────────────────────────────────────

#[cfg(windows)]
fn protect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN};
    use windows::Win32::Foundation::LocalFree;
    use std::ptr;

    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    unsafe {
        CryptProtectData(
            &mut data_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        ).map_err(|e| format!("DPAPI Protect failed: {}", e))?;

        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        let vec = slice.to_vec();
        let _ = LocalFree(windows::Win32::Foundation::HLOCAL(data_out.pbData as _));
        Ok(vec)
    }
}

#[cfg(windows)]
fn unprotect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN};
    use windows::Win32::Foundation::LocalFree;
    use std::ptr;

    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    unsafe {
        CryptUnprotectData(
            &mut data_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        ).map_err(|e| format!("DPAPI Unprotect failed: {}", e))?;

        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        let vec = slice.to_vec();
        let _ = LocalFree(windows::Win32::Foundation::HLOCAL(data_out.pbData as _));
        Ok(vec)
    }
}

#[cfg(not(windows))]
fn protect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}


/// Load the Arbiter configuration from disk.
///
/// If the vault does not exist, returns a default configuration.
pub fn load() -> Result<ArbiterConfig, String> {
    // Check cache first
    if let Ok(cache) = CONFIG_CACHE.read() {
        if let Some(config) = &*cache {
            return Ok(config.clone());
        }
    }

    let path = data_dir().join("arbiter.vault");
    if !path.exists() {
        info!("Signet: vault not found, using default configuration");
        let def = ArbiterConfig::default();
        let _ = CONFIG_CACHE.write().map(|mut c| *c = Some(def.clone()));
        return Ok(def);
    }

    let bytes = std::fs::read(&path).map_err(|e| format!("Signet: failed to read vault: {e}"))?;
    
    // DPAPI decrypt (or passthrough on non-windows)
    let dec_bytes = unprotect_data(&bytes)?;

    let config: ArbiterConfig = bincode::deserialize(&dec_bytes).unwrap_or_else(|e| {
        warn!("Signet: failed to deserialize decrypted vault, maybe corrupted or key changed? {e}");
        ArbiterConfig::default()
    });

    // Update cache
    let _ = CONFIG_CACHE.write().map(|mut c| *c = Some(config.clone()));
    rebuild_glob_cache(&config);

    Ok(config)
}

/// Save the Arbiter configuration to disk and invalidate cache.
pub fn save(config: &ArbiterConfig) -> Result<(), String> {
    let path = data_dir().join("arbiter.vault");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Signet: failed to create data directory: {e}"))?;
    }

    let bytes = bincode::serialize(config).map_err(|e| format!("Signet: failed to serialize config: {e}"))?;
    let enc_bytes = protect_data(&bytes)?;
    std::fs::write(path, enc_bytes).map_err(|e| format!("Signet: failed to write vault: {e}"))?;

    // Update cache
    let _ = CONFIG_CACHE.write().map(|mut c| *c = Some(config.clone()));
    rebuild_glob_cache(config);

    info!("Signet: configuration saved to vault");
    Ok(())
}

/// Force a reload of the configuration from disk on the next `load()` call.
pub fn reload_cache() {
    let _ = CONFIG_CACHE.write().map(|mut c| *c = None);
    // Also clear the glob cache so it is rebuilt on the next load() call.
    let _ = GLOB_CACHE.write().map(|mut g| g.clear());
}

/// Rebuild `GLOB_CACHE` from the current config's restricted path rules.
/// Called automatically by `load()` and `save()`.
fn rebuild_glob_cache(config: &ArbiterConfig) {
    let matchers: Vec<GlobMatcher> = config
        .restricted_paths
        .iter()
        .filter(|r| r.contains('*') || r.contains('?'))
        .filter_map(|r| Glob::new(r).ok())
        .map(|g| g.compile_matcher())
        .collect();
    let _ = GLOB_CACHE.write().map(|mut g| *g = matchers);
}

// ── Permission Helpers ───────────────────────────────────────────────────────

/// Helper to canonicalize a path for security checks.
fn secure_canonicalize(path: &Path) -> PathBuf {
    if path.exists() {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .map(|p| p.join(path.file_name().unwrap_or_default()))
            .unwrap_or_else(|| path.to_path_buf())
    }
}

/// Helper to check if a path matches a set of rules (supports globs and prefixes).
/// Uses pre-compiled `GLOB_CACHE` for wildcard rules to avoid per-call recompilation.
fn path_matches_rules(path: &Path, rules: &HashSet<String>) -> bool {
    let canon_path = secure_canonicalize(path);
    let path_str = canon_path.to_string_lossy();

    for rule in rules {
        // 1. Try exact/prefix match (canonicalized)
        let canon_rule = std::fs::canonicalize(rule).unwrap_or_else(|_| Path::new(rule).to_path_buf());
        if path_str.starts_with(canon_rule.to_string_lossy().as_ref()) {
            return true;
        }

        // 2. Wildcard rules: use pre-compiled matchers from GLOB_CACHE.
        //    We still iterate `rules` for the prefix check above, but for
        //    glob matching we defer to the cache to avoid recompiling per call.
    }

    // 3. Check pre-compiled glob cache (built from restricted_paths at config load/save).
    if let Ok(matchers) = GLOB_CACHE.read() {
        for matcher in matchers.iter() {
            if matcher.is_match(&*path_str) {
                return true;
            }
        }
    }

    false
}

/// Returns `true` if the given path is within a "Trusted Root".
pub fn is_path_trusted(config: &ArbiterConfig, path: impl AsRef<Path>) -> bool {
    if path_matches_rules(path.as_ref(), &config.trusted_paths) {
        return true;
    }
    warn!(path = ?path.as_ref(), "Signet: path rejected — not within a Trusted Root");
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

/// Returns `true` if the given path is within a "Restricted Zone" (Path Jailing).
pub fn is_path_restricted(config: &ArbiterConfig, path: impl AsRef<Path>) -> bool {
    if path_matches_rules(path.as_ref(), &config.restricted_paths) {
        warn!(path = ?path.as_ref(), "Signet: path rejected — within a Restricted Zone (Jail)");
        return true;
    }
    false
}

// ── Windows Startup Registry ────────────────────────────────────────────────

/// Synchronizes the "Launch on Startup" state with the Windows Registry.
///
/// Uses `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
pub fn sync_startup_registry(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY_CURRENT_USER,
            REG_SZ, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        };

        let sub_key = HSTRING::from("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let value_name = HSTRING::from("Arbiter");

        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &sub_key,
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
        };

        if status.is_err() {
            return Err(format!("Signet: failed to open registry key: {:?}", status));
        }

        let result = if enabled {
            let exe_path = std::env::current_exe()
                .map_err(|e| format!("Signet: failed to get current exe path: {e}"))?;
            
            // On Windows, current_exe might be arbiter-forge.exe if we're in the UI,
            // but we want arbiter.exe (the background service) to start.
            // If the current exe is arbiter-forge.exe, we look for arbiter.exe in the same dir.
            let mut startup_path = exe_path.clone();
            if let Some(name) = exe_path.file_name() {
                if name == "arbiter-forge.exe" {
                    startup_path = exe_path.parent().unwrap().join("arbiter.exe");
                }
            }

            let path_str = startup_path.to_string_lossy();
            let path_hstring = HSTRING::from(path_str.as_ref());
            
            info!(path = %path_str, "Signet: registering Arbiter for startup");
            unsafe {
                RegSetValueExW(
                    hkey,
                    &value_name,
                    0,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(
                        path_hstring.as_ptr() as *const u8,
                        (path_hstring.len() * 2) + 2,
                    )),
                )
            }
        } else {
            info!("Signet: removing Arbiter from startup registry");
            unsafe { RegDeleteValueW(hkey, &value_name) }
        };

        unsafe { let _ = RegCloseKey(hkey); }

        if result.is_err() && result.0 != 2 { // 2 = ERROR_FILE_NOT_FOUND, which is fine when deleting
            return Err(format!("Signet: registry operation failed: {:?}", result));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Ok(())
    }
}
