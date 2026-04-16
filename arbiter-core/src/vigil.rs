//! vigil.rs — The Vigil: signal watchers.
//!
//! Listens for OS-level triggers and dispatches them into the Atlas.
//! Each signal source is compiled only when its feature flag is active,
//! keeping the binary footprint minimal.
//!
//! Responsibilities:
//!   - Watch directory trees for file-write completion (vigil-fs).
//!   - Register and detect global hotkey combinations (vigil-keys).
//!   - Filter temporary/partial files (.tmp, .part) before dispatching.
//!   - Apply the Hibernation Guard: discard events older than 5 seconds
//!     on system wake to prevent post-sleep event floods.
//!
//! The Vigil does NOT execute actions — it only fires Summons events.

#[cfg(feature = "vigil-sys")]
pub mod sys;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use chrono::{DateTime, Utc, TimeZone};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::ordinance::{EnvContext, Summons, WardConfig, WardLayer};

lazy_static::lazy_static! {
    /// Tracks the last fire time of event signatures to prevent rapid double-execution.
    static ref COOLDOWN_MAP: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// The debounce window (milliseconds). Triggers within this window for the same
/// signature are dropped.
const DEBOUNCE_MS: u64 = 400;

/// Returns true if the event signature is currently in cooldown.
fn is_debounced(signature: &str) -> bool {
    let mut map = COOLDOWN_MAP.lock().unwrap();
    let now = Instant::now();
    
    if let Some(last_fire) = map.get(signature) {
        if now.duration_since(*last_fire).as_millis() < DEBOUNCE_MS as u128 {
            debug!(signature, "Vigil: dropping debounced event");
            return true;
        }
    }
    
    map.insert(signature.to_string(), now);
    
    // Prune old entries occasionally
    if map.len() > 100 {
        map.retain(|_, v| now.duration_since(*v).as_millis() < 5000);
    }
    
    false
}

// ── Shared Event Channel ──────────────────────────────────────────────────────

/// Create a bounded channel for Vigil → Atlas event delivery.
pub fn channel(capacity: usize) -> (mpsc::Sender<Summons>, mpsc::Receiver<Summons>) {
    mpsc::channel(capacity)
}

// ── Hibernation Guard ─────────────────────────────────────────────────────────

/// Maximum age (seconds) of a queued event before it is discarded on wake.
const STALE_EVENT_THRESHOLD_SECS: u64 = 5;

/// Returns `true` if an event timestamp is too old to act upon.
///
/// Use this after a system sleep/wake cycle to drain stale queued events.
pub fn is_stale(event_age_secs: u64) -> bool {
    if event_age_secs > STALE_EVENT_THRESHOLD_SECS {
        warn!(
            event_age_secs,
            "Hibernation Guard: discarding stale Vigil event"
        );
        return true;
    }
    false
}

// ── Temporary-File Filter ─────────────────────────────────────────────────────

/// Returns `true` for in-progress download/write extensions that should be ignored.
pub fn is_temp_file(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str()),
        Some("tmp" | "part" | "crdownload" | "download")
    )
}

/// Checks whether a file has finished writing using a successive size check.
///
/// Polls the file size twice with a short delay. If the sizes match, the
/// write is considered complete (The Steady State guard).
pub fn is_write_complete(path: &str) -> bool {
    let size_a = std::fs::metadata(path).map(|m| m.len()).ok();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let size_b = std::fs::metadata(path).map(|m| m.len()).ok();
    match (size_a, size_b) {
        (Some(a), Some(b)) => {
            let stable = a == b && b > 0;
            debug!(path, size = b, stable, "Successive size check");
            stable
        }
        _ => false,
    }
}

// ── File-System Watcher (vigil-fs) ────────────────────────────────────────────

#[cfg(feature = "vigil-fs")]
pub mod fs {
    use super::*;
    use notify::{recommended_watcher, Event, EventKind, RecursiveMode, Watcher};
    use globset::GlobMatcher;
    use tokio::sync::broadcast;

    /// Spawn a file-system watcher for the Ward described by `ward`.
    /// 
    /// Returns a broadcast sender that can be used to signal the watcher to shutdown.
    pub fn spawn_watcher(
        ward: WardConfig,
        filter: crate::filter::ArbiterFilter,
        tx: mpsc::Sender<Summons>,
    ) -> broadcast::Sender<()> {
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        let watch_path = ward.path.clone();
        let pattern = ward.pattern.clone();
        let analytical = ward.layer == WardLayer::Analytical;
        let recursive = ward.recursive;
        let ward_id = ward.id.clone();

        info!(%pattern, path = %watch_path.display(), analytical, recursive, "Vigil-fs: spawning watcher");

        // Pre-compile glob
        let matcher: Option<GlobMatcher> = if !pattern.is_empty() {
            match globset::GlobBuilder::new(&pattern)
                .case_insensitive(true)
                .build()
            {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => {
                    warn!(%e, %pattern, "Vigil-fs: invalid pattern");
                    None
                }
            }
        } else {
            None
        };

        std::thread::spawn(move || {
            let (ntx, nrx) = std::sync::mpsc::channel::<notify::Result<Event>>();
            let mut watcher = match recommended_watcher(ntx) {
                Ok(w) => w,
                Err(e) => {
                    warn!(%e, "Vigil-fs: failed to initialise watcher");
                    return;
                }
            };

            let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            if let Err(e) = watcher.watch(&watch_path, mode) {
                warn!(%e, path = %watch_path.display(), ?mode, "Vigil-fs: failed to watch path");
                return;
            }

            loop {
                // Check for shutdown signal
                if let Ok(_) = shutdown_rx.try_recv() {
                    info!(%ward_id, "Vigil-fs: shutdown signal received, terminating watcher");
                    break;
                }

                // Drain notify events with a short timeout to keep checking shutdown_rx
                match nrx.try_recv() {
                    Ok(Ok(event)) if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) => {
                        // Load signet config FRESH for every event batch to respect live Jailing efficiently
                        let signet_config = crate::signet::load().unwrap_or_default();

                        for path in &event.paths {
                            let path_str = path.to_string_lossy().to_string();
                            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                            // 1. Ignore directories and internal writes immediately
                            if path.is_dir() || filename.is_empty() || filter.is_own(&path_str) {
                                continue;
                            }

                            // 2. Recursion Allowed/Denied (Jail) Check
                            if crate::signet::is_path_restricted(&signet_config, path) {
                                continue; // Authoritative WARN is handled inside signet::is_path_restricted
                            }

                            if is_temp_file(&path_str) { continue; }


                            if let Some(ref m) = matcher {
                                if !m.is_match(filename) { continue; }
                            }

                            if !is_write_complete(&path_str) { continue; }

                            let mut context = super::EnvContext::new();
                            
                            // ── Level 0: Identity ─────────────────────────────────────
                            context.insert("file_path", &path_str);
                            if let Some(parent) = path.parent() {
                                context.insert("file_dir", &parent.to_string_lossy());
                            }
                            context.insert("file_name", filename);
                            context.insert("file_ext", path.extension().and_then(|e| e.to_str()).unwrap_or(""));
                            
                            let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                            context.insert("timestamp", &now_unix.to_string());
                            context.insert("timestamp_local", &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string());

                            // ── Level 1: Physical Attributes (OS metadata) ─────────────
                            // Free — no file handles opened, just stat() calls.
                            if let Ok(meta) = std::fs::metadata(path) {
                                let bytes = meta.len();
                                context.insert("file_size", &bytes.to_string());
                                context.insert("file_size_human", &format_bytes(bytes));

                                if let Ok(created) = meta.created() {
                                    let unix = created.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                                    context.insert("file_created_unix", &unix.to_string());
                                    let dt: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).unwrap();
                                    context.insert("file_created_iso", &dt.to_rfc3339());
                                    context.insert("file_created_local", &dt.with_timezone(&chrono::Local).format("%m/%d/%Y %I:%M %p").to_string());
                                }
                                if let Ok(modified) = meta.modified() {
                                    let unix = modified.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                                    let dt: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).unwrap();
                                    context.insert("file_modified_iso", &dt.to_rfc3339());
                                    context.insert("file_modified_local", &dt.with_timezone(&chrono::Local).format("%m/%d/%Y %I:%M %p").to_string());
                                }

                                context.insert("file_readonly", &meta.permissions().readonly().to_string());
                                
                                #[cfg(windows)]
                                {
                                    use std::os::windows::fs::MetadataExt;
                                    context.insert("file_hidden", &((meta.file_attributes() & 0x2) != 0).to_string());
                                }
                            }

                            let is_link = std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
                            context.insert("file_is_link", &is_link.to_string());

                            // ── Level 2: Deep Vigil hook (Analytical) ──────────────────
                            context.source_path = Some(path.clone());
                            context.integrity_scan = analytical;

                            let summons = Summons::FileCreated {
                                watch_path: watch_path.clone(),
                                pattern: pattern.clone(),
                                context,
                            };

                            // Debounce per-file to prevent rapid double-triggers for the same physical artifact
                            let debounce_sig = format!("{}|{}", summons.to_registry_key(), filename);
                            if is_debounced(&debounce_sig) { continue; }

                            if tx.blocking_send(summons).is_err() {
                                return;
                            }
                        }
                    }
                    Ok(Err(e)) => warn!(%e, "Vigil-fs: notify error"),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    _ => {}
                }
            }
        });

        shutdown_tx
    }

    /// Format a byte count into a human-readable string (KB / MB / GB).
    fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1_024;
        const MB: u64 = 1_024 * KB;
        const GB: u64 = 1_024 * MB;
        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

} // end pub mod fs

// ── Global Hotkey Watcher (vigil-keys) ───────────────────────────────────────

#[cfg(feature = "vigil-keys")]
pub mod keys {
    use super::*;

    pub fn register_hotkey(combo: String, tx: mpsc::Sender<Summons>) -> Result<(), String> {
        use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};

        let manager = GlobalHotKeyManager::new().map_err(|e| format!("HotKey manager init failed: {e:?}"))?;
        let hotkey: HotKey = combo.parse().map_err(|e| format!("Cannot parse hotkey '{combo}': {e:?}"))?;
        manager.register(hotkey).map_err(|e| format!("Hotkey registration failed: {e:?}"))?;

        info!(%combo, "Vigil-keys: hotkey registered");

        tokio::spawn(async move {
            loop {
                if let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state != global_hotkey::HotKeyState::Pressed {
                        continue;
                    }
                    let mut context = super::EnvContext::new();
                    context.insert("hotkey_combo", &combo);
                    context.insert("timestamp", &format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
                    context.insert("timestamp_local", &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string());
                    let summons = Summons::Hotkey {
                        combo: combo.clone(),
                        context,
                    };

                    if is_debounced(&summons.to_registry_key()) { continue; }
                    if tx.send(summons).await.is_err() { break; }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        std::mem::forget(manager);
        Ok(())
    }
}
