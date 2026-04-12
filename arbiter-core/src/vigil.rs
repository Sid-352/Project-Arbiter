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

use crate::ordinance::{EnvContext, Summons, WardConfig, WardLayer};

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
        let glob = ward.glob.clone();
        let analytical = ward.layer == WardLayer::Analytical;

        info!(%glob, path = %watch_path.display(), analytical, "Vigil-fs: spawning watcher");

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

                            // Glob filter — simple filename match
                            if !glob.is_empty() {
                                let filename =
                                    path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if !matches_glob(&glob, filename) {
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
                                    // ISO 8601 (UTC, seconds precision)
                                    let iso = unix_to_iso8601(unix);
                                    context.insert("file_modified_iso", &iso);
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
                                glob: glob.clone(),
                                context,
                            };                            if tx.blocking_send(summons).is_err() {
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

    /// Minimal glob: supports `*` as wildcard, case-insensitive on Windows.
    fn matches_glob(glob: &str, name: &str) -> bool {
        let glob = glob.to_lowercase();
        let name = name.to_lowercase();
        if let Some((prefix, suffix)) = glob.split_once('*') {
            name.starts_with(prefix) && name.ends_with(suffix)
        } else {
            glob == name
        }
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

    /// Convert a Unix timestamp (seconds since epoch) to an ISO 8601 UTC string.
    ///
    /// Produces the format `YYYY-MM-DDTHH:MM:SSZ` without any external crate.
    fn unix_to_iso8601(unix: u64) -> String {
        // Use std::time to derive wall-clock components without chrono.
        // Simple approach: delegate to a manual decomposition.
        let secs = unix;
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        let days = secs / 86400; // days since 1970-01-01

        // Gregorian calendar decomposition (handles leap years correctly).
        let (year, month, day) = days_to_ymd(days);
        format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
    }

    /// Convert days-since-epoch to (year, month, day) using the proleptic Gregorian calendar.
    fn days_to_ymd(days: u64) -> (u64, u64, u64) {
        // Algorithm from http://howardhinnant.github.io/date_algorithms.html
        let z = days + 719468;
        let era = z / 146097;
        let doe = z % 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y, m, d)
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
