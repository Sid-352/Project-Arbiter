//! shell.rs — The Baton: guarded shell command execution.
//!
//! Shell execution is the highest-privilege action Vassal can perform.
//! Every command must pass The Baton toggle before it runs — a per-target
//! explicit user allowance stored in The Signet.
//!
//! Responsibilities:
//!   - Execute a shell command given a Baton-allowed target path.
//!   - Validate the target is explicitly permitted before spawning.
//!   - Capture stdout/stderr and surface them as a structured result.
//!   - Never run as elevated — if a privileged target is needed, surface
//!     a clear error rather than escalating silently.

use std::process::{Command, Output};
use tracing::{info, warn};

// ── Baton Guard ───────────────────────────────────────────────────────────────

/// Error type for shell operations.
#[derive(Debug)]
pub enum ShellError {
    /// The Baton toggle is not active for this target.
    BatonNotGranted(String),
    /// The process failed to spawn (missing binary, permissions, etc.).
    SpawnFailed(String),
    /// The process ran but exited with a non-zero status.
    NonZeroExit { status: i32, stderr: String },
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BatonNotGranted(t) => {
                write!(
                    f,
                    "The Baton: '{t}' is not allowed — grant it in the Signet first"
                )
            }
            Self::SpawnFailed(e) => write!(f, "Shell: spawn failed: {e}"),
            Self::NonZeroExit { status, stderr } => {
                write!(f, "Shell: exit {status} — {stderr}")
            }
        }
    }
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// The output of a successful shell command.
#[derive(Debug)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Execute `command` with `args` if `target_key` is Baton-allowed.
///
/// `target_key` is the canonical identifier checked against the Signet's
/// `baton_allowed` set (typically the script path or command name).
///
/// `allowed_targets` — callers should pass `&config.baton_allowed`.
pub fn run(
    target_key: &str,
    command: &str,
    args: &[&str],
    allowed_targets: &std::collections::HashSet<String>,
) -> Result<ShellOutput, ShellError> {
    if !allowed_targets.contains(target_key) {
        warn!(%target_key, "The Baton: execution blocked — not in allowed set");
        return Err(ShellError::BatonNotGranted(target_key.to_string()));
    }

    info!(%command, ?args, "The Baton: executing allowed command");

    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| ShellError::SpawnFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();
    let exit_code = status.code().unwrap_or(-1);

    if !status.success() {
        warn!(%exit_code, %stderr, "Shell: command exited with error");
        return Err(ShellError::NonZeroExit {
            status: exit_code,
            stderr,
        });
    }

    info!(%exit_code, "Shell: command completed successfully");
    Ok(ShellOutput {
        stdout,
        stderr,
        exit_code,
    })
}

/// Spawn a command detached (fire-and-forget), verifying Baton first.
///
/// Unlike `run`, this does not wait for the process to finish and does
/// not capture output. Suitable for launching background applications.
pub fn spawn_detached(
    target_key: &str,
    command: &str,
    args: &[&str],
    allowed_targets: &std::collections::HashSet<String>,
) -> Result<(), ShellError> {
    if !allowed_targets.contains(target_key) {
        warn!(%target_key, "The Baton: detached spawn blocked");
        return Err(ShellError::BatonNotGranted(target_key.to_string()));
    }

    Command::new(command)
        .args(args)
        .spawn()
        .map_err(|e| ShellError::SpawnFailed(e.to_string()))?;

    info!(%command, "The Baton: process spawned detached");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_baton_granted() {
        let mut allowed = HashSet::new();
        allowed.insert("safe_echo".to_string());

        let cmd = if cfg!(target_os = "windows") {
            "cmd.exe"
        } else {
            "sh"
        };
        let args = if cfg!(target_os = "windows") {
            vec!["/c", "echo Hello"]
        } else {
            vec!["-c", "echo Hello"]
        };

        let result = run("safe_echo", cmd, &args, &allowed);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.stdout.contains("Hello"));
        assert_eq!(output.exit_code, 0);
    }

    #[test]
    fn test_baton_blocked() {
        let mut allowed = HashSet::new();
        allowed.insert("safe_echo".to_string());

        let cmd = if cfg!(target_os = "windows") {
            "cmd.exe"
        } else {
            "sh"
        };
        let args = if cfg!(target_os = "windows") {
            vec!["/c", "echo Malicious"]
        } else {
            vec!["-c", "echo Malicious"]
        };

        let result = run("malicious_script", cmd, &args, &allowed);

        match result {
            Err(ShellError::BatonNotGranted(req)) => {
                assert_eq!(req, "malicious_script");
            }
            _ => panic!("Expected BatonNotGranted error"),
        }
    }
}
