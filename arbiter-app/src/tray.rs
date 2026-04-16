//! tray.rs — System tray lifecycle for the Arbiter background service.
//!
//! Uses `tao` for the OS event loop and `tray-icon` for tray presence.
//! The tray is the *only* UI surface active at runtime.
//!
//! Tray menu items:
//!   • "Arbiter — Standing By"  (disabled status label)
//!   • "Open Forge"         (future: show the Forge Terminal)
//!   • separator
//!   • "Quit Arbiter"           (graceful shutdown)
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
    /// Reset requested via tray menu.
    Reset,
}

// ── Icon Builder ──────────────────────────────────────────────────────────────

/// Build and return the system tray icon and the status menu item handle.
///
/// The returned `TrayIcon` must be kept alive for the icon to remain visible.
pub fn build_tray() -> Result<(TrayIcon, MenuItem), Box<dyn std::error::Error>> {
    // Attempt to load the real icon.ico from the data directory
    let mut icon_path = std::path::Path::new("arbiter-data")
        .join("icon.ico");

    // Fallback for dev environment running from inside arbiter-app/
    if !icon_path.exists() {
        icon_path = std::path::Path::new("..").join("arbiter-data").join("icon.ico");
    }

    let icon = if icon_path.exists() {
        match image::open(icon_path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)?
            }
            Err(_) => build_fallback_icon()?,
        }
    } else {
        build_fallback_icon()?
    };

    let menu = Menu::new();
    let status_item = MenuItem::with_id("status", "Arbiter — Standing By", false, None);
    let reset_item = MenuItem::with_id("reset", "Reset Engine", true, None);
    let open_item = MenuItem::with_id("forge", "Open Forge", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit Arbiter", true, None);

    menu.append(&status_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&reset_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Arbiter — Standing By")
        .with_icon(icon)
        .build()?;

    info!("Tray icon built and visible");
    Ok((tray, status_item))
}

fn build_fallback_icon() -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for _ in 0..(16 * 16) {
        px.extend_from_slice(&[0x63, 0x66, 0xF1, 0xFF]); // Arbiter Accent Blue
    }
    Ok(tray_icon::Icon::from_rgba(px, 16, 16)?)
}

// ── Event Loop ────────────────────────────────────────────────────────────────

/// Run the tray event loop — **blocks the calling thread** until quit.
///
/// Must be called on the main thread (Windows COM / Cocoa requirement).
/// `on_quit` is a `FnOnce` consumed exactly once from whichever exit branch
/// fires first (menu Quit or engine-initiated Shutdown).
pub fn run_event_loop(on_event: impl Fn(TrayAppEvent, tao::event_loop::EventLoopProxy<TrayAppEvent>) + 'static) {
    use tao::event::Event;
    use tray_icon::menu::MenuEvent;
    use std::sync::{Arc, Mutex};

    let event_loop = EventLoopBuilder::<TrayAppEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Track spawned forge processes to kill them on exit
    let children = Arc::new(Mutex::new(Vec::<std::process::Child>::new()));

    // Build tray inside the event loop (Windows COM requirement).
    let (tray, status_item) = build_tray().expect("Failed to build system tray");

    info!("Arbiter tray event loop starting");

    let children_quit = children.clone();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        // ── Menu events ───────────────────────────────────────────────────────
        if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
            let id = menu_event.id.0.as_str();

            if id == "quit" {
                info!("Tray: Quit selected — killing children and shutting down");
                if let Ok(mut kids) = children_quit.lock() {
                    for mut child in kids.drain(..) {
                        let _ = child.kill();
                    }
                }
                on_event(TrayAppEvent::Shutdown, proxy.clone());
                *control_flow = ControlFlow::Exit;
                return;
            }

            if id == "reset" {
                info!("Tray: Reset requested");
                on_event(TrayAppEvent::Reset, proxy.clone());
            }

            if id == "forge" {
                info!("Tray: Spawning Forge user interface");
                
                let mut term_path = std::env::current_exe()
                    .unwrap_or_default()
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("arbiter-forge.exe");

                // Fallback for dev environment if not in the same folder (e.g. running via cargo)
                if !term_path.exists() {
                    let dev_path = std::path::Path::new("target").join("debug").join("arbiter-forge.exe");
                    if dev_path.exists() {
                        term_path = dev_path;
                    }
                }

                match std::process::Command::new(term_path).spawn() {
                    Ok(child) => {
                        if let Ok(mut kids) = children.lock() {
                            kids.push(child);
                        }
                    }
                    Err(e) => tracing::error!(%e, "Failed to spawn Forge process"),
                }
            }
        }

        // ── Engine → tray events ──────────────────────────────────────────────
        if let Event::UserEvent(app_event) = event {
            match app_event {
                TrayAppEvent::StatusUpdate(msg) => {
                    info!(%msg, "Tray: status update");
                    let _ = tray.set_tooltip(Some(format!("Arbiter — {}", msg)));
                    status_item.set_text(format!("Arbiter — {}", msg));
                }
                TrayAppEvent::Shutdown => {
                    info!("Tray: engine-initiated shutdown — killing children");
                    if let Ok(mut kids) = children_quit.lock() {
                        for mut child in kids.drain(..) {
                            let _ = child.kill();
                        }
                    }
                    on_event(TrayAppEvent::Shutdown, proxy.clone());
                    *control_flow = ControlFlow::Exit;
                }
                TrayAppEvent::Reset => {
                    on_event(TrayAppEvent::Reset, proxy.clone());
                }
            }
        }
    });
}
