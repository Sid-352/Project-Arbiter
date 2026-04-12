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
                            context.insert("file_path", &path_str);
                            context.insert(
                                "file_name",
                                path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                            );
                            context.insert(
                                "file_ext",
                                path.extension().and_then(|e| e.to_str()).unwrap_or(""),
                            );
                            context.insert(
                                "timestamp",
                                &format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
                            );

                            // Wire up the lazy resolver: store the real PathBuf
                            // and propagate the Ward's permission layer.
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
}

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
