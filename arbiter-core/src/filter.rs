//! filter.rs — explicit thread-ID / path tagging guard.
//!
//! Prevents The Vigil from reacting to file creations caused by The Inscribe
//! (Arbiter's own File I/O component).
//!
//! When The Inscribe moves a file, it adds the destination path here.
//! The Vigil checks this filter before dispatching a Summons.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Instant, Duration},
};

fn normalize_key(path: impl AsRef<Path>) -> String {
    let p = path.as_ref();
    let abs = if p.exists() {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    } else if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    abs.to_string_lossy().to_lowercase()
}

/// A thread-safe, shared set of paths and flags currently being manipulated by Arbiter itself.
#[derive(Debug, Clone, Default)]
pub struct ArbiterFilter {
    /// Maps normalized paths to the time they were last marked.
    active_paths: Arc<Mutex<HashMap<String, Instant>>>,
    /// When true, the engine is generating hardware input (Hand is active).
    /// Used by The Presence to inhibit self-abort cycles.
    interference_lock: Arc<AtomicBool>,
}

impl ArbiterFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a path as currently being written by Arbiter.
    pub fn mark(&self, path: impl AsRef<Path>) {
        let key = normalize_key(path);
        if let Ok(mut map) = self.active_paths.lock() {
            map.insert(key, Instant::now());
        }
    }

    /// Unmark a path (Arbiter finishes writing). 
    /// Note: Paths are also automatically expired after 3 seconds in is_own.
    pub fn unmark(&self, path: impl AsRef<Path>) {
        let key = normalize_key(path);
        if let Ok(mut map) = self.active_paths.lock() {
            map.remove(&key);
        }
    }

    /// Returns `true` if this path was recently marked by Arbiter (within 3 seconds).
    pub fn is_own(&self, path: impl AsRef<Path>) -> bool {
        let key = normalize_key(path);
        if let Ok(mut map) = self.active_paths.lock() {
            // Prune expired entries while we're here
            let now = Instant::now();
            let expiry = Duration::from_secs(3);
            map.retain(|_, &mut time| now.duration_since(time) < expiry);
            
            map.contains_key(&key)
        } else {
            false
        }
    }

    /// Inhibit presence detection (Hand is about to act).
    pub fn inhibit_presence(&self) {
        self.interference_lock.store(true, Ordering::SeqCst);
    }

    /// Resume presence detection (Hand has finished).
    pub fn resume_presence(&self) {
        self.interference_lock.store(false, Ordering::SeqCst);
    }

    /// Returns `true` if presence detection is currently inhibited.
    pub fn is_inhibited(&self) -> bool {
        self.interference_lock.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_mark_unmark() {
        let filter = ArbiterFilter::new();
        let p = Path::new("C:\\Engine\\Dummy\\file.txt");

        assert!(!filter.is_own(p));
        filter.mark(p);
        assert!(filter.is_own(p));
        filter.unmark(p);
        assert!(!filter.is_own(p));
    }

    #[test]
    fn test_filter_case_insensitivity() {
        let filter = ArbiterFilter::new();
        filter.mark("C:\\Temp\\File.txt");

        // Assert that a lowercased or differently cased variant matches
        assert!(filter.is_own("c:\\temp\\file.txt"));
        assert!(filter.is_own("C:\\temp\\FILE.TXT"));
    }

    #[test]
    fn test_filter_absolute_resolution() {
        let filter = ArbiterFilter::new();
        let rel_path = Path::new("test_file.txt");
        filter.mark(rel_path);

        let abs_path = std::env::current_dir().unwrap().join("test_file.txt");
        assert!(filter.is_own(abs_path));
    }
}
