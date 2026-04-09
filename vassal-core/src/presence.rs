//! presence.rs — Presence: human-input awareness and yield logic.
//!
//! Detects when a human touches the keyboard or mouse during an active
//! sequence and signals The Atlas to yield immediately.
//!
//! The yield is non-destructive: the engine simply stops the sequence
//! rather than reversing any already-executed actions.
//!
//! Compiled only when the `presence` feature is enabled.

use tokio::sync::mpsc;
use tracing::{debug, info};

// ── Presence Signal ───────────────────────────────────────────────────────────

/// A signal indicating that a human has touched an input device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceSignal {
    /// A mouse movement or button press was detected.
    MouseInput,
    /// A keyboard key press was detected.
    KeyboardInput,
}

// ── Presence Monitor ──────────────────────────────────────────────────────────

/// Spawn a background thread that monitors for human input events.
///
/// When an event is detected, a `PresenceSignal` is sent into `tx`.
/// The Atlas is expected to call `yield_to_presence()` upon receiving it.
///
/// The thread exits when `tx` is dropped.
pub fn spawn_monitor(tx: mpsc::Sender<PresenceSignal>) -> std::thread::JoinHandle<()> {
    info!("Presence monitor spawned");

    std::thread::spawn(move || {
        use rdev::{listen, Event, EventType};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Presence: tokio runtime failed");

        let callback = move |event: Event| {
            let signal = match event.event_type {
                EventType::MouseMove { .. }
                | EventType::ButtonPress(_)
                | EventType::ButtonRelease(_)
                | EventType::Wheel { .. } => Some(PresenceSignal::MouseInput),
                EventType::KeyPress(_) | EventType::KeyRelease(_) => {
                    Some(PresenceSignal::KeyboardInput)
                }
            };

            if let Some(sig) = signal {
                debug!(?sig, "Presence: input detected");
                // Block briefly to send — tolerable on a dedicated thread.
                let tx = tx.clone();
                rt.block_on(async move {
                    let _ = tx.send(sig).await;
                });
            }
        };

        if let Err(e) = listen(callback) {
            tracing::warn!(?e, "Presence monitor exited with error");
        }

        info!("Presence monitor thread exiting");
    })
}
