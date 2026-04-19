//! inscribe.rs — file-system write operations.
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

use globset::{Glob, GlobMatcher};

// ── Directory Jail Guard ──────────────────────────────────────────────────────

/// Error type for Inscribe operations.
#[derive(Debug, thiserror::Error)]
pub enum InscribeError {
    /// The target path is not within any trusted directory (Conservatory violation).
    #[error("Inscribe: path '{0}' is not in a trusted directory")]
    NotTrusted(String),
    /// The source path does not exist.
    #[error("Inscribe: source '{0}' does not exist")]
    SourceNotFound(String),
    /// A standard I/O error occurred.
    #[error("Inscribe: I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Verify a path is under at least one trusted root before any write.
/// Performs canonicalization to prevent path traversal (e.g. ..\..\).
fn assert_trusted(path: impl AsRef<Path>, trusted_roots: &[String]) -> Result<(), InscribeError> {
    let path = path.as_ref();
    
    // Canonicalize the path to resolve symlinks and '..' components.
    // If the path doesn't exist yet (common for destinations), canonicalize the parent.
    let canonical_path = if path.exists() {
        std::fs::canonicalize(path)?
    } else if let Some(parent) = path.parent() {
        // If parent exists, canonicalize it and join the filename.
        // If parent doesn't exist, we'll create it later, but for the check
        // we keep going up until we find something that exists or hit the root.
        let mut curr = parent;
        while !curr.exists() && curr.parent().is_some() {
            curr = curr.parent().unwrap();
        }
        std::fs::canonicalize(curr)?.join(path.file_name().unwrap_or_default())
    } else {
        path.to_path_buf()
    };

    let path_str = canonical_path.to_string_lossy();
    
    if trusted_roots
        .iter()
        .any(|root| {
            // Also canonicalize the trusted root for a fair comparison
            if let Ok(canon_root) = std::fs::canonicalize(root) {
                path_str.starts_with(canon_root.to_string_lossy().as_ref())
            } else {
                path_str.starts_with(root.as_str())
            }
        })
    {
        return Ok(());
    }
    warn!(%path_str, "Inscribe: Conservatory rejected path (Traversal or Untrusted)");
    Err(InscribeError::NotTrusted(path_str.to_string()))
}

// ── File Operations ───────────────────────────────────────────────────────────

/// Executes a closure with exponential backoff for Transient/Permission/Lock errors.
async fn retry_with_backoff<F, Fut, T>(mut action: F) -> std::io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::io::Result<T>>,
{
    let mut delay = 100;
    let mut attempts = 0;
    loop {
        match action().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                attempts += 1;
                if attempts >= 5 {
                    return Err(e);
                }
                warn!(%e, "Inscribe: Operation failed, retrying in {}ms (Attempt {})", delay, attempts);
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                delay *= 2;
            }
        }
    }
}

/// Helper to ensure destination is a full file path.
/// If `dst` is an existing directory or ends with a slash, appends the filename from `src`.
fn ensure_file_path(src: &Path, dst: &Path) -> PathBuf {
    let mut final_dst = dst.to_path_buf();
    
    // Check if dst is a directory or intended to be one (ends with slash)
    let is_dir_intent = dst.to_string_lossy().ends_with('/') || dst.to_string_lossy().ends_with('\\');
    
    if dst.is_dir() || is_dir_intent {
        if let Some(filename) = src.file_name() {
            final_dst = final_dst.join(filename);
        }
    }
    final_dst
}

/// Move `src` to `dst`, verifying `dst` parent is in a trusted directory.
/// Returns the final destination path used.
pub async fn move_file(src: impl AsRef<Path>, dst: impl AsRef<Path>, trusted_roots: &[String]) -> Result<PathBuf, InscribeError> {
    let src = src.as_ref();
    let dst_raw = dst.as_ref();
    
    if !src.exists() {
        return Err(InscribeError::SourceNotFound(src.display().to_string()));
    }

    let dst = ensure_file_path(src, dst_raw);
    assert_trusted(&dst, trusted_roots)?;

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Attempt rename first (atomic on same volume), fall back to copy+delete
    retry_with_backoff(|| async {
        if tokio::fs::rename(src, &dst).await.is_err() {
            tokio::fs::copy(src, &dst).await?;
            tokio::fs::remove_file(src).await?;
        }
        Ok(())
    }).await?;

    info!(src = %src.display(), dst = %dst.display(), "Inscribe: file moved");
    Ok(dst)
}

/// Copy `src` to `dst`, verifying `dst` parent is in a trusted directory.
/// Returns the final destination path used and bytes copied.
pub async fn copy_file(src: impl AsRef<Path>, dst: impl AsRef<Path>, trusted_roots: &[String]) -> Result<(PathBuf, u64), InscribeError> {
    let src = src.as_ref();
    let dst_raw = dst.as_ref();

    if !src.exists() {
        return Err(InscribeError::SourceNotFound(src.display().to_string()));
    }

    let dst = ensure_file_path(src, dst_raw);
    assert_trusted(&dst, trusted_roots)?;

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let bytes = retry_with_backoff(|| tokio::fs::copy(src, &dst)).await?;
    info!(src = %src.display(), dst = %dst.display(), bytes, "Inscribe: file copied");
    Ok((dst, bytes))
}

/// Delete a file, verifying it is in a trusted directory.
pub async fn delete_file(path: impl AsRef<Path>, trusted_roots: &[String]) -> Result<(), InscribeError> {
    let path = path.as_ref();
    assert_trusted(path, trusted_roots)?;

    if !path.exists() {
        return Err(InscribeError::SourceNotFound(path.display().to_string()));
    }

    retry_with_backoff(|| tokio::fs::remove_file(path)).await?;
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

    let matcher: Option<GlobMatcher> = if !pattern.is_empty() {
        match Glob::new(pattern) {
            Ok(g) => Some(g.compile_matcher()),
            Err(_) => None,
        }
    } else {
        None
    };

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy();
            
            let matched = if let Some(ref m) = matcher {
                m.is_match(&*name)
            } else {
                true
            };

            if matched {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_conservatory_allows_trusted() {
        let root = tempdir().unwrap();
        let root_str = root.path().to_string_lossy().to_string();

        let trusted_roots = vec![root_str.clone()];

        let src = root.path().join("source.txt");
        let dst = root.path().join("subfolder").join("dest.txt");

        File::create(&src).unwrap();

        // Should succeed because dst is within root
        let res = move_file(&src, &dst, &trusted_roots).await;
        assert!(res.is_ok());
        assert!(dst.exists());
        assert!(!src.exists());
    }

    #[tokio::test]
    async fn test_conservatory_blocks_untrusted() {
        let allowed_root = tempdir().unwrap();
        let malicious_root = tempdir().unwrap();

        let trusted_roots = vec![allowed_root.path().to_string_lossy().to_string()];

        let src = allowed_root.path().join("source.txt");
        let dst = malicious_root.path().join("dest.txt");

        File::create(&src).unwrap();

        // Should return NotTrusted because dst is inside malicious_root
        let res = copy_file(&src, &dst, &trusted_roots).await;
        match res {
            Err(InscribeError::NotTrusted(_)) => {}
            _ => panic!("Expected NotTrusted error, got {:?}", res),
        }
        assert!(!dst.exists());
    }

    #[tokio::test]
    async fn test_dry_run_warnings() {
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
