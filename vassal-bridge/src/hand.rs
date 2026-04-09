//! hand.rs — The Hand: the hardware execution bridge.
//!
//! Wraps `enigo` to provide a safe, coordinate-validated interface for
//! mouse and keyboard actions. Every operation goes through The Hand —
//! no other crate touches raw input APIs.
//!
//! Responsibilities:
//!   - Execute `Action` structs produced by The Atlas.
//!   - Validate screen coordinates before moving the mouse (Hardware Guard).
//!   - Queue semantics are enforced by the caller (The Queue); The Hand
//!     is intentionally stateless beyond `enigo` internals.
//!
//! Salvaged from: lithos-core/src/hardware.rs (full port, zero changes to logic).

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use tracing::{debug, warn};
use vassal_core::ordinance::{Action, ActionType};

// ── Hardware Bridge ───────────────────────────────────────────────────────────

/// The Hand: a stateful wrapper around `enigo` with coordinate validation.
pub struct HardwareBridge {
    enigo: Enigo,
    screen_width: i32,
    screen_height: i32,
}

impl HardwareBridge {
    /// Initialise The Hand for the given screen dimensions.
    ///
    /// Panics only if `enigo` itself fails to initialise — a hard system error.
    pub fn new(width: i32, height: i32) -> Self {
        let enigo = Enigo::new(&Settings::default())
            .expect("The Hand: failed to initialise enigo hardware bridge");
        debug!(width, height, "The Hand initialised");
        Self { enigo, screen_width: width, screen_height: height }
    }

    /// Execute a single resolved `Action`.
    ///
    /// Returns `Err` if the coordinate is out of bounds (Hardware Guard)
    /// or if the underlying `enigo` call fails.
    pub fn execute(&mut self, action: &Action) -> Result<(), String> {
        // Move to target coordinate before acting (if provided)
        if let Some(ref pt) = action.point {
            self.validate_coordinate(pt.x, pt.y)?;
            self.enigo
                .move_mouse(pt.x, pt.y, Coordinate::Abs)
                .map_err(|e| format!("The Hand: mouse move failed: {e:?}"))?;
            debug!(x = pt.x, y = pt.y, "The Hand: mouse positioned");
        }

        // Pre-action delay (The Queue pacing gate)
        if action.delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(action.delay_ms));
        }

        match &action.action_type {
            ActionType::Click => {
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("The Hand: click failed: {e:?}"))?;
            }
            ActionType::DoubleClick => {
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("The Hand: double-click (1) failed: {e:?}"))?;
                std::thread::sleep(std::time::Duration::from_millis(80));
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("The Hand: double-click (2) failed: {e:?}"))?;
            }
            ActionType::RightClick => {
                self.enigo
                    .button(Button::Right, Direction::Click)
                    .map_err(|e| format!("The Hand: right-click failed: {e:?}"))?;
            }
            ActionType::Type(text) => {
                self.enigo
                    .text(text)
                    .map_err(|e| format!("The Hand: type failed: {e:?}"))?;
            }
            ActionType::Scroll(amount) => {
                self.enigo
                    .scroll(*amount, Axis::Vertical)
                    .map_err(|e| format!("The Hand: scroll failed: {e:?}"))?;
            }
            ActionType::Navigate(keys) => {
                // OS-native navigation: pass keystrokes directly to enigo
                // e.g. "ctrl+shift+s", "super+s", "alt+tab"
                self.enigo
                    .text(keys)
                    .map_err(|e| format!("The Hand: navigate failed: {e:?}"))?;
            }
            ActionType::Wait(ms) => {
                std::thread::sleep(std::time::Duration::from_millis(*ms));
            }
        }

        Ok(())
    }

    // ── Hardware Guard ────────────────────────────────────────────────────────

    /// Reject coordinates outside the declared monitor bounds.
    fn validate_coordinate(&self, x: i32, y: i32) -> Result<(), String> {
        if x < 0 || x > self.screen_width || y < 0 || y > self.screen_height {
            let msg = format!(
                "Hardware Guard: ({x}, {y}) outside monitor bounds ({}×{})",
                self.screen_width, self.screen_height
            );
            warn!(%msg, "The Hand: coordinate rejected");
            return Err(msg);
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vassal_core::ordinance::{Action, ActionType, Point};

    #[test]
    fn coordinate_guard_rejects_out_of_bounds() {
        let bridge = HardwareBridge::new(1920, 1080);
        // Direct call to the guard — no enigo interaction
        assert!(bridge.validate_coordinate(2000, 500).is_err());
        assert!(bridge.validate_coordinate(-1, 0).is_err());
        assert!(bridge.validate_coordinate(960, 540).is_ok());
    }

    #[test]
    fn wait_action_does_not_need_coordinates() {
        let mut bridge = HardwareBridge::new(1920, 1080);
        let action = Action {
            action_type: ActionType::Wait(10),
            point: None,
            delay_ms: 0,
        };
        assert!(bridge.execute(&action).is_ok());
    }
}
