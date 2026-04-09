/// vassal-core — The engine's nerve center.
///
/// Module layout follows the Vassal Lexicon:
///   ordinance  — Data types: triggers, actions, sequences, log events.
///   atlas      — The FSM orchestrator and run-loop (The Atlas).
///   vigil      — System-event watchers: file changes, hotkeys (The Vigil).
///   presence   — Human-input detection and yield logic (Presence).
///   signet     — Encrypted config vault (The Signet).

pub mod ordinance;
pub mod atlas;

#[cfg(any(feature = "vigil-fs", feature = "vigil-keys"))]
pub mod vigil;

#[cfg(feature = "presence")]
pub mod presence;

#[cfg(feature = "signet")]
pub mod signet;

pub mod filter;
