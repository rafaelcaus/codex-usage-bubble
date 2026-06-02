// Expanded panel shown when a bubble is left-clicked.
//
// Plain opaque top-most popup with two horizontal usage bars (session/5h and
// weekly/7d) plus countdown text. Closes on focus loss.

use std::sync::{Mutex, MutexGuard, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::i18n::LocaleStrings;
use crate::os::dpi::scale as scale_to_dpi;
use crate::os::{to_utf16_nul as wide_str, Rgb as Color};
use crate::usage::ProviderId;

const CLASS_NAME: &str = "ClaudeCodeUsageBubblePanel";
const PANEL_W_LOGICAL: i32 = 280;
const PANEL_H_LOGICAL: i32 = 120;
const PADDING_LOGICAL: i32 = 14;
const ROW_GAP_LOGICAL: i32 = 8;
const LABEL_W_LOGICAL: i32 = 28;
const RIGHT_TEXT_W_LOGICAL: i32 = 96;
const BAR_HEIGHT_LOGICAL: i32 = 14;

pub struct PanelData {
    pub model: ProviderId,
    pub session_pct: f64,
    pub session_text: String,
    pub weekly_pct: f64,
    pub weekly_text: String,
    pub is_dark: bool,
    pub strings: LocaleStrings,
}

struct PanelState {
    hwnd: HWND,
    data: PanelData,
}

// HWND wraps `*mut c_void` which is `!Send`, but the panel state is only ever
// accessed from the UI thread — the Mutex exists to satisfy the `OnceLock`
// static contract, not because of real cross-thread sharing.
unsafe impl Send for PanelState {}

fn state() -> &'static Mutex<Option<PanelState>> {
    static S: OnceLock<Mutex<Option<PanelState>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn lock_state() -> MutexGuard<'static, Option<PanelState>> {
    state().lock().expect("panel state mutex poisoned")
}

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
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_w.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            log::error!("panel RegisterClassExW returned 0");
        }
    });
}

pub fn is_visible() -> bool {
    let s = lock_state();
    s.as_ref()
        .map(|p| unsafe { IsWindowVisible(p.hwnd).as_bool() })
        .unwrap_or(false)
}

pub fn current_model() -> Option<ProviderId> {
    lock_state().as_ref().map(|p| p.data.model)
}

/// Show the panel for the given model, anchored near the supplied bubble HWND.
/// If a panel is already visible, hide it instead (toggle).
pub fn toggle(data: PanelData, anchor_hwnd: HWND) {
    if is_visible() && current_model() == Some(data.model) {
        hide();
        return;
    }
    show(data, anchor_hwnd);
}

pub fn show(data: PanelData, anchor_hwnd: HWND) {
    register_class();

    let mut anchor_rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(anchor_hwnd, &mut anchor_rect);
    }
    let dpi = unsafe { GetDpiForWindow(anchor_hwnd).max(96) };
    let panel_w = scale_to_dpi(PANEL_W_LOGICAL, dpi);
    let panel_h = scale_to_dpi(PANEL_H_LOGICAL, dpi);
    let (x, y) = place_near(anchor_rect, panel_w, panel_h);

    let existing_hwnd = lock_state().as_ref().map(|p| p.hwnd);
    let hwnd = match existing_hwnd {
        Some(h) => h,
        None => match create_panel_window(x, y, panel_w, panel_h) {
            Some(h) => h,
            None => return,
        },
    };

    apply_win11_window_chrome(hwnd);

    {
        let mut guard = lock_state();
        if let Some(p) = guard.as_mut() {
            p.data = data;
        } else {
            *guard = Some(PanelState { hwnd, data });
        }
    }

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            x,
            y,
            panel_w,
            panel_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = InvalidateRect(hwnd, None, true);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn create_panel_window(x: i32, y: i32, w: i32, h: i32) -> Option<HWND> {
    let hwnd = unsafe {
        let class_w = wide_str(CLASS_NAME);
        let title_w = wide_str("Usage panel");
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            PCWSTR::from_raw(class_w.as_ptr()),
            PCWSTR::from_raw(title_w.as_ptr()),
            WS_POPUP,
            x,
            y,
            w,
            h,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .unwrap_or_default()
    };
    if hwnd == HWND::default() {
        log::error!("panel CreateWindowExW failed");
        None
    } else {
        Some(hwnd)
    }
}

/// Apply Windows 11 rounded corners. Win11-only — `DwmSetWindowAttribute`
/// returns an error on Win10 and earlier, which we deliberately swallow so
/// the panel falls back to square corners without complaint. Idempotent:
/// DWM ignores redundant identical-value sets, so calling this on every
/// `show()` is safe.
fn apply_win11_window_chrome(hwnd: HWND) {
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

pub fn hide() {
    let hwnd_opt = lock_state().as_ref().map(|p| p.hwnd);
    if let Some(hwnd) = hwnd_opt {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub fn destroy() {
    let hwnd_opt = lock_state().as_ref().map(|p| p.hwnd);
    if let Some(hwnd) = hwnd_opt {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    *lock_state() = None;
}

/// Refresh the panel's data (called from app when poll cycle completes).
pub fn refresh_data(data: PanelData) {
    let hwnd_opt = {
        let mut guard = lock_state();
        let Some(p) = guard.as_mut() else {
            return;
        };
        if p.data.model != data.model {
            // Showing a different model right now — caller can decide whether to swap.
            return;
        }
        p.data = data;
        Some(p.hwnd)
    };
    if let Some(hwnd) = hwnd_opt {
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
    }
}

// ---------- Window proc & painting ----------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_KILLFOCUS => {
            // Close the panel when it loses focus. Use ShowWindow rather than
            // DestroyWindow so we can re-show it next click without re-creating.
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            *lock_state() = None;
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn paint(hwnd: HWND, hdc: HDC) {
    let Some(data) = clone_data() else {
        return;
    };
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
    let scaled = |v: i32| scale_to_dpi(v, dpi);

    let bg = if data.is_dark {
        Color::from_hex("#1F1F1F")
    } else {
        Color::from_hex("#FAFAFA")
    };
    let text_color = if data.is_dark {
        Color::from_hex("#EAEAEA")
    } else {
        Color::from_hex("#1F1F1F")
    };
    let track = if data.is_dark {
        Color::from_hex("#3A3A3A")
    } else {
        Color::from_hex("#D6D6D6")
    };
    let session_accent =
        crate::usage_color::bar_fill_color(data.model, data.is_dark, data.session_pct);
    let weekly_accent =
        crate::usage_color::bar_fill_color(data.model, data.is_dark, data.weekly_pct);

    unsafe {
        let bg_brush = CreateSolidBrush(COLORREF(bg.into_colorref()));
        FillRect(hdc, &rc, bg_brush);
        let _ = DeleteObject(bg_brush);

        let stripe_color = crate::usage_color::accent_color_for(data.model, data.is_dark);
        let stripe_w = scaled(4);
        let stripe_rect = RECT {
            left: 0,
            top: 0,
            right: stripe_w,
            bottom: rc.bottom,
        };
        let stripe_brush = CreateSolidBrush(COLORREF(stripe_color.into_colorref()));
        FillRect(hdc, &stripe_rect, stripe_brush);
        let _ = DeleteObject(stripe_brush);

        // Header row: model label
        let header = match data.model {
            ProviderId::Claude => data.strings.claude_label.clone(),
            ProviderId::ChatGpt => data.strings.chatgpt_label.clone(),
            ProviderId::OpenCodeGo => data.strings.opencode_go_label.clone(),
        };
        draw_text(
            hdc,
            &header,
            text_color,
            scaled(PADDING_LOGICAL),
            scaled(PADDING_LOGICAL),
            rc.right - 2 * scaled(PADDING_LOGICAL),
            scaled(18),
            true,
            dpi,
        );

        let bar_x = scaled(PADDING_LOGICAL) + scaled(LABEL_W_LOGICAL) + scaled(4);
        let bar_w =
            rc.right - bar_x - scaled(PADDING_LOGICAL) - scaled(RIGHT_TEXT_W_LOGICAL) - scaled(4);
        let row1_y = scaled(PADDING_LOGICAL) + scaled(24);
        let row2_y = row1_y + scaled(BAR_HEIGHT_LOGICAL) + scaled(ROW_GAP_LOGICAL) + scaled(8);

        draw_row(
            hdc,
            &data.strings.session_window,
            scaled(PADDING_LOGICAL),
            row1_y,
            bar_x,
            bar_w,
            scaled(BAR_HEIGHT_LOGICAL),
            data.session_pct,
            &data.session_text,
            text_color,
            track,
            session_accent,
            dpi,
        );

        draw_row(
            hdc,
            &data.strings.weekly_window,
            scaled(PADDING_LOGICAL),
            row2_y,
            bar_x,
            bar_w,
            scaled(BAR_HEIGHT_LOGICAL),
            data.weekly_pct,
            &data.weekly_text,
            text_color,
            track,
            weekly_accent,
            dpi,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    hdc: HDC,
    label: &str,
    label_x: i32,
    row_y: i32,
    bar_x: i32,
    bar_w: i32,
    bar_h: i32,
    pct: f64,
    right_text: &str,
    text_color: Color,
    track: Color,
    accent: Color,
    dpi: u32,
) {
    let scaled = |v: i32| scale_to_dpi(v, dpi);
    let label_w = scaled(LABEL_W_LOGICAL);
    draw_text(
        hdc,
        label,
        text_color,
        label_x,
        row_y - scaled(2),
        label_w,
        bar_h + scaled(4),
        false,
        dpi,
    );

    unsafe {
        let track_brush = CreateSolidBrush(COLORREF(track.into_colorref()));
        let bar_rect = RECT {
            left: bar_x,
            top: row_y,
            right: bar_x + bar_w,
            bottom: row_y + bar_h,
        };
        FillRect(hdc, &bar_rect, track_brush);
        let _ = DeleteObject(track_brush);

        let fill_w = ((pct.clamp(0.0, 100.0) / 100.0) * bar_w as f64).round() as i32;
        if fill_w > 0 {
            let accent_brush = CreateSolidBrush(COLORREF(accent.into_colorref()));
            let fill_rect = RECT {
                left: bar_x,
                top: row_y,
                right: bar_x + fill_w,
                bottom: row_y + bar_h,
            };
            FillRect(hdc, &fill_rect, accent_brush);
            let _ = DeleteObject(accent_brush);
        }
    }

    let right_text_x = bar_x + bar_w + scaled(8);
    let right_text_w = scaled(RIGHT_TEXT_W_LOGICAL);
    draw_text(
        hdc,
        right_text,
        text_color,
        right_text_x,
        row_y - scaled(2),
        right_text_w,
        bar_h + scaled(4),
        false,
        dpi,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    hdc: HDC,
    text: &str,
    color: Color,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bold: bool,
    dpi: u32,
) {
    let mut text_w: Vec<u16> = text.encode_utf16().collect();
    let font_size = if bold {
        scale_to_dpi(13, dpi)
    } else {
        scale_to_dpi(11, dpi)
    };
    let font_name = wide_str("Segoe UI");
    unsafe {
        let weight = if bold { FW_SEMIBOLD.0 } else { FW_NORMAL.0 } as i32;
        let font = CreateFontW(
            -font_size,
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
            PCWSTR::from_raw(font_name.as_ptr()),
        );
        let old_font = SelectObject(hdc, font);
        SetTextColor(hdc, COLORREF(color.into_colorref()));
        SetBkMode(hdc, TRANSPARENT);
        let mut rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        let _ = DrawTextW(
            hdc,
            &mut text_w,
            &mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
    }
}

fn clone_data() -> Option<PanelData> {
    let guard = lock_state();
    let p = guard.as_ref()?;
    Some(PanelData {
        model: p.data.model,
        session_pct: p.data.session_pct,
        session_text: p.data.session_text.clone(),
        weekly_pct: p.data.weekly_pct,
        weekly_text: p.data.weekly_text.clone(),
        is_dark: p.data.is_dark,
        strings: p.data.strings.clone(),
    })
}

fn place_near(anchor: RECT, panel_w: i32, panel_h: i32) -> (i32, i32) {
    // Anchor below the bubble by default; flip above if it would clip.
    let mut x = anchor.left;
    let mut y = anchor.bottom + 8;
    // The virtual screen spans all monitors. Its origin is offset from
    // the primary monitor when a secondary monitor sits left of / above
    // the primary, so clamps must include SM_XVIRTUALSCREEN /
    // SM_YVIRTUALSCREEN — not just the width/height of the union.
    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let right = vx + vw;
    let bottom = vy + vh;
    if y + panel_h > bottom {
        y = anchor.top - panel_h - 8;
    }
    if y < vy {
        y = anchor.top.max(vy);
    }
    if x + panel_w > right {
        x = right - panel_w - 8;
    }
    if x < vx {
        x = vx + 8;
    }
    (x, y)
}
