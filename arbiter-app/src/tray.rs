//! tray.rs — System tray lifecycle for the Arbiter background service.
//!
//! Uses `tao` for the OS event loop and `tray-icon` for tray presence.
//! The tray is the *only* UI surface active at runtime.
//!
//! Tray menu items:
//!   • "Pause Engine" / "Resume Engine"
//!   • "Reset Engine"
//!   • "Open Forge"
//!   • "Quit Arbiter" (graceful shutdown)
//!
//! The engine continues running when the terminal window is closed.
//! Quitting through the tray is the canonical shutdown path.

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use std::sync::{Arc, Mutex};
use tracing::info;
use tray_icon::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};

#[derive(Clone)]
struct TrayIcons {
    idle: tray_icon::Icon,
    executing: tray_icon::Icon,
}

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
    /// Pause or resume requested via tray menu.
    SetPaused(bool),
}

fn resolve_forge_path() -> std::path::PathBuf {
    let mut term_path = std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("arbiter-forge.exe");

    if !term_path.exists() {
        let dev_path = std::path::Path::new("target").join("debug").join("arbiter-forge.exe");
        if dev_path.exists() {
            term_path = dev_path;
        }
    }

    term_path
}

fn spawn_forge(children: &Arc<Mutex<Vec<std::process::Child>>>) {
    let term_path = resolve_forge_path();
    match std::process::Command::new(term_path).spawn() {
        Ok(child) => {
            if let Ok(mut kids) = children.lock() {
                kids.push(child);
            }
        }
        Err(e) => tracing::error!(%e, "Failed to spawn Forge process"),
    }
}

fn load_icon_from_bytes(icon_bytes: &[u8]) -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(icon_bytes)?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)?)
}

fn is_process_elevated() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

// ── Icon Builder ──────────────────────────────────────────────────────────────

/// Build and return the system tray icon and menu item handles.
///
/// The returned `TrayIcon` must be kept alive for the icon to remain visible.
fn build_tray() -> Result<(TrayIcon, MenuItem, TrayIcons), Box<dyn std::error::Error>> {
    // Embed the icon.ico from the data directory into the executable binary.
    // This ensures that the tray icon is *always* available, regardless of 
    // the working directory (e.g. when launched via Windows Startup registry).
    let icon_idle_bytes = include_bytes!("../../arbiter-data/icon.ico");
    let icon_exec_bytes = include_bytes!("../../arbiter-data/icon_dot.ico");

    let icon_idle = match load_icon_from_bytes(icon_idle_bytes) {
        Ok(icon) => icon,
        Err(e) => {
            tracing::error!(%e, "Failed to load embedded icon; using fallback");
            build_fallback_icon()?
        }
    };

    let icon_executing = match load_icon_from_bytes(icon_exec_bytes) {
        Ok(icon) => icon,
        Err(e) => {
            tracing::error!(%e, "Failed to load embedded dot icon; using idle icon");
            icon_idle.clone()
        }
    };

    let icons = TrayIcons {
        idle: icon_idle.clone(),
        executing: icon_executing,
    };

    let menu = Menu::new();
    let elevated = is_process_elevated();
    let status_text = if elevated { "Arbiter (Elevated)" } else { "Arbiter (Standard)" };
    let status_item = MenuItem::with_id("status", status_text, false, None);
    let pause_item = MenuItem::with_id("pause", "Pause Engine", true, None);
    let reset_item = MenuItem::with_id("reset", "Reset Engine", true, None);
    let open_item = MenuItem::with_id("forge", "Open Forge", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit Arbiter", true, None);

    menu.append(&status_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&pause_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&reset_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(if elevated { "Arbiter — Standing By (Elevated)" } else { "Arbiter — Standing By" })
        .with_icon(icon_idle)
        .build()?;

    info!("Tray icon built and visible");
    Ok((tray, pause_item, icons))
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

    let event_loop = EventLoopBuilder::<TrayAppEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Track spawned forge processes to kill them on exit
    let children = Arc::new(Mutex::new(Vec::<std::process::Child>::new()));

    // Build tray inside the event loop (Windows COM requirement).
    let (tray, pause_item, icons) = build_tray().expect("Failed to build system tray");
    let mut paused = false;

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

                let mut was_open = false;
                if let Ok(mut kids) = children.lock() {
                    was_open = !kids.is_empty();
                    for mut child in kids.drain(..) {
                        let _ = child.kill();
                    }
                }
                if was_open {
                    info!("Tray: Reset restarting Forge instance");
                    spawn_forge(&children);
                }
            }

            if id == "pause" {
                paused = !paused;
                let label = if paused { "Resume Engine" } else { "Pause Engine" };
                pause_item.set_text(label);
                info!(paused, "Tray: Pause state toggled");
                on_event(TrayAppEvent::SetPaused(paused), proxy.clone());
            }

            if id == "forge" {
                info!("Tray: Spawning Forge user interface");
                spawn_forge(&children);
            }
        }

        // ── Engine → tray events ──────────────────────────────────────────────
        if let Event::UserEvent(app_event) = event {
            match app_event {
                TrayAppEvent::StatusUpdate(msg) => {
                    info!(%msg, "Tray: status update");
                    let is_executing = msg.starts_with("Executing:");
                    let icon = if is_executing {
                        icons.executing.clone()
                    } else {
                        icons.idle.clone()
                    };
                    let _ = tray.set_icon(Some(icon));
                    let _ = tray.set_tooltip(Some(format!("Arbiter — {}", msg)));
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
                TrayAppEvent::SetPaused(is_paused) => {
                    paused = is_paused;
                    let label = if paused { "Resume Engine" } else { "Pause Engine" };
                    pause_item.set_text(label);
                }
            }
        }
    });
}
