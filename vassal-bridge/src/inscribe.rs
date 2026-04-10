//! inscribe.rs — Inscribe: file-system write operations.
//!
//! Provides directory-jail-aware file operations. Every write is checked
//! against the Conservatory (trusted paths from The Signet) before execution.
//!
//! Responsibilities:
//!   - Move / copy / delete files within trusted directories.
//!   - Walk source directories and enumerate matching files (walkdir).
//!   - Return dry-run previews before any actual mutation.

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

// ── Directory Jail Guard ──────────────────────────────────────────────────────

/// Error type for Inscribe operations.
#[derive(Debug)]
pub enum InscribeError {
    /// The target path is not within any trusted directory (Conservatory violation).
    NotTrusted(String),
    /// The source path does not exist.
    SourceNotFound(String),
    /// A standard I/O error occurred.
    Io(std::io::Error),
}

impl std::fmt::Display for InscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotTrusted(p) => write!(f, "Inscribe: path '{p}' is not in a trusted directory"),
            Self::SourceNotFound(p) => write!(f, "Inscribe: source '{p}' does not exist"),
            Self::Io(e) => write!(f, "Inscribe: I/O error: {e}"),
        }
    }
}

impl From<std::io::Error> for InscribeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Verify a path is under at least one trusted root before any write.
fn assert_trusted(path: &Path, trusted_roots: &[String]) -> Result<(), InscribeError> {
    let path_str = path.to_string_lossy();
    if trusted_roots
        .iter()
        .any(|root| path_str.starts_with(root.as_str()))
    {
        return Ok(());
    }
    warn!(%path_str, "Inscribe: Conservatory rejected path");
    Err(InscribeError::NotTrusted(path_str.to_string()))
}

// ── File Operations ───────────────────────────────────────────────────────────

/// Move `src` to `dst`, verifying `dst` parent is in a trusted directory.
pub fn move_file(src: &Path, dst: &Path, trusted_roots: &[String]) -> Result<(), InscribeError> {
    assert_trusted(dst, trusted_roots)?;

    if !src.exists() {
        return Err(InscribeError::SourceNotFound(src.display().to_string()));
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Attempt rename first (atomic on same volume), fall back to copy+delete
    if std::fs::rename(src, dst).is_err() {
        std::fs::copy(src, dst)?;
        std::fs::remove_file(src)?;
    }

    info!(src = %src.display(), dst = %dst.display(), "Inscribe: file moved");
    Ok(())
}

/// Copy `src` to `dst`, verifying `dst` parent is in a trusted directory.
pub fn copy_file(src: &Path, dst: &Path, trusted_roots: &[String]) -> Result<u64, InscribeError> {
    assert_trusted(dst, trusted_roots)?;

    if !src.exists() {
        return Err(InscribeError::SourceNotFound(src.display().to_string()));
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let bytes = std::fs::copy(src, dst)?;
    info!(src = %src.display(), dst = %dst.display(), bytes, "Inscribe: file copied");
    Ok(bytes)
}

/// Delete a file, verifying it is in a trusted directory.
pub fn delete_file(path: &Path, trusted_roots: &[String]) -> Result<(), InscribeError> {
    assert_trusted(path, trusted_roots)?;

    if !path.exists() {
        return Err(InscribeError::SourceNotFound(path.display().to_string()));
    }

    std::fs::remove_file(path)?;
    info!(path = %path.display(), "Inscribe: file deleted");
    Ok(())
}

// ── Dry-Run Preview ───────────────────────────────────────────────────────────

/// A preview of what would be affected by an Inscribe operation.
#[derive(Debug)]
pub struct DryRunReport {
    /// Files that would be affected.
    pub affected: Vec<PathBuf>,
    /// Any system-critical paths detected (e.g. paths under Windows, System32).
    pub warnings: Vec<String>,
}

/// Walk `root` and return all files matching `pattern` as a dry-run preview.
///
/// Does NOT perform any write. Used for Perception Simulation before a rule
/// is activated (CONTEXT.md §4).
pub fn dry_run_walk(root: &Path, pattern: &str) -> DryRunReport {
    let mut affected = Vec::new();
    let mut warnings = Vec::new();

    let re = regex_for_glob(pattern);

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy();
            if re.is_match(&name) {
                let path = entry.path().to_path_buf();
                let path_str = path.to_string_lossy().to_lowercase();

                if path_str.contains("windows") || path_str.contains("system32") {
                    warnings.push(format!("System-critical path detected: {}", path.display()));
                }

                debug!(path = %path.display(), "Dry-run: would affect");
                affected.push(path);
            }
        }
    }

    DryRunReport { affected, warnings }
}

/// Convert a simple glob pattern (`*.ext`, `prefix*`) to a `regex::Regex`.
fn regex_for_glob(glob: &str) -> regex::Regex {
    use regex::Regex;
    let escaped = regex::escape(glob).replace("\\*", ".*");
    Regex::new(&format!("(?i)^{escaped}$")).unwrap_or_else(|_| Regex::new(".*").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_conservatory_allows_trusted() {
        let root = tempdir().unwrap();
        let root_str = root.path().to_string_lossy().to_string();

        let trusted_roots = vec![root_str.clone()];

        let src = root.path().join("source.txt");
        let dst = root.path().join("subfolder").join("dest.txt");

        File::create(&src).unwrap();

        // Should succeed because dst is within root
        let res = move_file(&src, &dst, &trusted_roots);
        assert!(res.is_ok());
        assert!(dst.exists());
        assert!(!src.exists());
    }

    #[test]
    fn test_conservatory_blocks_untrusted() {
        let allowed_root = tempdir().unwrap();
        let malicious_root = tempdir().unwrap();

        let trusted_roots = vec![allowed_root.path().to_string_lossy().to_string()];

        let src = allowed_root.path().join("source.txt");
        let dst = malicious_root.path().join("dest.txt");

        File::create(&src).unwrap();

        // Should return NotTrusted because dst is inside malicious_root
        let res = copy_file(&src, &dst, &trusted_roots);
        match res {
            Err(InscribeError::NotTrusted(_)) => {}
            _ => panic!("Expected NotTrusted error, got {:?}", res),
        }
        assert!(!dst.exists());
    }

    #[test]
    fn test_dry_run_warnings() {
        // Just verify the regex warning triggers correctly without writing a real system file
        let sys_root = tempdir().unwrap();

        // Windows warning check only checks if path contains 'system32' (case insensitive converted)
        let f2 = sys_root.path().join("SYSTEM32");
        std::fs::create_dir_all(&f2).unwrap();
        File::create(f2.join("dummy.sys")).unwrap();

        let report = dry_run_walk(sys_root.path(), "*.sys");
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("System-critical")));
    }
}
