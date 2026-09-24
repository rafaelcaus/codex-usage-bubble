// Floating rounded-card bubble window.
//
// Top-level window with WS_POPUP + WS_EX_LAYERED + WS_EX_TOPMOST + WS_EX_NOACTIVATE.
// Slightly rounded card (corner_radius = 10% of width) so wide captions are
// never pinched by curves. Top holds the progress ring (PRIMARY window =
// weekly quota on Pro) with "RESTA" + big remaining-% glyph. Below it, a
// single countdown caption: precise time left to the weekly reset.
//
// Painting is hybrid: tiny-skia renders the shape (AA fills + AA stroked arc)
// into a Pixmap; the Pixmap is copied byte-for-byte into a 32bpp BI_RGB DIB;
// GDI then overlays ClearType text on top; UpdateLayeredWindow blits the result
// to the screen with per-pixel alpha. WM_NCHITTEST returns HTCAPTION inside the
// stadium so the OS handles drag for free.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHAppBarMessage, ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETTASKBARPOS,
    APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::os::dpi::scale as scale_to_dpi;
use crate::os::{to_utf16_nul as wide_str, Rgb as Color};

const TIMER_FULLSCREEN_CHECK: usize = 5;
const TIMER_PULSE: usize = 6;
const TIMER_TIME_PROGRESS: usize = 7;
const PULSE_INTERVAL_MS: u32 = 80;
const TIME_PROGRESS_INTERVAL_MS: u32 = 60_000;
use crate::usage::ProviderId;

// ---------- Public types & API ----------

// Width clamps in logical pixels. Height is derived per width (see
// `bubble_height_logical`) — aspect tapers from 3:1 at the small end toward
// 2.6:1 at the large end so the bars look proportionally chunkier as the
// bubble grows.
    pub const MIN_BUBBLE_SIZE: i32 = 50;
pub const MAX_BUBBLE_SIZE: i32 = 360;
    pub const DEFAULT_BUBBLE_SIZE: i32 = 50;
pub const RESIZE_STEP_LOGICAL: i32 = 20;
const SNAP_ZONE_LOGICAL: i32 = 12;
const CORNER_SNAP_ZONE_LOGICAL: i32 = 32;
const CORNER_INSET_LOGICAL: i32 = 12;
const TASKBAR_GAP_LOGICAL: i32 = 4;
const PEER_ALIGN_TOLERANCE_LOGICAL: i32 = 8;
const CLASS_NAME: &str = "ClaudeCodeUsageBubble";
const FULLSCREEN_POLL_MS: u32 = 350;
const FULLSCREEN_EDGE_TOLERANCE_PX: i32 = 2;
const FIVE_HOURS_SECS: u64 = 5 * 60 * 60;
const SEVEN_DAYS_SECS: u64 = 7 * 24 * 60 * 60;

pub struct BubbleConfig {
    pub model: ProviderId,
    pub size_logical: i32,
    pub position: Option<(i32, i32)>,
    pub session_pct: Option<f64>,
    pub session_text: String,
    pub session_resets_at: Option<SystemTime>,
    pub weekly_pct: Option<f64>,
    pub weekly_text: String,
    pub weekly_resets_at: Option<SystemTime>,
    pub is_dark: bool,
}

/// Fork (Rafael): minimal vertical card — ring + ONE countdown caption.
/// Height is computed from the exact same content math as the layout below
/// (in logical units), so the window always fits its content at any DPI.
fn bubble_height_logical(width_logical: i32) -> i32 {
    let pad = width_logical * 6 / 100;
    let ring = width_logical - 2 * pad;
    let big = (ring * 24 / 100).max(4);
    let small = ((big * 40) / 100).max(3);
    let cap = small + 5;
    let g1 = (ring * 8 / 100).max(2);
    pad + ring + g1 + cap + pad + 2
}

#[derive(Clone, Copy)]
enum UsageWindowKind {
    Primary,
    Secondary,
}

fn window_duration_secs(model: ProviderId, window: UsageWindowKind) -> u64 {
    // Claude exposes 5h/7d directly. Codex exposes primary/secondary fields;
    // the product maps those to the same short/long windows in the compact UI.
    match (model, window) {
        (ProviderId::Claude, UsageWindowKind::Primary) => FIVE_HOURS_SECS,
        (ProviderId::Claude, UsageWindowKind::Secondary) => SEVEN_DAYS_SECS,
        (ProviderId::ChatGpt, UsageWindowKind::Primary) => FIVE_HOURS_SECS,
        (ProviderId::ChatGpt, UsageWindowKind::Secondary) => SEVEN_DAYS_SECS,
        (ProviderId::OpenCodeGo, UsageWindowKind::Primary) => SEVEN_DAYS_SECS,
        (ProviderId::OpenCodeGo, UsageWindowKind::Secondary) => 30 * 24 * 60 * 60,
    }
}

fn remaining_fraction(resets_at: Option<SystemTime>, duration_secs: u64) -> Option<f32> {
    let reset = resets_at?;
    let remaining = reset
        .duration_since(SystemTime::now())
        .unwrap_or_else(|_| Duration::from_secs(0));
    Some((remaining.as_secs_f64() / duration_secs as f64).clamp(0.0, 1.0) as f32)
}

/// Owner-supplied event callbacks. The bubble window proc is a leaf — it
/// doesn't know about `app`. The owner installs these once at startup so the
/// proc can dispatch UI events back without an upward `crate::app::` reach.
pub struct Callbacks {
    pub on_click: fn(HWND, ProviderId),
    pub on_right_click: fn(HWND, ProviderId, POINT),
    pub on_moved: fn(ProviderId, (i32, i32)),
    pub on_resized: fn(ProviderId, i32),
    pub on_menu_command: fn(u32, HWND),
    pub on_settings_changed: fn(),
}

static CALLBACKS: OnceLock<Callbacks> = OnceLock::new();

/// Install the owner's callbacks. Called once by `app::run` before any
/// bubble is created. Subsequent calls are silently ignored.
pub fn install_callbacks(cb: Callbacks) {
    let _ = CALLBACKS.set(cb);
}

fn dispatch<F: FnOnce(&Callbacks)>(f: F) {
    if let Some(cb) = CALLBACKS.get() {
        f(cb);
    } else {
        log::warn!("bubble event dispatched before install_callbacks; event dropped");
    }
}

/// Register the bubble window class. Idempotent; safe to call before the first
/// `create()` from the UI thread.
pub fn register_class() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| unsafe {
        let class_w = wide_str(CLASS_NAME);
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_SIZEALL).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_w.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            log::error!("bubble RegisterClassExW returned 0");
        }
    });
}

/// Create a bubble window. Returns the HWND. The caller (app::run) owns the
/// message-loop dispatch.
pub fn create(config: BubbleConfig) -> HWND {
    register_class();
    let initial_size_logical = config.size_logical.clamp(MIN_BUBBLE_SIZE, MAX_BUBBLE_SIZE);
    let dpi_for_create = crate::os::dpi::for_system();
    let width_px = scale_to_dpi(initial_size_logical, dpi_for_create);
    let height_px = scale_to_dpi(bubble_height_logical(initial_size_logical), dpi_for_create);
    let (x, y) = config
        .position
        .unwrap_or_else(|| default_position(width_px, height_px, config.model));
    let hwnd = unsafe {
        let class_w = wide_str(CLASS_NAME);
        let title_w = wide_str("Claude Code Usage Bubble");
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_w.as_ptr()),
            PCWSTR::from_raw(title_w.as_ptr()),
            WS_POPUP,
            x,
            y,
            width_px,
            height_px,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .unwrap_or_default()
    };

    if hwnd == HWND::default() {
        log::error!("bubble CreateWindowExW failed");
        return hwnd;
    }

    // Embed app icon in window non-client (mostly cosmetic; toolwindows
    // don't show captions but the icon helps in dev tooling). The HICONs
    // are extracted once at process startup and reused across every bubble
    // create() so we don't leak a pair per toggle cycle.
    let (large_icon, small_icon) = app_icons();
    unsafe {
        if !large_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_BIG as usize),
                LPARAM(large_icon.0 as isize),
            );
        }
        if !small_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(small_icon.0 as isize),
            );
        }
    }

    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
    lock_bubbles().insert(
        hwnd.0 as isize,
        BubbleState {
            model: config.model,
            size_logical: initial_size_logical,
            dpi,
            session_pct: config.session_pct,
            session_text: config.session_text,
            session_resets_at: config.session_resets_at,
            weekly_pct: config.weekly_pct,
            weekly_text: config.weekly_text,
            weekly_resets_at: config.weekly_resets_at,
            is_dark: config.is_dark,
            drag_start_pos: None,
            hidden_by_fullscreen: false,
            user_hidden: false,
            hidden_by_focus: false,
            focus_miss_count: 0,
            pulse_phase: 0,
            pulse_timer_armed: false,
            time_progress_timer_armed: false,
        },
    );

    log::info!(
        "bubble create model={:?} pos=({x},{y}) size={width_px}x{height_px} dpi={dpi}",
        config.model
    );

    // Defense in depth: settings::load already validates positions against
    // currently-connected monitors, but a monitor unplug between load and
    // create (or a partially-off-screen saved position) is still possible.
    clamp_into_work_area(hwnd);

    render(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Periodic fullscreen-foreground check.
        SetTimer(hwnd, TIMER_FULLSCREEN_CHECK, FULLSCREEN_POLL_MS, None);
    }

    hwnd
}

pub fn destroy(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(hwnd, TIMER_FULLSCREEN_CHECK);
        let _ = KillTimer(hwnd, TIMER_PULSE);
        let _ = KillTimer(hwnd, TIMER_TIME_PROGRESS);
        let _ = DestroyWindow(hwnd);
    }
}

/// Extract the EXE's own icon pair once per process. Stored as raw pointer
/// values because `HICON` is `!Send`/`!Sync`; reconstituted for each caller.
/// The pair is intentionally never destroyed — Windows tears them down on
/// process exit, and one pair per process is bounded leak rather than the
/// O(bubble-toggles) leak we'd get from extracting per `create()`.
fn app_icons() -> (HICON, HICON) {
    static ICONS: OnceLock<(isize, isize)> = OnceLock::new();
    let (big, small) = *ICONS.get_or_init(|| unsafe {
        let mut large = HICON::default();
        let mut small = HICON::default();
        let mut exe = [0u16; 260];
        GetModuleFileNameW(HMODULE::default(), &mut exe);
        let _ = ExtractIconExW(
            PCWSTR::from_raw(exe.as_ptr()),
            0,
            Some(&mut large),
            Some(&mut small),
            1,
        );
        if large.is_invalid() && small.is_invalid() {
            log::warn!("ExtractIconExW yielded null handles; bubbles will be iconless");
        }
        (large.0 as isize, small.0 as isize)
    });
    (HICON(big as *mut _), HICON(small as *mut _))
}

pub fn update_data(
    hwnd: HWND,
    session_pct: Option<f64>,
    session_text: String,
    session_resets_at: Option<SystemTime>,
    weekly_pct: Option<f64>,
    weekly_text: String,
    weekly_resets_at: Option<SystemTime>,
) {
    {
        let mut bubbles = lock_bubbles();
        let Some(b) = bubbles.get_mut(&(hwnd.0 as isize)) else {
            return;
        };
        b.session_pct = session_pct;
        b.session_text = session_text;
        b.session_resets_at = session_resets_at;
        b.weekly_pct = weekly_pct;
        b.weekly_text = weekly_text;
        b.weekly_resets_at = weekly_resets_at;
    }
    sync_pulse_timer(hwnd);
    sync_time_progress_timer(hwnd);
    render(hwnd);
}

fn any_pct_in_alarm(b: &BubbleState) -> bool {
    b.session_pct.is_some_and(|p| p >= 95.0) || b.weekly_pct.is_some_and(|p| p >= 95.0)
}

fn sync_pulse_timer(hwnd: HWND) {
    let (should_be_armed, currently_armed) = {
        let bubbles = lock_bubbles();
        let Some(b) = bubbles.get(&(hwnd.0 as isize)) else {
            return;
        };
        (any_pct_in_alarm(b), b.pulse_timer_armed)
    };
    if should_be_armed == currently_armed {
        return;
    }
    unsafe {
        if should_be_armed {
            SetTimer(hwnd, TIMER_PULSE, PULSE_INTERVAL_MS, None);
        } else {
            let _ = KillTimer(hwnd, TIMER_PULSE);
        }
    }
    if let Some(b) = lock_bubbles().get_mut(&(hwnd.0 as isize)) {
        b.pulse_timer_armed = should_be_armed;
        if !should_be_armed {
            b.pulse_phase = 0;
        }
    }
}

fn sync_time_progress_timer(hwnd: HWND) {
    let (should_be_armed, currently_armed) = {
        let bubbles = lock_bubbles();
        let Some(b) = bubbles.get(&(hwnd.0 as isize)) else {
            return;
        };
        (
            b.session_resets_at.is_some() || b.weekly_resets_at.is_some(),
            b.time_progress_timer_armed,
        )
    };
    if should_be_armed == currently_armed {
        return;
    }
    unsafe {
        if should_be_armed {
            SetTimer(hwnd, TIMER_TIME_PROGRESS, TIME_PROGRESS_INTERVAL_MS, None);
        } else {
            let _ = KillTimer(hwnd, TIMER_TIME_PROGRESS);
        }
    }
    if let Some(b) = lock_bubbles().get_mut(&(hwnd.0 as isize)) {
        b.time_progress_timer_armed = should_be_armed;
    }
}

pub fn update_dark_mode(hwnd: HWND, is_dark: bool) {
    {
        let mut bubbles = lock_bubbles();
        let Some(b) = bubbles.get_mut(&(hwnd.0 as isize)) else {
            return;
        };
        b.is_dark = is_dark;
    }
    render(hwnd);
}

pub fn set_user_visible(hwnd: HWND, visible: bool) {
    {
        let mut bubbles = lock_bubbles();
        let Some(b) = bubbles.get_mut(&(hwnd.0 as isize)) else {
            return;
        };
        b.user_hidden = !visible;
    }
    unsafe {
        let cmd = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        let _ = ShowWindow(hwnd, cmd);
    }
    // A layered window's composited surface is dropped while hidden, so
    // ShowWindow(SW_SHOWNOACTIVATE) on its own renders blank until the next
    // UpdateLayeredWindow. The cached BubbleState (pcts + texts) hasn't gone
    // anywhere, so just re-paint from it so the bubble pops back with the
    // last good data instead of empty placeholders.
    if visible {
        render(hwnd);
    }
}

pub fn position(hwnd: HWND) -> Option<(i32, i32)> {
    let mut r = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut r).is_err() {
            return None;
        }
    }
    Some((r.left, r.top))
}

pub fn model(hwnd: HWND) -> Option<ProviderId> {
    lock_bubbles().get(&(hwnd.0 as isize)).map(|b| b.model)
}

pub fn size_logical(hwnd: HWND) -> Option<i32> {
    lock_bubbles()
        .get(&(hwnd.0 as isize))
        .map(|b| b.size_logical)
}

// ---------- State ----------

struct BubbleState {
    model: ProviderId,
    size_logical: i32,
    dpi: u32,
    session_pct: Option<f64>,
    session_text: String,
    session_resets_at: Option<SystemTime>,
    weekly_pct: Option<f64>,
    weekly_text: String,
    weekly_resets_at: Option<SystemTime>,
    is_dark: bool,
    drag_start_pos: Option<(i32, i32)>,
    hidden_by_fullscreen: bool,
    user_hidden: bool,
    /// Fork (Rafael): focus-follow ("Somente sobre o ChatGPT") hid the bubble
    /// because the foreground window is not the ChatGPT app nor our own UI.
    hidden_by_focus: bool,
    /// Fork: consecutive foreground ticks NOT on ChatGPT. Hides only after
    /// 2 in a row so a split-second focus steal (alt+tab transit, toast)
    /// doesn't flicker the bubble.
    focus_miss_count: u8,
    /// Frame counter for the ≥95% pulse animation. Increments on each
    /// TIMER_PULSE tick when at least one bar is in the alarm band.
    pulse_phase: u32,
    /// Whether TIMER_PULSE is currently armed for this bubble.
    pulse_timer_armed: bool,
    /// Whether TIMER_TIME_PROGRESS is armed to keep reset-time visuals current.
    time_progress_timer_armed: bool,
}

fn bubbles() -> &'static Mutex<HashMap<isize, BubbleState>> {
    static BUBBLES: OnceLock<Mutex<HashMap<isize, BubbleState>>> = OnceLock::new();
    BUBBLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_bubbles() -> MutexGuard<'static, HashMap<isize, BubbleState>> {
    bubbles().lock().expect("bubble state mutex poisoned")
}

// ---------- Window proc ----------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => hit_test(hwnd, lparam),
        WM_ENTERSIZEMOVE => {
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            if let Some(b) = lock_bubbles().get_mut(&(hwnd.0 as isize)) {
                b.drag_start_pos = Some((r.left, r.top));
            }
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            // WM_NCLBUTTONUP isn't reliably delivered for HTCAPTION drags; instead
            // we infer click-vs-drag from whether the window actually moved.
            let start = {
                let mut bubbles = lock_bubbles();
                let start = bubbles
                    .get(&(hwnd.0 as isize))
                    .and_then(|b| b.drag_start_pos);
                if let Some(b) = bubbles.get_mut(&(hwnd.0 as isize)) {
                    b.drag_start_pos = None;
                }
                start
            };
            let mut current = RECT::default();
            let _ = GetWindowRect(hwnd, &mut current);
            let moved = match start {
                // Fork: 8px instead of 3px — tiny accidental pointer slips on
                // a quick click must still count as a click (opens the panel),
                // not as a drag.
                Some((sx, sy)) => (current.left - sx).abs() >= 8 || (current.top - sy).abs() >= 8,
                None => false,
            };
            if moved {
                snap_to_edge(hwnd);
                if let Some(model) = model(hwnd) {
                    if let Some(pos) = position(hwnd) {
                        dispatch(|cb| (cb.on_moved)(model, pos));
                    }
                }
            } else if let Some(model) = model(hwnd) {
                dispatch(|cb| (cb.on_click)(hwnd, model));
            }
            LRESULT(0)
        }
        WM_NCRBUTTONUP => {
            if let Some(model) = model(hwnd) {
                let pt = lparam_to_point(lparam);
                dispatch(|cb| (cb.on_right_click)(hwnd, model, pt));
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let modifiers = (wparam.0 & 0xFFFF) as u32;
            const MK_CONTROL: u32 = 0x0008;
            if modifiers & MK_CONTROL != 0 {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
                let step = if delta > 0 {
                    RESIZE_STEP_LOGICAL
                } else {
                    -RESIZE_STEP_LOGICAL
                };
                resize_step(hwnd, step);
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_DPICHANGED => {
            let new_dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if let Some(b) = lock_bubbles().get_mut(&(hwnd.0 as isize)) {
                b.dpi = new_dpi;
            }
            let rect_ptr = lparam.0 as *const RECT;
            if !rect_ptr.is_null() {
                let r = *rect_ptr;
                let _ = SetWindowPos(
                    hwnd,
                    HWND::default(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            render(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                w if w == TIMER_FULLSCREEN_CHECK => check_fullscreen(hwnd),
                w if w == TIMER_PULSE => {
                    if let Some(b) = lock_bubbles().get_mut(&(hwnd.0 as isize)) {
                        b.pulse_phase = b.pulse_phase.wrapping_add(1);
                    }
                    render(hwnd);
                }
                w if w == TIMER_TIME_PROGRESS => render(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            dispatch(|cb| (cb.on_menu_command)(wparam.0 as u32, hwnd));
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Taskbar move / auto-hide toggle / DPI change / theme toggle
            // all post this. Re-clamp into the new work area (bubble must
            // not end up hidden behind the new taskbar position) and ask
            // the app to re-read the light/dark setting — Windows fires
            // this message when the user flips the OS theme in Settings.
            clamp_into_work_area(hwnd);
            dispatch(|cb| (cb.on_settings_changed)());
            LRESULT(0)
        }
        WM_DESTROY => {
            lock_bubbles().remove(&(hwnd.0 as isize));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let pt = lparam_to_point(lparam);
    let mut r = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut r).is_err() {
            return LRESULT(HTNOWHERE as isize);
        }
    }
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    let radius = corner_radius_px(w, h);
    // Local coordinates relative to top-left of the bubble.
    let lx = pt.x - r.left;
    let ly = pt.y - r.top;
    if point_in_rounded_rect(lx, ly, w, h, radius) {
        LRESULT(HTCAPTION as isize)
    } else {
        LRESULT(HTTRANSPARENT as isize)
    }
}

fn corner_radius_px(w: i32, h: i32) -> i32 {
    // Fork: match the painted shape (10% rounded card, not a full pill),
    // so clicks/hit-testing agree with the visible outline. `h` unused.
    let _ = h;
    (w * 10 / 100).max(1)
}

fn point_in_rounded_rect(x: i32, y: i32, w: i32, h: i32, r: i32) -> bool {
    if x < 0 || y < 0 || x >= w || y >= h {
        return false;
    }
    // The straight horizontal and vertical strips are always inside; only the
    // four corner squares need the circular falloff check.
    let in_x_strip = x >= r && x < w - r;
    let in_y_strip = y >= r && y < h - r;
    if in_x_strip || in_y_strip {
        return true;
    }
    let cx = if x < r { r } else { w - 1 - r };
    let cy = if y < r { r } else { h - 1 - r };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn lparam_to_point(lparam: LPARAM) -> POINT {
    let lo = (lparam.0 & 0xFFFF) as i16 as i32;
    let hi = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    POINT { x: lo, y: hi }
}

// ---------- Resize / snap ----------

fn resize_step(hwnd: HWND, delta: i32) {
    let Some(current) = size_logical(hwnd) else {
        return;
    };
    set_size_logical(hwnd, current + delta);
}

pub fn set_size_logical(hwnd: HWND, size_logical: i32) {
    let (new_logical, dpi) = {
        let mut bubbles = lock_bubbles();
        let Some(b) = bubbles.get_mut(&(hwnd.0 as isize)) else {
            return;
        };
        let new_logical = size_logical.clamp(MIN_BUBBLE_SIZE, MAX_BUBBLE_SIZE);
        if new_logical == b.size_logical {
            return;
        }
        b.size_logical = new_logical;
        (new_logical, b.dpi)
    };
    let width_px = scale_to_dpi(new_logical, dpi);
    let height_px = scale_to_dpi(bubble_height_logical(new_logical), dpi);
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
        // Resize centered on existing center.
        let cx = (r.left + r.right) / 2;
        let cy = (r.top + r.bottom) / 2;
        let new_x = cx - width_px / 2;
        let new_y = cy - height_px / 2;
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            new_x,
            new_y,
            width_px,
            height_px,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    render(hwnd);
    if let Some(m) = model(hwnd) {
        dispatch(|cb| (cb.on_resized)(m, new_logical));
    }
}

fn snap_to_edge(hwnd: HWND) {
    let dpi = lock_bubbles()
        .get(&(hwnd.0 as isize))
        .map(|b| b.dpi)
        .unwrap_or(96);
    let edge_zone = scale_to_dpi(SNAP_ZONE_LOGICAL, dpi);
    let corner_zone = scale_to_dpi(CORNER_SNAP_ZONE_LOGICAL, dpi);
    let corner_inset = scale_to_dpi(CORNER_INSET_LOGICAL, dpi);
    let taskbar_gap = scale_to_dpi(TASKBAR_GAP_LOGICAL, dpi);
    let peer_tolerance = scale_to_dpi(PEER_ALIGN_TOLERANCE_LOGICAL, dpi);

    let mut r = RECT::default();
    let monitor;
    unsafe {
        if GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    }
    if monitor.is_invalid() {
        return;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
    }
    let wa = info.rcWork;
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    let mut nx = r.left;
    let mut ny = r.top;

    // 1. Corner snap — if the bubble's nearest-corner distance is under the
    //    32-px corner zone, slam it into the corner with the 12-px inset.
    let snapped_to_corner = try_corner_snap(&mut nx, &mut ny, &wa, w, h, corner_zone, corner_inset);

    if !snapped_to_corner {
        // 2. Edge snap (existing behavior) — also handles taskbar-adjacency
        //    when the taskbar steals from the work area on the same edge.
        let taskbar = read_taskbar();
        snap_to_work_area_edges(&mut nx, &mut ny, &wa, w, h, edge_zone);
        if let Some(tb) = taskbar {
            snap_alongside_taskbar(&mut nx, &mut ny, &tb, &wa, w, h, edge_zone, taskbar_gap);
        }

        // 3. Peer vertical alignment — when the other bubble is within ±8 px
        //    on Y, snap the dragged bubble to share its baseline.
        align_with_peer(hwnd, &mut ny, peer_tolerance);
    }

    // Clamp into the work area in any case (so the bubble can't be lost off-screen).
    nx = nx.clamp(wa.left, (wa.right - w).max(wa.left));
    ny = ny.clamp(wa.top, (wa.bottom - h).max(wa.top));

    if nx != r.left || ny != r.top {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                nx,
                ny,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn try_corner_snap(
    nx: &mut i32,
    ny: &mut i32,
    wa: &RECT,
    w: i32,
    h: i32,
    zone: i32,
    inset: i32,
) -> bool {
    // Distance from each work-area corner to the bubble's nearest corner.
    let tl = (*nx - wa.left).abs() + (*ny - wa.top).abs();
    let tr = (wa.right - (*nx + w)).abs() + (*ny - wa.top).abs();
    let bl = (*nx - wa.left).abs() + (wa.bottom - (*ny + h)).abs();
    let br = (wa.right - (*nx + w)).abs() + (wa.bottom - (*ny + h)).abs();
    let min = tl.min(tr).min(bl).min(br);
    if min > zone * 2 {
        return false;
    }
    if min == tl {
        *nx = wa.left + inset;
        *ny = wa.top + inset;
    } else if min == tr {
        *nx = wa.right - inset - w;
        *ny = wa.top + inset;
    } else if min == bl {
        *nx = wa.left + inset;
        *ny = wa.bottom - inset - h;
    } else {
        *nx = wa.right - inset - w;
        *ny = wa.bottom - inset - h;
    }
    true
}

fn snap_to_work_area_edges(nx: &mut i32, ny: &mut i32, wa: &RECT, w: i32, h: i32, zone: i32) {
    if (*nx - wa.left).abs() < zone {
        *nx = wa.left;
    } else if (wa.right - (*nx + w)).abs() < zone {
        *nx = wa.right - w;
    }
    if (*ny - wa.top).abs() < zone {
        *ny = wa.top;
    } else if (wa.bottom - (*ny + h)).abs() < zone {
        *ny = wa.bottom - h;
    }
}

struct Taskbar {
    rect: RECT,
    edge: u32,
}

fn read_taskbar() -> Option<Taskbar> {
    let mut abd = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        ..Default::default()
    };
    let res = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) };
    if res == 0 {
        return None;
    }
    Some(Taskbar {
        rect: abd.rc,
        edge: abd.uEdge,
    })
}

fn snap_alongside_taskbar(
    nx: &mut i32,
    ny: &mut i32,
    tb: &Taskbar,
    wa: &RECT,
    w: i32,
    h: i32,
    zone: i32,
    gap: i32,
) {
    // Only snap on the taskbar's docked edge. The bubble docks against the
    // inner face of the taskbar with a 4-px gap so it visually leans on it.
    match tb.edge {
        e if e == ABE_BOTTOM => {
            let target = tb.rect.top - gap - h;
            if (*ny - target).abs() < zone {
                *ny = target.max(wa.top);
            }
        }
        e if e == ABE_TOP => {
            let target = tb.rect.bottom + gap;
            if (*ny - target).abs() < zone {
                *ny = target.min(wa.bottom - h);
            }
        }
        e if e == ABE_LEFT => {
            let target = tb.rect.right + gap;
            if (*nx - target).abs() < zone {
                *nx = target.min(wa.right - w);
            }
        }
        e if e == ABE_RIGHT => {
            let target = tb.rect.left - gap - w;
            if (*nx - target).abs() < zone {
                *nx = target.max(wa.left);
            }
        }
        _ => {}
    }
}

fn clamp_into_work_area(hwnd: HWND) {
    let mut r = RECT::default();
    let monitor;
    unsafe {
        if GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    }
    if monitor.is_invalid() {
        return;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
    }
    let wa = info.rcWork;
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    let nx = r.left.clamp(wa.left, (wa.right - w).max(wa.left));
    let mut ny = r.top.clamp(wa.top, (wa.bottom - h).max(wa.top));

    // When both bubbles get clamped to the same bottom-right corner (e.g.,
    // saved positions were on a disconnected monitor and the validator missed
    // them), keep the Codex-above-Claude stagger that `default_position` uses
    // so they don't visually stack.
    let is_codex = lock_bubbles()
        .get(&(hwnd.0 as isize))
        .is_some_and(|b| matches!(b.model, ProviderId::ChatGpt));
    if is_codex && nx == wa.right - w && ny == wa.bottom - h {
        const STAGGER_GAP: i32 = 24;
        ny = (ny - h - STAGGER_GAP).max(wa.top);
    }

    if nx != r.left || ny != r.top {
        log::warn!(
            "clamp_into_work_area moved bubble from ({}, {}) to ({nx}, {ny})",
            r.left,
            r.top
        );
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                nx,
                ny,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn align_with_peer(this_hwnd: HWND, ny: &mut i32, tolerance: i32) {
    let bubbles = lock_bubbles();
    for (id, _) in bubbles.iter() {
        if *id == this_hwnd.0 as isize {
            continue;
        }
        let peer_hwnd = HWND(*id as *mut c_void);
        let mut pr = RECT::default();
        unsafe {
            if GetWindowRect(peer_hwnd, &mut pr).is_err() {
                continue;
            }
        }
        if (*ny - pr.top).abs() <= tolerance {
            *ny = pr.top;
            return;
        }
    }
}

// ---------- Fullscreen detection ----------

fn check_fullscreen(bubble_hwnd: HWND) {
    let fg = unsafe { GetForegroundWindow() };
    let evaluation = evaluate_foreground_fullscreen(fg, bubble_hwnd);

    let (was_hidden_by_fs, user_hidden) = {
        let bubbles = lock_bubbles();
        let Some(b) = bubbles.get(&(bubble_hwnd.0 as isize)) else {
            return;
        };
        (b.hidden_by_fullscreen, b.user_hidden)
    };

    if evaluation.is_fullscreen && !was_hidden_by_fs {
        unsafe {
            let _ = ShowWindow(bubble_hwnd, SW_HIDE);
        }
        if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
            b.hidden_by_fullscreen = true;
        }
        log_fullscreen_decision("hide", &evaluation, false);
    } else if !evaluation.is_fullscreen && was_hidden_by_fs {
        show_after_fullscreen(bubble_hwnd, user_hidden);
        log_fullscreen_decision("show", &evaluation, user_hidden);
    }

    check_focus_follow(bubble_hwnd, fg);
}

// ---------- Fork (Rafael): focus-follow ("Somente sobre o ChatGPT") ----------

/// Master switch, pushed from `app` (settings + Settings-menu toggle).
static ONLY_OVER_CHATGPT: AtomicBool = AtomicBool::new(true);

pub fn set_only_over_chatgpt(enabled: bool) {
    ONLY_OVER_CHATGPT.store(enabled, Ordering::Relaxed);
}

/// File name of the ChatGPT desktop app (which also hosts Codex mode).
const CHATGPT_EXE: &str = "ChatGPT.exe";

/// Runs on the same 0.35s foreground timer as the fullscreen check: hides the
/// bubble while the user works in any other app, shows it again when ChatGPT
/// (or our own bubble/panel/menu, which share our PID) takes the foreground.
fn check_focus_follow(bubble_hwnd: HWND, fg: HWND) {
    if !ONLY_OVER_CHATGPT.load(Ordering::Relaxed) {
        // Feature off: release any focus-hide so the bubble comes back.
        let needs_show = {
            let mut bubbles = lock_bubbles();
            match bubbles.get_mut(&(bubble_hwnd.0 as isize)) {
                Some(b) if b.hidden_by_focus => {
                    b.hidden_by_focus = false;
                    !b.hidden_by_fullscreen && !b.user_hidden
                }
                _ => false,
            }
        };
        if needs_show {
            unsafe {
                let _ = ShowWindow(bubble_hwnd, SW_SHOWNOACTIVATE);
            }
            render(bubble_hwnd);
        }
        return;
    }

    let chatgpt_fg = foreground_is_chatgpt_or_own(bubble_hwnd, fg);
    let (hidden_by_focus, hidden_by_fs, user_hidden, miss_count) = {
        let bubbles = lock_bubbles();
        match bubbles.get(&(bubble_hwnd.0 as isize)) {
            Some(b) => (
                b.hidden_by_focus,
                b.hidden_by_fullscreen,
                b.user_hidden,
                b.focus_miss_count,
            ),
            None => return,
        }
    };

    if chatgpt_fg {
        if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
            b.focus_miss_count = 0;
        }
        if hidden_by_focus {
            if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
                b.hidden_by_focus = false;
            }
            if !hidden_by_fs && !user_hidden {
                unsafe {
                    let _ = ShowWindow(bubble_hwnd, SW_SHOWNOACTIVATE);
                }
                render(bubble_hwnd);
            }
            log::info!(
                "focus-follow: show (ChatGPT in foreground: {})",
                describe_foreground(fg)
            );
        }
        return;
    }

    // Not on ChatGPT: hide only after 2 consecutive misses (~0.7s) so a
    // transient focus steal doesn't flicker the bubble.
    let misses = miss_count.saturating_add(1);
    if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
        b.focus_miss_count = misses;
    }
    if misses >= 2 && !hidden_by_focus && !user_hidden {
        unsafe {
            let _ = ShowWindow(bubble_hwnd, SW_HIDE);
        }
        if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
            b.hidden_by_focus = true;
        }
        log::info!(
            "focus-follow: hide (foreground is not ChatGPT: {})",
            describe_foreground(fg)
        );
    }
}

/// True when the foreground window belongs to the ChatGPT desktop app or to
/// our own UI (bubble/panel/context menu share our PID, so interacting with
/// them never hides the bubble). Fails open: an unidentifiable foreground
/// window keeps the bubble visible rather than hiding it.
fn foreground_is_chatgpt_or_own(bubble_hwnd: HWND, fg: HWND) -> bool {
    if fg == HWND::default() || fg == bubble_hwnd {
        return true;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return true;
    }
    if pid == unsafe { GetCurrentProcessId() } {
        return true;
    }
    match foreground_image_file_name(pid) {
        Some(name) => name.eq_ignore_ascii_case(CHATGPT_EXE),
        None => true,
    }
}

/// Short foreground description for focus-follow log lines, e.g.
/// `ChatGPT.exe`, `chrome.exe`, `own-ui` or `unknown(pid=1234)`.
fn describe_foreground(fg: HWND) -> String {
    if fg == HWND::default() {
        return String::from("none");
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return String::from("unknown(pid=0)");
    }
    if pid == unsafe { GetCurrentProcessId() } {
        return String::from("own-ui");
    }
    match foreground_image_file_name(pid) {
        Some(name) => name,
        None => format!("unidentified(pid={pid})"),
    }
}

fn foreground_image_file_name(pid: u32) -> Option<String> {    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut len: u32 = 512;
        let mut buf = vec![0u16; len as usize];
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit(['\\', '/']).next().map(|s| s.to_string())
    }
}

struct FullscreenEvaluation {
    is_fullscreen: bool,
    hwnd: HWND,
    class_name: String,
    bounds: Option<RECT>,
    bounds_source: &'static str,
    monitor: Option<RECT>,
    reason: &'static str,
}

fn evaluate_foreground_fullscreen(fg: HWND, bubble_hwnd: HWND) -> FullscreenEvaluation {
    let mut evaluation = FullscreenEvaluation {
        is_fullscreen: false,
        hwnd: fg,
        class_name: String::new(),
        bounds: None,
        bounds_source: "none",
        monitor: None,
        reason: "not evaluated",
    };

    if fg == HWND::default() {
        evaluation.reason = "no foreground window";
        return evaluation;
    }

    evaluation.class_name = window_class_name(fg);
    if fg == bubble_hwnd || is_ignored_fullscreen_class(&evaluation.class_name) {
        evaluation.reason = "ignored foreground class";
        return evaluation;
    }
    if unsafe { !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() } {
        evaluation.reason = "foreground invisible or minimized";
        return evaluation;
    }
    if is_dwm_cloaked(fg) {
        evaluation.reason = "foreground cloaked";
        return evaluation;
    }

    let Some((bounds, source)) = visible_window_bounds(fg) else {
        evaluation.reason = "foreground bounds unavailable";
        return evaluation;
    };
    evaluation.bounds = Some(bounds);
    evaluation.bounds_source = source;

    let monitor = unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        evaluation.reason = "foreground monitor unavailable";
        return evaluation;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { !GetMonitorInfoW(monitor, &mut info).as_bool() } {
        evaluation.reason = "foreground monitor info unavailable";
        return evaluation;
    }
    evaluation.monitor = Some(info.rcMonitor);

    if !rect_covers_monitor(&bounds, &info.rcMonitor) {
        evaluation.reason = "visible bounds do not cover monitor";
        return evaluation;
    }

    if !window_style_allows_fullscreen(fg) {
        evaluation.reason = "standard framed window";
        return evaluation;
    }

    evaluation.is_fullscreen = true;
    evaluation.reason = "visible bounds cover monitor";
    evaluation
}

fn show_after_fullscreen(bubble_hwnd: HWND, user_hidden: bool) {
    if !user_hidden {
        unsafe {
            let _ = ShowWindow(bubble_hwnd, SW_SHOWNOACTIVATE);
        }
        // Re-paint so the layered surface has the cached data again
        // (see comment in `set_user_visible`).
        render(bubble_hwnd);
    }
    if let Some(b) = lock_bubbles().get_mut(&(bubble_hwnd.0 as isize)) {
        b.hidden_by_fullscreen = false;
    }
}

fn visible_window_bounds(hwnd: HWND) -> Option<(RECT, &'static str)> {
    let mut rect = RECT::default();
    let dwm_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
    };
    if dwm_ok && !rect_is_empty(&rect) {
        return Some((rect, "dwm-extended-frame"));
    }

    unsafe {
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
    }
    if rect_is_empty(&rect) {
        None
    } else {
        Some((rect, "window-rect"))
    }
}

fn is_dwm_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn is_ignored_fullscreen_class(class_name: &str) -> bool {
    ["Progman", "WorkerW", "Shell_TrayWnd", CLASS_NAME]
        .iter()
        .any(|ignored| class_name.eq_ignore_ascii_case(ignored))
}

fn window_style_allows_fullscreen(hwnd: HWND) -> bool {
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    if style == 0 && unsafe { GetLastError() } != WIN32_ERROR(0) {
        return false;
    }
    window_style_bits_allow_fullscreen(style as u32)
}

fn window_style_bits_allow_fullscreen(style: u32) -> bool {
    let has_child = style & WS_CHILD.0 != 0;
    let has_popup = style & WS_POPUP.0 != 0;
    let has_caption = style & WS_CAPTION.0 != 0;
    let has_resize_frame = style & WS_THICKFRAME.0 != 0;

    !has_child && (has_popup || (!has_caption && !has_resize_frame))
}

fn rect_covers_monitor(rect: &RECT, bounds: &RECT) -> bool {
    rect.left <= bounds.left + FULLSCREEN_EDGE_TOLERANCE_PX
        && rect.top <= bounds.top + FULLSCREEN_EDGE_TOLERANCE_PX
        && rect.right >= bounds.right - FULLSCREEN_EDGE_TOLERANCE_PX
        && rect.bottom >= bounds.bottom - FULLSCREEN_EDGE_TOLERANCE_PX
}

fn rect_is_empty(rect: &RECT) -> bool {
    rect.right <= rect.left || rect.bottom <= rect.top
}

fn log_fullscreen_decision(action: &str, evaluation: &FullscreenEvaluation, user_hidden: bool) {
    log::info!(
        "bubble fullscreen {action} fg=0x{:X} class={} reason={} bounds_source={} bounds={} monitor={} user_hidden={}",
        evaluation.hwnd.0 as usize,
        if evaluation.class_name.is_empty() {
            "<unknown>"
        } else {
            evaluation.class_name.as_str()
        },
        evaluation.reason,
        evaluation.bounds_source,
        format_rect(evaluation.bounds.as_ref()),
        format_rect(evaluation.monitor.as_ref()),
        user_hidden
    );
}

fn format_rect(rect: Option<&RECT>) -> String {
    match rect {
        Some(r) => format!(
            "({},{} {}x{})",
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top
        ),
        None => String::from("<none>"),
    }
}

#[cfg(test)]
mod fullscreen_tests {
    use super::*;

    #[test]
    fn style_bits_allow_popup_or_borderless_windows_only() {
        assert!(window_style_bits_allow_fullscreen(WS_POPUP.0));
        assert!(window_style_bits_allow_fullscreen(0));
        assert!(!window_style_bits_allow_fullscreen(
            WS_CAPTION.0 | WS_THICKFRAME.0
        ));
        assert!(!window_style_bits_allow_fullscreen(WS_CHILD.0 | WS_POPUP.0));
    }

    #[test]
    fn rect_cover_check_allows_small_edge_differences() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let almost_exact = RECT {
            left: 1,
            top: 2,
            right: 1919,
            bottom: 1078,
        };
        let inset = RECT {
            left: 8,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        assert!(rect_covers_monitor(&almost_exact, &monitor));
        assert!(!rect_covers_monitor(&inset, &monitor));
    }
}

// ---------- Painting ----------

// Sized for the widest English countdown text the bubble renders.
const COUNTDOWN_TEMPLATE: &str = "999d";

/// Geometry for the fork's vertical minimalist card, in DPI-scaled pixels.
///
/// The outline is a vertical pill (`corner_radius = canvas_w / 2`). Top holds
/// the 5h progress ring with the "RESTA" label + big remaining-% glyph.
/// Below: primary caption line, weekly usage bar, weekly caption line.
struct BubbleLayout {
    canvas_w: i32,
    canvas_h: i32,
    corner_radius: i32,
    ring_cx: f32,
    ring_cy: f32,
    ring_radius: f32,
    ring_stroke_w: f32,
    time_ring_radius: f32,
    time_ring_stroke_w: f32,
    resta_label_rect: RECT,
    pct_rect: RECT,
    countdown_rect: RECT,
    big_font_px: i32,
    small_font_px: i32,
    main_font_px: i32,
}

fn compute_bubble_layout(size_logical: i32, dpi: u32, mem_dc: HDC) -> BubbleLayout {
    let width_px = scale_to_dpi(size_logical, dpi);
    let height_px = scale_to_dpi(bubble_height_logical(size_logical), dpi);
    let pad = (width_px * 6 / 100).max(1);
    let ring_d = width_px - 2 * pad;

    let ring_stroke_w = ((ring_d * 6 / 100).max(1)) as f32;
    let ring_cx = (width_px as f32) / 2.0;
    let ring_cy = (pad + ring_d / 2) as f32;
    // Ring centerline: midway between outer and inner edge, then keep stroke
    // inside the padding. ring_radius is the centerline radius.
    let ring_outer = (ring_d as f32) / 2.0 - ring_stroke_w / 2.0 - 1.0;
    let ring_radius = (ring_outer - ring_stroke_w / 2.0).max(1.0);
    // Inner ring renders the remaining-time arc. Floor stroke at 2 logical so
    // it stays visible at smaller bubble sizes (clamp 1 produced a hairline
    // that disappeared into the track on dark themes).
    let time_ring_stroke_w = (ring_stroke_w * 0.55).max(1.0);
    let time_gap = ((ring_d * 3 / 100) as f32).max(1.0);
    let time_ring_radius =
        (ring_radius - ring_stroke_w - time_gap).max(time_ring_stroke_w);

    let big_font_px = (ring_d * 24 / 100).max(4);
    let small_font_px = ((big_font_px * 40) / 100).max(3);
    let main_font_px = small_font_px;

    // Ring texts, vertically centered inside the ring.
    let label_h = small_font_px + scale_to_dpi(2, dpi);
    let pct_h = big_font_px + scale_to_dpi(2, dpi);
    let label_pct_gap = (big_font_px * 12 / 100).max(1);
    let ring_text_h = label_h + label_pct_gap + pct_h;
    let ring_text_top = pad + (ring_d - ring_text_h) / 2;
    let resta_label_rect = RECT {
        left: pad,
        top: ring_text_top,
        right: width_px - pad,
        bottom: ring_text_top + label_h,
    };
    let pct_rect = RECT {
        left: pad,
        top: ring_text_top + label_h + label_pct_gap,
        right: width_px - pad,
        bottom: ring_text_top + ring_text_h,
    };

    // Single countdown caption below the ring (nothing else).
    let cap_h = main_font_px + scale_to_dpi(5, dpi);
    let ring_gap = (ring_d * 8 / 100).max(2);
    let y = pad + ring_d + ring_gap;
    let countdown_rect = RECT {
        left: pad,
        top: y,
        right: width_px - pad,
        bottom: y + cap_h,
    };
    let _ = mem_dc;

    BubbleLayout {
        canvas_w: width_px,
        canvas_h: height_px,
        corner_radius: (width_px * 10 / 100).max(1),
        ring_cx,
        ring_cy,
        ring_radius,
        ring_stroke_w,
        time_ring_radius,
        time_ring_stroke_w,
        resta_label_rect,
        pct_rect,
        countdown_rect,
        big_font_px,
        small_font_px,
        main_font_px,
    }
}

/// Render the bubble's shape into a fresh tiny-skia `Pixmap`. The Pixmap is
/// premultiplied RGBA at one byte per channel — the caller copies it into the
/// GDI DIB section, then GDI text is overlaid on top.
fn paint_bubble_pixmap(layout: &BubbleLayout, inputs: &PaintInputs) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(layout.canvas_w as u32, layout.canvas_h as u32)?;
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let bg = if inputs.is_dark {
        Color::from_hex("#1F1F1F")
    } else {
        Color::from_hex("#F3F3F3")
    };
    let track = if inputs.is_dark {
        Color::from_hex("#2C2C2C")
    } else {
        Color::from_hex("#E2E2E2")
    };
    // Inner-ring / time-bar neutral track. Lifted off the background to
    // clear WCAG 1.4.11 3:1 on dark themes (#303030 on #1F1F1F was ~1.13:1).
    let time_track = if inputs.is_dark {
        Color::from_hex("#404040")
    } else {
        Color::from_hex("#E0E0E0")
    };
    let time_fill = if inputs.is_dark {
        Color::from_hex("#B0B0B0")
    } else {
        Color::from_hex("#666666")
    };

    // ---- Stadium background ----
    {
        let mut paint = Paint::default();
        paint.set_color(rgb_to_skia(bg));
        paint.anti_alias = true;
        let r = layout.corner_radius as f32;
        let w = layout.canvas_w as f32;
        let h = layout.canvas_h as f32;

        // Two end-cap circles + middle rect. Overlap is fine — same color.
        let mut pb = PathBuilder::new();
        pb.push_circle(r, r, r);
        pb.push_circle(r, h - r, r);
        if let Some(p) = pb.finish() {
            pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }
        if let Some(rect) = Rect::from_xywh(0.0, r, w, (h - 2.0 * r).max(0.0)) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    // ---- Ring (5h) ----
    {
        // Track: full circle in muted color.
        let mut paint = Paint::default();
        paint.set_color(rgb_to_skia(track));
        paint.anti_alias = true;
        let mut stroke = Stroke::default();
        stroke.width = layout.ring_stroke_w;
        let mut pb = PathBuilder::new();
        pb.push_circle(layout.ring_cx, layout.ring_cy, layout.ring_radius);
        if let Some(p) = pb.finish() {
            pixmap.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }

        // Active sweep arc. Fork: fuel-gauge — the ring shows what REMAINS
        // (100 - used). Colors/thresholds below still use `pct` (used).
        if let Some(pct) = inputs.session_pct {
            let sweep = ((100.0 - pct).clamp(0.0, 100.0) / 100.0) as f32;
            if sweep > 0.0 {
                let mut color =
                    crate::usage_color::bar_fill_color(inputs.model, inputs.is_dark, pct);
                if pct >= 95.0 {
                    let t = pulse_triangle(inputs.pulse_phase);
                    color = brighten(color, t);
                }
                let mut paint = Paint::default();
                paint.set_color(rgb_to_skia(color));
                paint.anti_alias = true;
                let mut stroke = Stroke::default();
                stroke.width = layout.ring_stroke_w;
                stroke.line_cap = LineCap::Round;
                if let Some(path) =
                    build_arc(layout.ring_cx, layout.ring_cy, layout.ring_radius, sweep)
                {
                    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
        }

        // Inner ring: true remaining time for the 5h/primary window. This
        // stays neutral so it reads as time, not another quota alarm.
        let mut paint = Paint::default();
        paint.set_color(rgb_to_skia(time_track));
        paint.anti_alias = true;
        let mut stroke = Stroke::default();
        stroke.width = layout.time_ring_stroke_w;
        let mut pb = PathBuilder::new();
        pb.push_circle(layout.ring_cx, layout.ring_cy, layout.time_ring_radius);
        if let Some(p) = pb.finish() {
            pixmap.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }
        if let Some(frac) = remaining_fraction(
            inputs.session_resets_at,
            window_duration_secs(inputs.model, UsageWindowKind::Primary),
        ) {
            if frac > 0.0 {
                let mut paint = Paint::default();
                paint.set_color(rgb_to_skia(time_fill));
                paint.anti_alias = true;
                let mut stroke = Stroke::default();
                stroke.width = layout.time_ring_stroke_w;
                stroke.line_cap = LineCap::Round;
                if let Some(path) = build_remaining_arc(
                    layout.ring_cx,
                    layout.ring_cy,
                    layout.time_ring_radius,
                    frac,
                ) {
                    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
        }
    }


    Some(pixmap)
}

/// Fill a horizontal pill at `(x, y, w, h)` with circular end caps of radius
/// `cap`. Used for both track and fill segments in the tail bars.
fn paint_pill(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, cap: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(rgb_to_skia(color));
    paint.anti_alias = true;
    let mut pb = PathBuilder::new();
    pb.push_circle(x + cap, y + h * 0.5, cap);
    pb.push_circle(x + w - cap, y + h * 0.5, cap);
    if let Some(p) = pb.finish() {
        pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(rect) = Rect::from_xywh(x + cap, y, (w - 2.0 * cap).max(0.0), h) {
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

fn rgb_to_skia(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, 0xFF)
}

/// Build a clockwise arc path starting at 12 o'clock, sweeping `sweep_fraction`
/// of a full turn. Sampled — tiny-skia 0.11 lacks a direct arc primitive.
fn build_arc(cx: f32, cy: f32, radius: f32, sweep_fraction: f32) -> Option<tiny_skia::Path> {
    let segments = ((sweep_fraction * 64.0).ceil() as usize).max(1);
    let mut pb = PathBuilder::new();
    let start_angle: f32 = -std::f32::consts::FRAC_PI_2;
    let total = sweep_fraction * std::f32::consts::TAU;
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let a = start_angle + t * total;
        let x = cx + a.cos() * radius;
        let y = cy + a.sin() * radius;
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

/// Clockwise arc that ENDS at 12 o'clock; the consumed wedge grows clockwise
/// from 12 as `remaining_fraction` shrinks, mirroring a clock-hand countdown.
fn build_remaining_arc(
    cx: f32,
    cy: f32,
    radius: f32,
    remaining_fraction: f32,
) -> Option<tiny_skia::Path> {
    let frac = remaining_fraction.clamp(0.0, 1.0);
    let segments = ((frac * 64.0).ceil() as usize).max(1);
    let mut pb = PathBuilder::new();
    let twelve: f32 = -std::f32::consts::FRAC_PI_2;
    let total = frac * std::f32::consts::TAU;
    let start_angle = twelve + (std::f32::consts::TAU - total);
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let a = start_angle + t * total;
        let x = cx + a.cos() * radius;
        let y = cy + a.sin() * radius;
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

/// Copy a premultiplied-RGBA `Pixmap` into the 32bpp BI_RGB DIB the bubble
/// uses for `UpdateLayeredWindow`. The DIB stores BGRA bytes (little-endian
/// `0xAARRGGBB` when read as u32); tiny-skia's premultiplied alpha is exactly
/// the format `AC_SRC_ALPHA` expects.
fn copy_pixmap_to_dib(pixmap: &Pixmap, dst: &mut [u32]) {
    let src = pixmap.data();
    let pixel_count = (pixmap.width() * pixmap.height()) as usize;
    for i in 0..pixel_count {
        let r = src[i * 4];
        let g = src[i * 4 + 1];
        let b = src[i * 4 + 2];
        let a = src[i * 4 + 3];
        dst[i] = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    }
}

/// Re-stamp the alpha byte of every DIB pixel from the source `Pixmap`. Used
/// after GDI text rendering, which writes RGB but leaves the BI_RGB DIB's
/// "reserved" alpha byte at zero — making glyph pixels appear transparent
/// when `UpdateLayeredWindow` composites with `AC_SRC_ALPHA`.
fn restore_alpha_from_pixmap(pixmap: &Pixmap, dst: &mut [u32]) {
    let src = pixmap.data();
    let pixel_count = (pixmap.width() * pixmap.height()) as usize;
    for i in 0..pixel_count {
        let a = src[i * 4 + 3] as u32;
        dst[i] = (dst[i] & 0x00FF_FFFF) | (a << 24);
    }
}

fn measure_text_w(hdc: HDC, text: &str, font_height_px: i32) -> i32 {
    use windows::Win32::Foundation::SIZE;
    let font_name = wide_str("Segoe UI");
    let mut w: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let font = CreateFontW(
            -font_height_px,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            (FF_SWISS.0 | DEFAULT_PITCH.0) as u32,
            PCWSTR::from_raw(font_name.as_ptr()),
        );
        let old = SelectObject(hdc, font);
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &mut w, &mut size);
        SelectObject(hdc, old);
        let _ = DeleteObject(font);
        size.cx
    }
}

struct PaintInputs {
    model: ProviderId,
    session_pct: Option<f64>,
    session_text: String,
    session_resets_at: Option<SystemTime>,
    weekly_pct: Option<f64>,
    weekly_text: String,
    weekly_resets_at: Option<SystemTime>,
    is_dark: bool,
    pulse_phase: u32,
}

fn render(hwnd: HWND) {
    let (size_logical, dpi, inputs) = {
        let bubbles = lock_bubbles();
        let Some(b) = bubbles.get(&(hwnd.0 as isize)) else {
            return;
        };
        (
            b.size_logical,
            b.dpi,
            PaintInputs {
                model: b.model,
                session_pct: b.session_pct,
                session_text: b.session_text.clone(),
                session_resets_at: b.session_resets_at,
                weekly_pct: b.weekly_pct,
                weekly_text: b.weekly_text.clone(),
                weekly_resets_at: b.weekly_resets_at,
                is_dark: b.is_dark,
                pulse_phase: b.pulse_phase,
            },
        )
    };

    unsafe {
        let screen_dc = GetDC(hwnd);
        if screen_dc.is_invalid() {
            return;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            ReleaseDC(hwnd, screen_dc);
            return;
        }
        let layout = compute_bubble_layout(size_logical, dpi, mem_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: layout.canvas_w,
                biHeight: -layout.canvas_h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib =
            CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        if dib.is_invalid() || bits.is_null() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(hwnd, screen_dc);
            return;
        }
        let old_bmp = SelectObject(mem_dc, dib);

        let pixel_count = (layout.canvas_w * layout.canvas_h) as usize;
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, pixel_count);

        // Paint shape via tiny-skia (AA), then copy into the DIB. GDI text
        // overlays on top of the resulting bitmap.
        let pixmap_opt = paint_bubble_pixmap(&layout, &inputs);
        if let Some(ref pixmap) = pixmap_opt {
            copy_pixmap_to_dib(pixmap, pixels);
        } else {
            pixels.fill(0);
        }
        paint_bubble_text(mem_dc, &layout, &inputs);
        // GDI text writes RGB into the 32bpp BI_RGB DIB but does not preserve
        // the alpha byte (per the BITMAPINFOHEADER contract: byte 3 is
        // "reserved/0" for BI_RGB). UpdateLayeredWindow with AC_SRC_ALPHA then
        // reads those zeroed alpha bytes and paints the glyph pixels as fully
        // transparent — desktop bleeds through. Fix: re-stamp the alpha
        // channel from the tiny-skia Pixmap we still have in scope. This
        // preserves the AA alpha on the stadium's curved perimeter and forces
        // glyph pixels back to the opacity tiny-skia computed for that
        // location (255 in the interior, 0 outside).
        if let Some(ref pixmap) = pixmap_opt {
            restore_alpha_from_pixmap(pixmap, pixels);
        }

        let mut wr = RECT::default();
        let _ = GetWindowRect(hwnd, &mut wr);
        let pt_dst = POINT {
            x: wr.left,
            y: wr.top,
        };
        let pt_src = POINT { x: 0, y: 0 };
        let sz = SIZE {
            cx: layout.canvas_w,
            cy: layout.canvas_h,
        };
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: 1, // AC_SRC_ALPHA
        };
        let _ = UpdateLayeredWindow(
            hwnd,
            screen_dc,
            Some(&pt_dst),
            Some(&sz),
            mem_dc,
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(hwnd, screen_dc);
    }
}

/// Triangle wave in [0, 1] with period 24 ticks. 0 at phase=0,12; 1 at phase=6,18.
fn pulse_triangle(phase: u32) -> f64 {
    let p = (phase % 24) as i32;
    let dist = (p - 12).abs(); // 0..12
    1.0 - (dist as f64 / 12.0)
}

/// Linearly brighten `c` toward white by `t` in [0, 1].
fn brighten(c: Color, t: f64) -> Color {
    // Map t to a smaller brightness delta — the pulse should be a subtle nudge.
    let t = t.clamp(0.0, 1.0) * 0.30;
    Color::new(
        ((c.r as f64) + (255.0 - c.r as f64) * t).round() as u8,
        ((c.g as f64) + (255.0 - c.g as f64) * t).round() as u8,
        ((c.b as f64) + (255.0 - c.b as f64) * t).round() as u8,
    )
}

/// Fork (Rafael): vertical minimalist texts — "RESTA" + big remaining-% in
/// the ring, centered primary caption below it, weekly caption at the bottom.
fn paint_bubble_text(hdc: HDC, layout: &BubbleLayout, inputs: &PaintInputs) {
    let text_color = if inputs.is_dark {
        Color::from_hex("#EAEAEA")
    } else {
        Color::from_hex("#1F1F1F")
    };

    let font_name = wide_str("Segoe UI");
    unsafe {
        // Big remaining-% uses FW_BOLD to anchor the eye against the ring.
        let big_font = create_font(layout.big_font_px, &font_name, FW_BOLD.0 as i32);
        let small_font = create_font(layout.small_font_px, &font_name, FW_SEMIBOLD.0 as i32);
        SetBkMode(hdc, TRANSPARENT);

        let prev_font = SelectObject(hdc, small_font);

        // "RESTA": full-contrast so it stays readable at tiny sizes.
        SetTextColor(hdc, COLORREF(text_color.into_colorref()));
        draw_text_in_rect(hdc, &layout.resta_label_rect, "RESTA", DT_CENTER);

        // Big remaining-% glyph centered in the ring.
        SelectObject(hdc, big_font);
        SetTextColor(hdc, COLORREF(text_color.into_colorref()));
        let pct_text = match inputs.session_pct {
            Some(p) => format!("{:.0}%", (100.0 - p).clamp(0.0, 100.0)),
            None => String::from("—"),
        };
        draw_text_in_rect(hdc, &layout.pct_rect, &pct_text, DT_CENTER);

        // Single countdown caption: precise time left, BOLD, pure
        // black-on-light / white-on-dark. Base size +3px over the caption
        // font, then shrink-to-fit so long texts ("6 dias e 19 horas")
        // never clip at small bubble sizes.
        let count_color = if inputs.is_dark {
            Color::from_hex("#FFFFFF")
        } else {
            Color::from_hex("#000000")
        };
        let avail_w = (layout.countdown_rect.right - layout.countdown_rect.left).max(0);
        // NOTE: no `dpi` in scope here; +3px literal matches the logical
        // formula at 96dpi and shrink-to-fit guarantees no clipping anywhere.
        // Fit target leaves a 2px breathing room: measure_text_w uses a
        // NORMAL font while we draw BOLD (slightly wider).
        let mut count_px = layout.main_font_px + 3;
        while count_px > 4
            && measure_text_w(hdc, &inputs.session_text, count_px) > avail_w.saturating_sub(2)
        {
            count_px -= 1;
        }
        let count_font = create_font(count_px, &font_name, FW_BOLD.0 as i32);
        SelectObject(hdc, count_font);
        SetTextColor(hdc, COLORREF(count_color.into_colorref()));
        draw_text_in_rect(hdc, &layout.countdown_rect, &inputs.session_text, DT_CENTER);

        SelectObject(hdc, prev_font);
        let _ = DeleteObject(big_font);
        let _ = DeleteObject(small_font);
        let _ = DeleteObject(count_font);
    }
}


/// Draw `text` into `rect` with the given horizontal alignment flag, vertically
/// centered. The DT_NOCLIP flag preserves ascenders/descenders that would
/// otherwise be clipped by tight rects.
fn draw_text_in_rect(hdc: HDC, rect: &RECT, text: &str, halign: DRAW_TEXT_FORMAT) {
    let mut buf = wide_str(text);
    let len_no_nul = buf.len().saturating_sub(1);
    let mut r = *rect;
    unsafe {
        let _ = DrawTextW(
            hdc,
            &mut buf[..len_no_nul],
            &mut r,
            halign | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
        );
    }
}

fn draw_tail_text_in_rect(hdc: HDC, rect: &RECT, text: &str, halign: DRAW_TEXT_FORMAT) {
    if rect.right <= rect.left {
        return;
    }
    let mut buf = wide_str(text);
    let len_no_nul = buf.len().saturating_sub(1);
    let mut r = *rect;
    unsafe {
        let _ = DrawTextW(
            hdc,
            &mut buf[..len_no_nul],
            &mut r,
            halign | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
}

fn create_font(height_px: i32, name_w: &[u16], weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height_px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            (FF_SWISS.0 | DEFAULT_PITCH.0) as u32,
            PCWSTR::from_raw(name_w.as_ptr()),
        )
    }
}

// ---------- Helpers ----------

fn default_position(width_px: i32, height_px: i32, model: ProviderId) -> (i32, i32) {
    // Bottom-right of primary work area, with a 24-pixel gap from the edges.
    // Stagger the Codex bubble above the Claude one if both are enabled.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let wa = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            }
        };
        let gap = 24;
        let stagger = match model {
            ProviderId::Claude => 0,
            ProviderId::ChatGpt => height_px + gap,
            ProviderId::OpenCodeGo => 2 * (height_px + gap),
        };
        let x = wa.right - width_px - gap;
        let y = wa.bottom - height_px - gap - stagger;
        (x, y)
    }
}
