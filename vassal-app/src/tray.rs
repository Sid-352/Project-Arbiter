//! tray.rs — System tray lifecycle for the Vassal background service.
//!
//! Uses `tao` for the OS event loop and `tray-icon` for tray presence.
//! The tray is the *only* UI surface active at runtime.
//!
//! Tray menu items:
//!   • "Vassal — Standing By"  (disabled status label)
//!   • "Open Terminal"         (future: show the iced Terminal of Commands)
//!   • separator
//!   • "Quit Vassal"           (graceful shutdown)
//!
//! The engine continues running when the terminal window is closed.
//! Quitting through the tray is the canonical shutdown path.

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tracing::info;
use tray_icon::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};

// ── Tray App Events ───────────────────────────────────────────────────────────

/// Events emitted from engine threads back into the tray event loop.
#[derive(Debug)]
#[allow(dead_code)]
pub enum TrayAppEvent {
    /// The engine wants to update the tray tooltip.
    StatusUpdate(String),
    /// Graceful shutdown requested by an engine thread.
    Shutdown,
}

// ── Icon Builder ──────────────────────────────────────────────────────────────

/// Build and return the system tray icon.
///
/// The returned `TrayIcon` must be kept alive for the icon to remain visible.
pub fn build_tray() -> Result<TrayIcon, Box<dyn std::error::Error>> {
    // Minimal 16×16 RGBA icon — accent-blue placeholder.
    // Replaced with a real .ico asset in the UI phase.
    let icon_rgba: Vec<u8> = {
        let mut px = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..(16 * 16) {
            px.extend_from_slice(&[0x00, 0x96, 0xFF, 0xFF]); // #0096FF accent
        }
        px
    };
    let icon = tray_icon::Icon::from_rgba(icon_rgba, 16, 16)?;

    let menu = Menu::new();
    let status_item = MenuItem::new("Vassal — Standing By", false, None);
    let open_item = MenuItem::new("Open Terminal", true, None);
    let quit_item = MenuItem::new("Quit Vassal", true, None);

    menu.append(&status_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Vassal — Standing By")
        .with_icon(icon)
        .build()?;

    info!("Tray icon built and visible");
    Ok(tray)
}

// ── Event Loop ────────────────────────────────────────────────────────────────

/// Run the tray event loop — **blocks the calling thread** until quit.
///
/// Must be called on the main thread (Windows COM / Cocoa requirement).
/// `on_quit` is a `FnOnce` consumed exactly once from whichever exit branch
/// fires first (menu Quit or engine-initiated Shutdown).
pub fn run_event_loop(on_quit: impl FnOnce() + 'static) {
    use tao::event::Event;
    use tray_icon::menu::MenuEvent;

    let event_loop = EventLoopBuilder::<TrayAppEvent>::with_user_event().build();

    // Build tray inside the event loop (Windows COM requirement).
    let _tray = build_tray().expect("Failed to build system tray");

    info!("Vassal tray event loop starting");

    // Wrap in Option so the FnOnce can be taken from either exit branch.
    let mut on_quit = Some(on_quit);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        // ── Menu events ───────────────────────────────────────────────────────
        if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
            let id = menu_event.id.0.as_str();

            if id.contains("Quit") {
                info!("Tray: Quit selected — initiating shutdown");
                if let Some(f) = on_quit.take() {
                    f();
                }
                *control_flow = ControlFlow::Exit;
                return;
            }

            if id.contains("Terminal") {
                info!("Tray: Open Terminal (deferred — UI phase)");
                // TODO: spawn iced Terminal of Commands window
            }
        }

        // ── Engine → tray events ──────────────────────────────────────────────
        if let Event::UserEvent(app_event) = event {
            match app_event {
                TrayAppEvent::StatusUpdate(msg) => {
                    info!(%msg, "Tray: status update");
                    // TODO: update tray tooltip via tray-icon API
                }
                TrayAppEvent::Shutdown => {
                    info!("Tray: engine-initiated shutdown");
                    if let Some(f) = on_quit.take() {
                        f();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
        }
    });
}
