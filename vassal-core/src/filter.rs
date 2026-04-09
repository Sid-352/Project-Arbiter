//! filter.rs — The Filter: explicit thread-ID / path tagging guard.
//! 
//! Prevents The Vigil from reacting to file creations caused by The Inscribe
//! (Vassal's own File I/O component).
//!
//! When The Inscribe moves a file, it adds the destination path here.
//! The Vigil checks this filter before dispatching a Summons.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};

fn normalize_key(path: impl AsRef<Path>) -> String {
    let p = path.as_ref();
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    abs.to_string_lossy().to_lowercase()
}

/// A thread-safe, shared set of paths currently being written by Vassal itself.
#[derive(Debug, Clone, Default)]
pub struct VassalFilter {
    active_paths: Arc<Mutex<HashSet<String>>>,
}

impl VassalFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a path as currently being written by Vassal.
    pub fn mark(&self, path: impl AsRef<Path>) {
        let key = normalize_key(path);
        if let Ok(mut set) = self.active_paths.lock() {
            set.insert(key);
        }
    }

    /// Unmark a path (Vassal finishes writing).
    pub fn unmark(&self, path: impl AsRef<Path>) {
        let key = normalize_key(path);
        if let Ok(mut set) = self.active_paths.lock() {
            set.remove(&key);
        }
    }

    /// Returns `true` if this path is currently marked by Vassal.
    pub fn is_own(&self, path: impl AsRef<Path>) -> bool {
        let key = normalize_key(path);
        if let Ok(set) = self.active_paths.lock() {
            set.contains(&key)
        } else {
            false
        }
    }
}
