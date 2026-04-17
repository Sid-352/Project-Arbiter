//! arbiter-core — Engine's nerve center.
//!
//! Module layout:
//!   decree     — Data types: triggers, actions, sequences.
//!   atlas      — Orchestrator and run-loop.
//!   vigil      — System-event watchers.
//!   presence   — Yield logic for human interference.
//!   signet     — Encrypted config vault.

pub mod atlas;
pub mod decree;
pub mod protocol;
pub mod ledger;

#[cfg(any(feature = "vigil-fs", feature = "vigil-keys"))]
pub mod vigil;

#[cfg(feature = "presence")]
pub mod presence;

#[cfg(feature = "signet")]
pub mod signet;

pub mod filter;
