#[cfg(feature = "vigil-sys")]
pub mod sys;

use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::decree::{EnvContext, Summons, WardConfig, WardLayer};

lazy_static::lazy_static! {
    static ref COOLDOWN_MAP: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
}

const DEBOUNCE_MS: u64 = 400;

fn is_debounced(signature: &str) -> bool {
    let mut map = match COOLDOWN_MAP.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("Vigil: COOLDOWN_MAP poisoned, recovering...");
            poisoned.into_inner()
        }
    };
    let now = Instant::now();

    if let Some(last_fire) = map.get(signature) {
        if now.duration_since(*last_fire).as_millis() < DEBOUNCE_MS as u128 {
            debug!(signature, "Vigil: dropping debounced event");
            return true;
        }
    }

    map.insert(signature.to_string(), now);

    if map.len() > 100 {
        map.retain(|_, v| now.duration_since(*v).as_millis() < 5000);
    }

    false
}

pub fn channel(capacity: usize) -> (mpsc::Sender<Summons>, mpsc::Receiver<Summons>) {
    mpsc::channel(capacity)
}

const STALE_EVENT_THRESHOLD_SECS: u64 = 5;

pub fn is_stale(event_age_secs: u64) -> bool {
    if event_age_secs > STALE_EVENT_THRESHOLD_SECS {
        warn!(
            event_age_secs,
            "Hibernation Guard: discarding stale Vigil event"
        );
        return true;
    }
    false
}

pub fn is_temp_file(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str()),
        Some("tmp" | "part" | "crdownload" | "download")
    )
}

pub fn is_write_complete(path: &str) -> bool {
    let size_a = std::fs::metadata(path).map(|m| m.len()).ok();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let size_b = std::fs::metadata(path).map(|m| m.len()).ok();
    match (size_a, size_b) {
        (Some(a), Some(b)) => {
            let stable = a == b && b > 0;
            debug!(path, size = b, stable, "Vigil-fs: successive size check");
            stable
        }
        _ => false,
    }
}

#[cfg(feature = "vigil-fs")]
pub mod fs {
    use super::*;
    use globset::GlobMatcher;
    use notify::{recommended_watcher, Event, EventKind, RecursiveMode, Watcher};
    use tokio::sync::broadcast;

    pub fn spawn_watcher(
        ward: WardConfig,
        filter: crate::filter::ArbiterFilter,
        tx: mpsc::Sender<Summons>,
    ) -> broadcast::Sender<()> {
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        let watch_path = ward.path.clone();
        let pattern = ward.pattern.clone();
        let analytical = ward.layer == WardLayer::Analytical;
        let recursive = ward.recursive;
        let ward_id = ward.id.clone();

        info!(%pattern, path = %watch_path.display(), analytical, recursive, "Vigil-fs: spawning watcher");

        let matcher: Option<GlobMatcher> = if !pattern.is_empty() {
            match globset::GlobBuilder::new(&pattern)
                .case_insensitive(true)
                .build()
            {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => {
                    warn!(%e, %pattern, "Vigil-fs: invalid pattern");
                    None
                }
            }
        } else {
            None
        };

        std::thread::spawn(move || {
            let (ntx, nrx) = std::sync::mpsc::channel::<notify::Result<Event>>();
            let mut watcher = match recommended_watcher(ntx) {
                Ok(w) => w,
                Err(e) => {
                    warn!(%e, "Vigil-fs: failed to initialise watcher");
                    return;
                }
            };

            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            let mut watching = false;
            let mut missing_logged = false;

            loop {
                if shutdown_rx.try_recv().is_ok() {
                    info!(%ward_id, "Vigil-fs: shutdown signal received, terminating watcher");
                    break;
                }

                if !watching {
                    if !watch_path.exists() {
                        if !missing_logged {
                            warn!(
                                path = %watch_path.display(),
                                ?mode,
                                "Vigil-fs: watch path unavailable at startup; retrying until available"
                            );
                            missing_logged = true;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    }

                    match watcher.watch(&watch_path, mode) {
                        Ok(_) => {
                            watching = true;
                            missing_logged = false;
                            info!(path = %watch_path.display(), ?mode, "Vigil-fs: watcher attached");
                        }
                        Err(e) => {
                            warn!(
                                %e,
                                path = %watch_path.display(),
                                ?mode,
                                "Vigil-fs: failed to attach watcher; retrying"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(1000));
                            continue;
                        }
                    }
                }

                match nrx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(Ok(event))
                        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) =>
                    {
                        let signet_config = crate::signet::load().unwrap_or_default();

                        for path in &event.paths {
                            let path_str = path.to_string_lossy().to_string();
                            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                            if path.is_dir() || filename.is_empty() || filter.is_own(&path_str) {
                                continue;
                            }

                            if crate::signet::is_path_restricted(&signet_config, path) {
                                continue; // Authoritative WARN is handled inside signet::is_path_restricted
                            }

                            if is_temp_file(&path_str) {
                                continue;
                            }

                            if let Some(ref m) = matcher {
                                if !m.is_match(filename) {
                                    continue;
                                }
                            }

                            let mut context = super::EnvContext::new();

                            context.insert("file_path", &path_str);
                            if let Some(parent) = path.parent() {
                                context.insert("file_dir", &parent.to_string_lossy());
                            }
                            context.insert("file_name", filename);
                            context.insert(
                                "file_ext",
                                path.extension().and_then(|e| e.to_str()).unwrap_or(""),
                            );

                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            context.insert("timestamp", &now_unix.to_string());
                            context.insert(
                                "timestamp_local",
                                &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string(),
                            );

                            if let Ok(meta) = std::fs::metadata(path) {
                                let bytes = meta.len();
                                context.insert("file_size", &bytes.to_string());
                                context.insert("file_size_human", &format_bytes(bytes));

                                if let Ok(created) = meta.created() {
                                    let unix = created
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    context.insert("file_created_unix", &unix.to_string());
                                    let dt: DateTime<Utc> =
                                        Utc.timestamp_opt(unix as i64, 0).unwrap();
                                    context.insert("file_created_iso", &dt.to_rfc3339());
                                    context.insert(
                                        "file_created_local",
                                        &dt.with_timezone(&chrono::Local)
                                            .format("%m/%d/%Y %I:%M %p")
                                            .to_string(),
                                    );
                                }
                                if let Ok(modified) = meta.modified() {
                                    let unix = modified
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    let dt: DateTime<Utc> =
                                        Utc.timestamp_opt(unix as i64, 0).unwrap();
                                    context.insert("file_modified_iso", &dt.to_rfc3339());
                                    context.insert(
                                        "file_modified_local",
                                        &dt.with_timezone(&chrono::Local)
                                            .format("%m/%d/%Y %I:%M %p")
                                            .to_string(),
                                    );
                                }

                                context.insert(
                                    "file_readonly",
                                    &meta.permissions().readonly().to_string(),
                                );

                                #[cfg(windows)]
                                {
                                    use std::os::windows::fs::MetadataExt;
                                    context.insert(
                                        "file_hidden",
                                        &((meta.file_attributes() & 0x2) != 0).to_string(),
                                    );
                                }
                            }

                            let is_link = std::fs::symlink_metadata(path)
                                .map(|m| m.file_type().is_symlink())
                                .unwrap_or(false);
                            context.insert("file_is_link", &is_link.to_string());

                            #[cfg(windows)]
                            if let Some(owner) = get_file_owner_windows(&path_str) {
                                context.insert("file_owner", &owner);
                            }

                            context.source_path = Some(path.clone());
                            context.integrity_scan = analytical;

                            let summons = Summons::FileCreated {
                                watch_path: watch_path.clone(),
                                pattern: pattern.clone(),
                                context,
                            };

                            let debounce_sig =
                                format!("{}|{}", summons.to_registry_key(), filename);
                            let path_str_check = path_str.clone();

                            let tx_clone = tx.clone();
                            std::thread::spawn(move || {
                                if !super::is_write_complete(&path_str_check) {
                                    return;
                                }
                                if is_debounced(&debounce_sig) {
                                    return;
                                }
                                let _ = tx_clone.blocking_send(summons);
                            });
                        }
                    }
                    Ok(_) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        shutdown_tx
    }

    fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1_024;
        const MB: u64 = 1_024 * KB;
        const GB: u64 = 1_024 * MB;
        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

    #[cfg(windows)]
    fn get_file_owner_windows(path: &str) -> Option<String> {
        use windows::core::{HSTRING, PWSTR};
        use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{
            LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
        };

        let path_w = HSTRING::from(path);
        let mut owner_sid = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();

        unsafe {
            // Step 1: obtain the owner SID.
            if GetNamedSecurityInfoW(
                &path_w,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner_sid),
                None,
                None,
                None,
                &mut sd,
            )
            .is_err()
            {
                return None;
            }

            // Step 2: resolve SID → name + domain.
            let mut name_len: u32 = 256;
            let mut domain_len: u32 = 256;
            let mut name_buf = vec![0u16; 256];
            let mut domain_buf = vec![0u16; 256];
            let mut sid_type = SID_NAME_USE::default();

            let looked_up = LookupAccountSidW(
                None,
                owner_sid,
                PWSTR(name_buf.as_mut_ptr()),
                &mut name_len,
                PWSTR(domain_buf.as_mut_ptr()),
                &mut domain_len,
                &mut sid_type,
            );

            // Step 3: release the security descriptor regardless of lookup outcome.
            // SECURITY_DESCRIPTORs from GetNamedSecurityInfoW must be freed with LocalFree.
            if !sd.0.is_null() {
                use windows::Win32::Foundation::LocalFree;
                let _ = LocalFree(windows::Win32::Foundation::HLOCAL(sd.0 as _));
            }

            if looked_up.is_err() {
                return None;
            }

            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let domain = String::from_utf16_lossy(&domain_buf[..domain_len as usize]);

            // Format identically to how Windows Explorer displays ownership.
            Some(if domain.is_empty() {
                name
            } else {
                format!("{}\\{}", domain, name)
            })
        }
    }
} // end pub mod fs

#[cfg(feature = "vigil-keys")]
pub mod keys {
    use super::*;

    pub enum HotkeyCommand {
        Register(String, tokio::sync::mpsc::Sender<Summons>),
    }

    lazy_static::lazy_static! {
        static ref HOTKEY_TX: std::sync::mpsc::Sender<HotkeyCommand> = {
            let (tx, rx) = std::sync::mpsc::channel::<HotkeyCommand>();
            std::thread::spawn(move || {
                use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager, GlobalHotKeyEvent};
                let manager = match GlobalHotKeyManager::new() {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!("Vigil-keys: failed to init manager: {:?}", e);
                        return;
                    }
                };

                let mut senders: std::collections::HashMap<u32, (String, tokio::sync::mpsc::Sender<Summons>)> = std::collections::HashMap::new();
                let receiver = GlobalHotKeyEvent::receiver();

                loop {
                    while let Ok(cmd) = rx.try_recv() {
                        match cmd {
                            HotkeyCommand::Register(combo, sum_tx) => {
                                if let Ok(hotkey) = combo.parse::<HotKey>() {
                                    if manager.register(hotkey).is_ok() {
                                        senders.insert(hotkey.id(), (combo.clone(), sum_tx));
                                        info!(%combo, "Vigil-keys: hotkey registered");
                                    } else {
                                        warn!(%combo, "Vigil-keys: failed to register hotkey");
                                    }
                                } else {
                                    warn!(%combo, "Vigil-keys: invalid hotkey string");
                                }
                            }
                        }
                    }

                    if let Ok(event) = receiver.try_recv() {
                        if event.state == global_hotkey::HotKeyState::Pressed {
                            if let Some((combo, sum_tx)) = senders.get(&event.id) {
                                let mut context = super::EnvContext::new();
                                context.insert("hotkey_combo", combo);
                                context.insert("timestamp", &format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
                                context.insert("timestamp_local", &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string());
                                let summons = super::Summons::Hotkey {
                                    combo: combo.clone(),
                                    context,
                                };

                                if !super::is_debounced(&summons.to_registry_key()) {
                                    let _ = sum_tx.blocking_send(summons);
                                }
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            });
            tx
        };
    }

    pub fn register_hotkey(
        combo: String,
        tx: tokio::sync::mpsc::Sender<Summons>,
    ) -> Result<(), String> {
        HOTKEY_TX
            .send(HotkeyCommand::Register(combo, tx))
            .map_err(|e| e.to_string())
    }
}

#[cfg(feature = "vigil-clipboard")]
pub mod clipboard {
    use super::*;
    use tokio::sync::broadcast;
    use windows::core::w;
    use windows::Win32::Foundation::*;
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn spawn_watcher(tx: mpsc::Sender<Summons>) -> broadcast::Sender<()> {
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);

        std::thread::spawn(move || {
            unsafe {
                let h_instance = GetModuleHandleW(None).unwrap_or_default();
                let window_class = w!("ArbiterClipboardWatcher");

                unsafe extern "system" fn wnd_proc(
                    hwnd: HWND,
                    msg: u32,
                    wparam: WPARAM,
                    lparam: LPARAM,
                ) -> LRESULT {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }

                let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: HINSTANCE(h_instance.0),
                    lpszClassName: window_class,
                    ..Default::default()
                };

                // Ignore error if already registered
                let _ = windows::Win32::UI::WindowsAndMessaging::RegisterClassW(&wc);

                let hwnd_res = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    window_class,
                    w!("ArbiterClipboard"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    HMENU::default(),
                    HINSTANCE(h_instance.0),
                    None,
                );

                let hwnd = match hwnd_res {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::error!(?e, "Vigil-clipboard: failed to create message window");
                        return;
                    }
                };

                if AddClipboardFormatListener(hwnd).is_err() {
                    tracing::error!("Vigil-clipboard: failed to add listener");
                    let _ = DestroyWindow(hwnd);
                    return;
                }

                let mut msg = MSG::default();
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        let _ = RemoveClipboardFormatListener(hwnd);
                        let _ = DestroyWindow(hwnd);
                        break;
                    }

                    while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
                        if msg.message == WM_CLIPBOARDUPDATE {
                            let mut context = EnvContext::new();
                            if let Some(content) = get_clipboard_text() {
                                context.insert("clipboard_content", &content);
                            }
                            context.insert(
                                "timestamp",
                                &format!(
                                    "{}",
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs()
                                ),
                            );
                            context.insert(
                                "timestamp_local",
                                &chrono::Local::now().format("%m/%d/%Y %I:%M %p").to_string(),
                            );

                            let summons = Summons::Clipboard { context };
                            if !is_debounced("Clipboard") {
                                let _ = tx.blocking_send(summons);
                            }
                        }
                        let _ = DispatchMessageW(&msg);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        });

        shutdown_tx
    }

    fn get_clipboard_text() -> Option<String> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return None;
            }

            // CF_UNICODETEXT is 13
            let handle_res = GetClipboardData(13);
            let result = if let Ok(h) = handle_res {
                let h_global = HGLOBAL(h.0);
                let ptr = windows::Win32::System::Memory::GlobalLock(h_global);
                if !ptr.is_null() {
                    let size = windows::Win32::System::Memory::GlobalSize(h_global);
                    let p_u16 = ptr as *const u16;
                    let max_len = size / 2; // size in bytes, each u16 is 2 bytes

                    let mut len = 0;
                    while len < max_len && *p_u16.add(len) != 0 {
                        len += 1;
                    }

                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(p_u16, len));
                    let _ = windows::Win32::System::Memory::GlobalUnlock(h_global);
                    Some(text)
                } else {
                    None
                }
            } else {
                None
            };

            let _ = CloseClipboard();
            result
        }
    }
}
