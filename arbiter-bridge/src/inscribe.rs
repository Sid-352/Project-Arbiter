use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use globset::{Glob, GlobMatcher};

#[derive(Debug, thiserror::Error)]
pub enum InscribeError {
    #[error("Inscribe: path '{0}' is not in a trusted directory")]
    NotTrusted(String),
    #[error("Inscribe: source '{0}' does not exist")]
    SourceNotFound(String),
    #[error("Inscribe: I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn assert_trusted(path: impl AsRef<Path>, trusted_roots: &[String]) -> Result<(), InscribeError> {
    let path = path.as_ref();

    let canonical_path = if path.exists() {
        std::fs::canonicalize(path)?
    } else if let Some(parent) = path.parent() {
        let mut curr = parent;
        while !curr.exists() && curr.parent().is_some() {
            curr = curr.parent().unwrap();
        }
        std::fs::canonicalize(curr)?.join(path.file_name().unwrap_or_default())
    } else {
        path.to_path_buf()
    };

    if trusted_roots.iter().any(|root| {
        if let Ok(canon_root) = std::fs::canonicalize(root) {
            canonical_path.starts_with(canon_root)
        } else {
            canonical_path.to_string_lossy().starts_with(root)
        }
    }) {
        return Ok(());
    }
    let path_str = canonical_path.to_string_lossy().to_string();
    warn!(%path_str, "Inscribe: Conservatory rejected path (Traversal or Untrusted)");
    Err(InscribeError::NotTrusted(path_str))
}

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

fn ensure_file_path(src: &Path, dst: &Path) -> PathBuf {
    let mut final_dst = dst.to_path_buf();

    // Check if dst is a directory or intended to be one (ends with slash)
    let is_dir_intent =
        dst.to_string_lossy().ends_with('/') || dst.to_string_lossy().ends_with('\\');

    if dst.is_dir() || is_dir_intent {
        if let Some(filename) = src.file_name() {
            final_dst = final_dst.join(filename);
        }
    }
    final_dst
}

pub async fn move_file(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    trusted_roots: &[String],
) -> Result<PathBuf, InscribeError> {
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

    retry_with_backoff(|| async {
        if tokio::fs::rename(src, &dst).await.is_err() {
            tokio::fs::copy(src, &dst).await?;
            tokio::fs::remove_file(src).await?;
        }
        Ok(())
    })
    .await?;

    info!(src = %src.display(), dst = %dst.display(), "Inscribe: file moved");
    Ok(dst)
}

pub async fn copy_file(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    trusted_roots: &[String],
) -> Result<(PathBuf, u64), InscribeError> {
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

pub async fn delete_file(
    path: impl AsRef<Path>,
    trusted_roots: &[String],
) -> Result<(), InscribeError> {
    let path = path.as_ref();
    assert_trusted(path, trusted_roots)?;

    if !path.exists() {
        return Err(InscribeError::SourceNotFound(path.display().to_string()));
    }

    retry_with_backoff(|| tokio::fs::remove_file(path)).await?;
    info!(path = %path.display(), "Inscribe: file deleted");
    Ok(())
}

#[derive(Debug)]
pub struct DryRunReport {
    pub affected: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

pub fn dry_run_walk(root: &Path, pattern: &str) -> DryRunReport {
    let mut affected = Vec::new();
    let mut warnings = Vec::new();

    let matcher: Option<GlobMatcher> = if pattern.is_empty() {
        None
    } else {
        match Glob::new(pattern) {
            Ok(g) => Some(g.compile_matcher()),
            Err(_) => None,
        }
    };

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
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
                    warnings.push(format!(
                        "System-critical path detected: {path_name}",
                        path_name = path.display()
                    ));
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
            _ => panic!("Expected NotTrusted error, got {res:?}"),
        }
        assert!(!dst.exists());
    }

    #[tokio::test]
    async fn test_dry_run_warnings() {
        let sys_root = tempdir().unwrap();
        let f2 = sys_root.path().join("SYSTEM32");
        std::fs::create_dir_all(&f2).unwrap();
        File::create(f2.join("dummy.sys")).unwrap();

        let report = dry_run_walk(sys_root.path(), "*.sys");
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("System-critical")));
    }
    #[tokio::test]
    async fn missing_source_file_returns_error() {
        let root = tempdir().unwrap();

        let trusted = vec![root.path().to_string_lossy().to_string()];

        let src = root.path().join("missing.txt");
        let dst = root.path().join("out.txt");

        let res = move_file(&src, &dst, &trusted).await;

        assert!(matches!(res, Err(InscribeError::SourceNotFound(_))));
    }

    #[tokio::test]
    async fn delete_file_removes_file() {
        let root = tempdir().unwrap();

        let trusted = vec![root.path().to_string_lossy().to_string()];

        let file = root.path().join("temp.txt");

        File::create(&file).unwrap();

        let res = delete_file(&file, &trusted).await;

        assert!(res.is_ok());
        assert!(!file.exists());
    }

    #[test]
    fn dry_run_detects_matching_files() {
        let root = tempdir().unwrap();

        let file1 = root.path().join("a.log");
        let file2 = root.path().join("b.txt");

        File::create(&file1).unwrap();
        File::create(&file2).unwrap();

        let report = dry_run_walk(root.path(), "*.log");

        assert_eq!(report.affected.len(), 1);
        assert!(report.affected[0].to_string_lossy().contains("a.log"));
    }

    #[tokio::test]
    async fn copy_file_returns_bytes_and_creates_destination() {
        let root = tempdir().unwrap();

        let trusted = vec![root.path().to_string_lossy().to_string()];

        let src = root.path().join("source.txt");
        let dst = root.path().join("copied.txt");

        std::fs::write(&src, "hello world").unwrap();

        let result = copy_file(&src, &dst, &trusted).await;

        assert!(result.is_ok());

        let (returned_path, bytes) = result.unwrap();

        assert_eq!(returned_path, dst);
        assert_eq!(bytes, 11);

        assert!(dst.exists());
    }
}
