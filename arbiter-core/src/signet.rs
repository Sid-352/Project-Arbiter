use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use globset::{Glob, GlobMatcher};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbiterConfig {
    pub trusted_paths: HashSet<String>,
    pub restricted_paths: HashSet<String>,
    pub baton_allowed: HashSet<String>,
    #[serde(default)]
    pub launch_on_startup: bool,
}

impl Default for ArbiterConfig {
    fn default() -> Self {
        let mut trusted_paths = HashSet::new();
        if let Some(dl) = dirs::download_dir() {
            trusted_paths.insert(dl.to_string_lossy().to_string());
        }
        if let Some(desktop) = dirs::desktop_dir() {
            trusted_paths.insert(desktop.to_string_lossy().to_string());
        }

        // Baton whitelists are empty by default for security.
        let baton_allowed = HashSet::new();

        Self {
            trusted_paths,
            restricted_paths: HashSet::new(),
            baton_allowed,
            launch_on_startup: false,
        }
    }
}

lazy_static! {
    static ref CONFIG_CACHE: Arc<RwLock<Option<ArbiterConfig>>> = Arc::new(RwLock::new(None));
    static ref GLOB_CACHE: Arc<RwLock<Vec<GlobMatcher>>> = Arc::new(RwLock::new(Vec::new()));
}

// Using current_exe() means the data directory is always found next to the binary.
pub fn data_dir() -> PathBuf {
    // 1. Try environment override (e.g. from cargo run)
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest);
        // If we are in a crate directory, look for arbiter-data in the parent (workspace root)
        if p.ends_with("arbiter-core")
            || p.ends_with("arbiter-app")
            || p.ends_with("arbiter-forge")
            || p.ends_with("arbiter-bridge")
            || p.ends_with("arbiter-inquisitor")
        {
            if let Some(root) = p.parent() {
                let data = root.join("arbiter-data");
                if data.exists() {
                    return data;
                }
            }
        }
        let data = p.join("arbiter-data");
        if data.exists() {
            return data;
        }
    }

    // 2. Try relative to executable (Production/Portable mode)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let mut current = Some(parent);
            while let Some(p) = current {
                let data = p.join("arbiter-data");
                if data.exists() {
                    return data;
                }
                current = p.parent();
                // Stop at drive root
                if p.to_string_lossy().len() <= 3 {
                    break;
                }
            }
        }
    }

    // 3. Absolute fallback to CWD
    PathBuf::from("arbiter-data")
}

#[cfg(windows)]
fn protect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    unsafe {
        CryptProtectData(
            &data_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
        .map_err(|e| format!("DPAPI Protect failed: {}", e))?;

        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        let vec = slice.to_vec();
        let _ = LocalFree(windows::Win32::Foundation::HLOCAL(data_out.pbData as _));
        Ok(vec)
    }
}

#[cfg(windows)]
fn unprotect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    unsafe {
        CryptUnprotectData(
            &data_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
        .map_err(|e| format!("DPAPI Unprotect failed: {}", e))?;

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

    let config: ArbiterConfig = rmp_serde::from_slice(&dec_bytes).unwrap_or_else(|e| {
        warn!("Signet: failed to deserialize decrypted vault, maybe corrupted or key changed? {e}");
        ArbiterConfig::default()
    });

    // Update cache
    let _ = CONFIG_CACHE.write().map(|mut c| *c = Some(config.clone()));
    rebuild_glob_cache(&config);

    Ok(config)
}

pub fn save(config: &ArbiterConfig) -> Result<(), String> {
    let path = data_dir().join("arbiter.vault");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Signet: failed to create data directory: {e}"))?;
    }

    let bytes = rmp_serde::to_vec_named(config)
        .map_err(|e| format!("Signet: failed to serialize config: {e}"))?;
    let enc_bytes = protect_data(&bytes)?;
    std::fs::write(path, enc_bytes).map_err(|e| format!("Signet: failed to write vault: {e}"))?;

    // Update cache
    let _ = CONFIG_CACHE.write().map(|mut c| *c = Some(config.clone()));
    rebuild_glob_cache(config);

    info!("Signet: configuration saved to vault");
    Ok(())
}

pub fn reload_cache() {
    let _ = CONFIG_CACHE.write().map(|mut c| *c = None);
    // Also clear the glob cache so it is rebuilt on the next load() call.
    let _ = GLOB_CACHE.write().map(|mut g| g.clear());
}

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

fn path_matches_rules(path: &Path, rules: &HashSet<String>) -> bool {
    let canon_path = secure_canonicalize(path);

    for rule in rules {
        // 1. Try exact/prefix match (canonicalized)
        let canon_rule =
            std::fs::canonicalize(rule).unwrap_or_else(|_| Path::new(rule).to_path_buf());
        if canon_path.starts_with(&canon_rule) {
            return true;
        }

        // 2. Wildcard rules: use pre-compiled matchers from GLOB_CACHE.
        //    We still iterate `rules` for the prefix check above, but for
        //    glob matching we defer to the cache to avoid recompiling per call.
    }

    // 3. Check pre-compiled glob cache (built from restricted_paths at config load/save).
    if let Ok(matchers) = GLOB_CACHE.read() {
        let path_str = canon_path.to_string_lossy();
        for matcher in matchers.iter() {
            if matcher.is_match(&*path_str) {
                return true;
            }
        }
    }

    false
}

pub fn is_path_trusted(config: &ArbiterConfig, path: impl AsRef<Path>) -> bool {
    if path_matches_rules(path.as_ref(), &config.trusted_paths) {
        return true;
    }
    warn!(path = ?path.as_ref(), "Signet: path rejected — not within a Trusted Root");
    false
}

pub fn is_command_allowed(config: &ArbiterConfig, command: &str) -> bool {
    if config.baton_allowed.contains(command) {
        return true;
    }
    warn!(%command, "Signet: command rejected — not in Baton Whitelist");
    false
}

pub fn is_path_restricted(config: &ArbiterConfig, path: impl AsRef<Path>) -> bool {
    if path_matches_rules(path.as_ref(), &config.restricted_paths) {
        warn!(path = ?path.as_ref(), "Signet: path rejected — within a Restricted Zone (Jail)");
        return true;
    }
    false
}

pub fn sync_startup_registry(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY_CURRENT_USER,
            KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
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

        unsafe {
            let _ = RegCloseKey(hkey);
        }

        if result.is_err() && result.0 != 2 {
            // 2 = ERROR_FILE_NOT_FOUND, which is fine when deleting
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
