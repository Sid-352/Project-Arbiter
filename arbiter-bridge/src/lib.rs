//! arbiter-bridge — mechanical execution bridge.
//!
//! Provides three independent, focused modules:
//!   hand     — enigo hardware bridge: mouse + keyboard (Hand).
//!   inscribe — file-system write operations (Inscribe).
//!   shell    — process spawning with Baton toggle guard (The Baton).

pub mod runner;
pub mod hand;
pub mod inscribe;
pub mod shell;
