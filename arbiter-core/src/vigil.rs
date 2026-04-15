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
    use globset::{Glob, GlobMatcher};

    /// Spawn a file-system watcher for the Ward described by `ward` and forward
    /// matching `FileCreated` events into `tx`.
    ///
    /// The watcher thread applies:
    ///   1. Temporary-file filter (`is_temp_file`)
    ///   2. Successive size check (`is_write_complete`)
    ///
    /// The `EnvContext` attached to each `Summons` will have `integrity_scan`
    /// set when `ward.layer == WardLayer::Analytical`, enabling the lazy
    /// SHA-256 / MIME resolver in `EnvContext::resolve`.
    ///
    /// **Policy note:** This function makes no security decisions itself —
    /// the caller (`main.rs`) is responsible for setting the correct `WardLayer`.
    ///
    /// The thread runs until `tx` is dropped.
    pub fn spawn_watcher(
        ward: WardConfig,
        filter: crate::filter::ArbiterFilter,
        tx: mpsc::Sender<Summons>,
    ) -> std::thread::JoinHandle<()> {
        let watch_path = ward.path.clone();
        let pattern = ward.pattern.clone();
        let analytical = ward.layer == WardLayer::Analytical;

        info!(%pattern, path = %watch_path.display(), analytical, "Vigil-fs: spawning watcher");

        // Pre-compile glob for performance
        let matcher: Option<GlobMatcher> = if !pattern.is_empty() {
            match Glob::new(&pattern) {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => {
                    warn!(%e, %pattern, "Vigil-fs: invalid pattern, falling back to match-all");
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

            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::Recursive) {
                warn!(%e, path = %watch_path.display(), "Vigil-fs: failed to watch path");
                return;
            }

            for result in nrx {
                match result {
                    Ok(event)
                        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) =>
                    {
                        for path in &event.paths {
                            let path_str = path.to_string_lossy().to_string();

                            if filter.is_own(&path_str) {
                                debug!(%path_str, "Vigil-fs: skipping Arbiter internal write");
                                continue;
                            }

                            if is_temp_file(&path_str) {
                                debug!(%path_str, "Vigil-fs: skipping temp file");
                                continue;
                            }

                            // Pattern filter using globset
                            if let Some(ref m) = matcher {
                                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if !m.is_match(filename) {
                                    continue;
                                }
                            }

                            if !is_write_complete(&path_str) {
                                debug!(%path_str, "Vigil-fs: write not yet complete, skipping");
                                continue;
                            }

                            let mut context = super::EnvContext::new();

                            // ── Always-present identity variables ───────────────
                            context.insert("file_path", &path_str);
                            context.insert(
                                "file_name",
                                path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                            );
                            context.insert(
                                "file_ext",
                                path.extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or(""),
                            );
                            // Trigger timestamp (when the event fired, not file mtime)
                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            context.insert("timestamp", &now_unix.to_string());

                            // ── Layer 1: Physical Attributes (OS metadata) ───────
                            // Free — no file handles opened, just stat() calls.
                            if let Ok(meta) = std::fs::metadata(path) {
                                // Size
                                let bytes = meta.len();
                                context.insert("file_size", &bytes.to_string());
                                context.insert("file_size_human", &format_bytes(bytes));

                                // Timestamps
                                if let Ok(created) = meta.created() {
                                    let unix = created
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    context.insert("file_created_unix", &unix.to_string());
                                }
                                if let Ok(modified) = meta.modified() {
                                    let unix = modified
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    // ISO 8601 using chrono
                                    let dt: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).unwrap();
                                    context.insert("file_modified_iso", &dt.to_rfc3339());
                                }

                                // Attributes
                                let readonly = meta.permissions().readonly();
                                context.insert("file_readonly", &readonly.to_string());

                                #[cfg(windows)]
                                {
                                    use std::os::windows::fs::MetadataExt;
                                    // FILE_ATTRIBUTE_HIDDEN = 0x2
                                    let hidden = (meta.file_attributes() & 0x2) != 0;
                                    context.insert("file_hidden", &hidden.to_string());
                                }
                                #[cfg(not(windows))]
                                {
                                    context.insert("file_hidden", "false");
                                }
                            }

                            // Symlink / shortcut check (lstat — does not follow links)
                            let is_link = std::fs::symlink_metadata(path)
                                .map(|m| m.file_type().is_symlink())
                                .unwrap_or(false);
                            context.insert("file_is_link", &is_link.to_string());

                            // ── Wire up the lazy resolver (Layer 2) ─────────────
                            // Store the real PathBuf so resolve() can open the file
                            // on demand. integrity_scan gates the Signet Guard.
                            context.source_path = Some(path.clone());
                            context.integrity_scan = analytical;


                            let summons = Summons::FileCreated {
                                watch_path: watch_path.clone(),
                                pattern: pattern.clone(),
                                context,
                            };

                            if is_debounced(&summons.to_registry_key()) {
                                continue;
                            }

                            if tx.blocking_send(summons).is_err() {
                                break; // Channel closed — watcher done
                            }
                        }
                    }
                    Err(e) => warn!(%e, "Vigil-fs: notify error"),
                    _ => {}
                }
            }

            info!("Vigil-fs: watcher thread exiting");
        })
    }

    /// Format a byte count into a human-readable string (KB / MB / GB).
    ///
    /// Uses 1024-based units to match Windows Explorer conventions.
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

    /// Register a global hotkey and forward `Hotkey` events into `tx`.
    ///
    /// `combo` is a string like `"Ctrl+Shift+V"` parsed by `global-hotkey`.
    /// Returns an error string if registration fails.
    pub fn register_hotkey(combo: String, tx: mpsc::Sender<Summons>) -> Result<(), String> {
        use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};

        let manager =
            GlobalHotKeyManager::new().map_err(|e| format!("HotKey manager init failed: {e:?}"))?;

        let hotkey: HotKey = combo
            .parse()
            .map_err(|e| format!("Cannot parse hotkey '{combo}': {e:?}"))?;

        manager
            .register(hotkey)
            .map_err(|e| format!("Hotkey registration failed: {e:?}"))?;

        info!(%combo, "Vigil-keys: hotkey registered");

        tokio::spawn(async move {
            loop {
                if let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
                    // Signet Guard: Only trigger on Press, ignore Release to prevent double execution
                    if event.state != global_hotkey::HotKeyState::Pressed {
                        continue;
                    }
                    debug!(?event, "Vigil-keys: hotkey fired");
                    let mut context = super::EnvContext::new();
                    context.insert("hotkey_combo", &combo);
                    context.insert(
                        "timestamp",
                        &format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
                    );
                    let summons = Summons::Hotkey {
                        combo: combo.clone(),
                        context,
                    };

                    if is_debounced(&summons.to_registry_key()) {
                        continue;
                    }

                    if tx.send(summons).await.is_err() {
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        // Keep the manager alive for the process lifetime
        std::mem::forget(manager);
        Ok(())
    }
}
