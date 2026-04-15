//! protocol.rs — The Arbiter IPC Protocol.
//!
//! Shared definitions for communication between the background service
//! (arbiter-app) and the UI terminal (arbiter-forge).

use serde::{Deserialize, Serialize};

// ── Pipe Constants ───────────────────────────────────────────────────────────

/// Outbound telemetry stream (App -> Forge).
/// Protocol: Newline-delimited JSON of `LogEntry`.
pub const PIPE_TELEMETRY: &str = r"\\.\pipe\arbiter_telemetry";

/// Inbound control stream (Forge -> App).
/// Protocol: Newline-delimited JSON of `ForgeCommand`.
pub const PIPE_COMMAND: &str = r"\\.\pipe\arbiter_command";

// ── Data Types ────────────────────────────────────────────────────────────────

/// Commands sent from the Forge UI to the Arbiter Engine.
#[derive(Debug, Serialize, Deserialize)]
pub enum ForgeCommand {
    /// Save a new or updated ordinance definition.
    SaveDecree(crate::ledger::OrdinanceDef),
    /// Request a reload of all ward configurations.
    ReloadWards,
    /// Manually trigger a specific ordinance.
    ManualRun { summons_key: String },
}

/// A structured log entry for transmission over the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Timestamp (UTC RFC3339) or simple duration string.
    #[serde(default)]
    pub time: String,
    /// Short category tag (e.g. "ATLAS", "VIGIL", "HAND").
    pub tag: String,
    /// Full message text.
    pub message: String,
    /// True if this represents a fault or error state.
    pub is_error: bool,
    /// The ID of the ordinance currently executing, if any.
    pub ordinance_id: Option<String>,
}
