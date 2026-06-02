// Tray-area icon management — stateless module-level API.
//
// Each enabled provider gets one notification-area icon. `sync` reconciles
// the set of registered icons with the supplied desired list, issuing
// `NIM_ADD`, `NIM_MODIFY`, or `NIM_DELETE` per icon. We track registration
// state in a private mutex so callers (the app orchestrator) don't have
// to thread a `Manager` through their snapshot/clone pipelines.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_INFOTIP_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

pub mod badge;
pub mod callback;

pub use crate::usage::ProviderId as IconKind;

/// Menu-command ID for the "Show widget" toggle that appears in the
/// right-click menu and is also fired by left-clicking a tray icon.
pub const IDM_TOGGLE_WIDGET: u16 = 50;

/// Notification message routed back to the owner HWND when the user
/// interacts with a tray icon (left/right click).
pub const WM_APP_TRAY: u32 = 0x8003;

#[derive(Clone, Debug)]
pub struct TrayIcon {
    pub kind: IconKind,
    pub percent: Option<f64>,
    pub tooltip: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    None,
    ToggleWidget,
    ShowContextMenu,
}

fn registered() -> &'static Mutex<HashSet<IconKind>> {
    static R: OnceLock<Mutex<HashSet<IconKind>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Reconcile registered icons with the desired list.
pub fn sync(owner: HWND, desired: &[TrayIcon]) {
    let mut current = registered().lock().expect("tray registry mutex poisoned");
    let target: HashSet<IconKind> = desired.iter().map(|i| i.kind).collect();

    let to_remove: Vec<IconKind> = current.difference(&target).copied().collect();
    for kind in to_remove {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &build_data(owner, kind));
        }
        current.remove(&kind);
    }

    for icon in desired {
        let hicon = badge::render_hicon(icon.kind, icon.percent);
        let mut data = build_data(owner, icon.kind);
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.hIcon = hicon;
        write_utf16(&mut data.szTip, &icon.tooltip);
        let msg = if current.contains(&icon.kind) {
            NIM_MODIFY
        } else {
            NIM_ADD
        };
        unsafe {
            if Shell_NotifyIconW(msg, &data).as_bool() && msg == NIM_ADD {
                current.insert(icon.kind);
            }
            let _ = DestroyIcon(hicon);
        }
    }
}

/// Show a yellow-warning balloon on an already-registered icon.
pub fn notify_warning(owner: HWND, kind: IconKind, title: &str, body: &str) {
    notify_inner(owner, kind, title, body, NIIF_WARNING);
}

/// Show a blue-info balloon on an already-registered icon.
pub fn notify_info(owner: HWND, kind: IconKind, title: &str, body: &str) {
    notify_inner(owner, kind, title, body, NIIF_INFO);
}

fn notify_inner(
    owner: HWND,
    kind: IconKind,
    title: &str,
    body: &str,
    flags: NOTIFY_ICON_INFOTIP_FLAGS,
) {
    let mut data = build_data(owner, kind);
    data.uFlags = NIF_INFO;
    write_utf16(&mut data.szInfoTitle, title);
    write_utf16(&mut data.szInfo, body);
    data.dwInfoFlags = flags;
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Tear down every registered icon. Call from app shutdown if you want to.
#[allow(dead_code)]
pub fn remove_all(owner: HWND) {
    let mut current = registered().lock().expect("tray registry mutex poisoned");
    for kind in current.drain().collect::<Vec<_>>() {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &build_data(owner, kind));
        }
    }
}

fn build_data(owner: HWND, kind: IconKind) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: owner,
        uID: icon_id(kind),
        uCallbackMessage: WM_APP_TRAY,
        ..Default::default()
    }
}

fn icon_id(kind: IconKind) -> u32 {
    kind.metadata().tray_icon_id
}

fn write_utf16(dst: &mut [u16], src: &str) {
    let units: Vec<u16> = src.encode_utf16().collect();
    let n = units.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&units[..n]);
    if n < dst.len() {
        dst[n] = 0;
    }
}
