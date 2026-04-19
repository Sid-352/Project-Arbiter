//! hand.rs — hardware execution bridge.
//!
//! Wraps `enigo` to provide a safe, coordinate-validated interface for
//! mouse and keyboard actions. Every operation goes through Hand —
//! no other crate touches raw input APIs.
//!
//! Responsibilities:
//!   - Execute `Action` structs produced by The Atlas.
//!   - Validate screen coordinates before moving the mouse (Hardware Guard).
//!   - Queue semantics are enforced by the caller (The Queue); Hand
//!     is intentionally stateless beyond `enigo` internals.
//!
//! Salvaged from: lithos-core/src/hardware.rs (full port, zero changes to logic).

use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings,
};
use tracing::{debug, warn};
use arbiter_core::decree::{Action, ActionType};

// ── Hardware Bridge ───────────────────────────────────────────────────────────

/// Hand: a stateful wrapper around `enigo` with coordinate validation.
pub struct HardwareBridge {
    enigo: Enigo,
    screen_width: i32,
    screen_height: i32,
}

impl HardwareBridge {
    /// Initialise Hand for the given screen dimensions.
    ///
    /// Panics only if `enigo` itself fails to initialise — a hard system error.
    pub fn new(width: i32, height: i32) -> Self {
        let enigo = Enigo::new(&Settings::default())
            .expect("Hand: failed to initialise enigo hardware bridge");
        debug!(width, height, "Hand initialised");
        Self {
            enigo,
            screen_width: width,
            screen_height: height,
        }
    }

    /// Execute a single resolved `Action`.
    ///
    /// Returns `Err` if the coordinate is out of bounds (Hardware Guard)
    /// or if the underlying `enigo` call fails.
    pub async fn execute(&mut self, action: &Action) -> Result<(), String> {
        // Move to target coordinate before acting (if provided)
        if let Some(ref pt) = action.point {
            self.validate_coordinate(pt.x, pt.y)?;
            self.enigo
                .move_mouse(pt.x, pt.y, Coordinate::Abs)
                .map_err(|e| format!("Hand: mouse move failed: {e:?}"))?;
        }

        match &action.action_type {
            ActionType::Click => {
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("Hand: click failed: {e:?}"))?;
            }
            ActionType::DoubleClick => {
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("Hand: double-click (1) failed: {e:?}"))?;
                tokio::time::sleep(std::time::Duration::from_millis(80)).await; // Fine-grained internal click-speed delay
                self.enigo
                    .button(Button::Left, Direction::Click)
                    .map_err(|e| format!("Hand: double-click (2) failed: {e:?}"))?;
            }
            ActionType::RightClick => {
                self.enigo
                    .button(Button::Right, Direction::Click)
                    .map_err(|e| format!("Hand: right-click failed: {e:?}"))?;
            }
            ActionType::Type(text) => {
                if !text.is_empty() {
                    for c in text.chars() {
                        match c {
                            '\n' => {
                                self.enigo.key(enigo::Key::Return, Direction::Click)
                                    .map_err(|e| format!("Hand: newline failed: {e:?}"))?;
                            }
                            '\r' => { /* skip carriage returns */ }
                            '\t' => {
                                self.enigo.key(enigo::Key::Tab, Direction::Click)
                                    .map_err(|e| format!("Hand: tab failed: {e:?}"))?;
                            }
                            _ => {
                                let s = c.to_string();
                                self.enigo.text(&s)
                                    .map_err(|e| format!("Hand: char type failed ('{c}'): {e:?}"))?;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    }
                }
            }
            ActionType::Scroll(amount) => {
                self.enigo
                    .scroll(*amount, Axis::Vertical)
                    .map_err(|e| format!("Hand: scroll failed: {e:?}"))?;
            }
            ActionType::Navigate(keys) => {
                // OS-native navigation: parse and press keys
                let keys_lower = keys.to_lowercase();
                let parts: Vec<&str> = keys_lower.split('+').collect();
                let mut modifiers = Vec::new();
                let mut target_key = None;

                for part in parts {
                    match part.trim() {
                        "ctrl" | "control" => modifiers.push(enigo::Key::Control),
                        "alt" => modifiers.push(enigo::Key::Alt),
                        "shift" => modifiers.push(enigo::Key::Shift),
                        "super" | "win" | "command" | "meta" => modifiers.push(enigo::Key::Meta),
                        "return" | "enter" => target_key = Some(enigo::Key::Return),
                        "esc" | "escape" => target_key = Some(enigo::Key::Escape),
                        "tab" => target_key = Some(enigo::Key::Tab),
                        "space" => target_key = Some(enigo::Key::Space),
                        "backspace" => target_key = Some(enigo::Key::Backspace),
                        "delete" => target_key = Some(enigo::Key::Delete),
                        "up" => target_key = Some(enigo::Key::UpArrow),
                        "down" => target_key = Some(enigo::Key::DownArrow),
                        "left" => target_key = Some(enigo::Key::LeftArrow),
                        "right" => target_key = Some(enigo::Key::RightArrow),
                        s if s.len() == 1 => {
                            target_key = Some(enigo::Key::Unicode(s.chars().next().unwrap()));
                        }
                        other => {
                            warn!(%other, "Hand: unknown navigation key - ignoring");
                        }
                    }
                }

                // Execute key sequence
                for &mod_key in &modifiers {
                    self.enigo
                        .key(mod_key, Direction::Press)
                        .map_err(|e| format!("Hand: modifier press failed: {e:?}"))?;
                }

                if let Some(k) = target_key {
                    self.enigo
                        .key(k, Direction::Click)
                        .map_err(|e| format!("Hand: key click failed: {e:?}"))?;
                }

                for &mod_key in modifiers.iter().rev() {
                    self.enigo
                        .key(mod_key, Direction::Release)
                        .map_err(|e| format!("Hand: modifier release failed: {e:?}"))?;
                }

                debug!(%keys, "Hand: navigation executed");
            }
            ActionType::Wait(_) => {
                // No-op here: waits are now handled asynchronously by the Runner
            }
            // File & Shell actions are handled directly by the Runner, not Hand.
            other => {
                warn!(action = ?other, "Hand received a non-synthetic action — ignoring");
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
            warn!(%msg, "Hand: coordinate rejected");
            return Err(msg);
        }
        Ok(())
        }
        }

        impl Drop for HardwareBridge {
        fn drop(&mut self) {
        // Safety: Release all possible modifiers to avoid leaving the user's
        // keyboard in a "stuck" state if the engine panics or is dropped mid-action.
        let modifiers = [
            enigo::Key::Control,
            enigo::Key::Alt,
            enigo::Key::Shift,
            enigo::Key::Meta,
        ];

        for &key in &modifiers {
            let _ = self.enigo.key(key, Direction::Release);
        }
        debug!("Hand: safety release executed (all modifiers up)");
        }
        }

        // ── Helpers ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arbiter_core::decree::{Action, ActionType};

    #[tokio::test]
    async fn coordinate_guard_rejects_out_of_bounds() {
        let bridge = HardwareBridge::new(1920, 1080);
        // Direct call to the guard — no enigo interaction
        assert!(bridge.validate_coordinate(2000, 500).is_err());
        assert!(bridge.validate_coordinate(-1, 0).is_err());
        assert!(bridge.validate_coordinate(960, 540).is_ok());
    }

    #[tokio::test]
    async fn wait_action_does_not_need_coordinates() {
        let mut bridge = HardwareBridge::new(1920, 1080);
        let action = Action {
            action_type: ActionType::Wait(10),
            point: None,
            delay_ms: 0,
        };
        assert!(bridge.execute(&action).await.is_ok());
    }
}
