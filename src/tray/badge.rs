// Anti-aliased tray badge rendering.
//
// We draw a filled circle + percentage-sweep ring with tiny-skia, then
// hand the resulting BGRA pixmap to Win32 as an HICON via
// `CreateIconIndirect`. The badge is intentionally text-free — the
// floating bubble already shows the exact percentage; the tray badge
// is just a coarse colour-and-fill indicator.

use std::ffi::c_void;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

use crate::usage::ProviderId;

const BADGE_PX: u32 = 32;

pub fn render_hicon(kind: ProviderId, percent: Option<f64>) -> HICON {
    let pixmap = render_pixmap(kind, percent);
    pixmap_to_hicon(&pixmap).unwrap_or_default()
}

fn render_pixmap(kind: ProviderId, percent: Option<f64>) -> Pixmap {
    let mut pixmap = Pixmap::new(BADGE_PX, BADGE_PX).expect("32×32 pixmap");
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let cx = BADGE_PX as f32 / 2.0;
    let cy = BADGE_PX as f32 / 2.0;
    let outer = (BADGE_PX as f32 / 2.0) - 1.0;
    let inner = outer * 0.62;

    // Filled inner disk in the model's base tint.
    let base = base_color(kind);
    {
        let mut paint = Paint::default();
        paint.set_color_rgba8(base[0], base[1], base[2], 255);
        paint.anti_alias = true;
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, inner);
        if let Some(path) = pb.finish() {
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    // Track ring (the unused portion of the quota).
    {
        let mut paint = Paint::default();
        paint.set_color_rgba8(0x3a, 0x3a, 0x3a, 255);
        paint.anti_alias = true;
        let mut stroke = Stroke::default();
        stroke.width = outer - inner - 1.0;
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, (inner + outer) / 2.0);
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    // Active sweep — percentage of ring filled in usage colour.
    if let Some(p) = percent {
        let sweep = (p.clamp(0.0, 100.0) / 100.0) as f32;
        if sweep > 0.0 {
            let fill = usage_color(kind, p);
            let mut paint = Paint::default();
            paint.set_color_rgba8(fill[0], fill[1], fill[2], 255);
            paint.anti_alias = true;
            let mut stroke = Stroke::default();
            stroke.width = outer - inner - 1.0;
            stroke.line_cap = tiny_skia::LineCap::Round;

            let pb_path = build_arc(cx, cy, (inner + outer) / 2.0, sweep);
            if let Some(path) = pb_path {
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }

    pixmap
}

fn build_arc(cx: f32, cy: f32, radius: f32, sweep_fraction: f32) -> Option<tiny_skia::Path> {
    // tiny-skia 0.11 lacks a direct arc primitive. Approximate by sampling
    // points along the circumference from the 12 o'clock position clockwise.
    let segments = (sweep_fraction * 64.0).ceil() as usize;
    let segments = segments.max(1);
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

fn base_color(kind: ProviderId) -> [u8; 3] {
    match kind {
        // Warm orange-ish tint reads as "Claude" without copying the
        // upstream's exact #D97757; close enough to be familiar.
        ProviderId::Claude => [0x2a, 0x1f, 0x1c],
        // Cool dark slate for ChatGPT/Codex.
        ProviderId::ChatGpt => [0x1a, 0x1f, 0x26],
        // Deep green for OpenCode Go.
        ProviderId::OpenCodeGo => [0x12, 0x2a, 0x20],
    }
}

/// Sweep-ring fill color for the tray badge. The badge inner disk is always
/// dark regardless of system theme, so we pass `is_dark = true` to keep the
/// ring readable (Codex sweep stays white instead of charcoal).
fn usage_color(kind: ProviderId, percent: f64) -> [u8; 3] {
    let c = crate::usage_color::bar_fill_color(kind, true, percent);
    [c.r, c.g, c.b]
}

// ---------- Pixmap → HICON ----------

fn pixmap_to_hicon(pixmap: &Pixmap) -> Option<HICON> {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let pixels = pixmap.data(); // tiny-skia premultiplied RGBA bytes

    // Build a 32bpp top-down DIB section for the colour bitmap.
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let hdc = GetDC(HWND::default());
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color_bmp = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .ok()
            .unwrap_or_default();
        ReleaseDC(HWND::default(), hdc);
        if color_bmp.is_invalid() || bits.is_null() {
            return None;
        }
        // tiny-skia produces RGBA (premultiplied); GDI's DIB is BGRA. Swap.
        let pixel_count = (width * height) as usize;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u32, pixel_count);
        for i in 0..pixel_count {
            let r = pixels[i * 4];
            let g = pixels[i * 4 + 1];
            let b = pixels[i * 4 + 2];
            let a = pixels[i * 4 + 3];
            dst[i] = (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | (b as u32);
        }

        // Monochrome AND mask — opaque pixels marked 0, transparent 1.
        let mask_row_stride = ((width + 15) / 16) * 2; // 16-bit aligned per scanline
        let mut mask = vec![0u8; (mask_row_stride * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                let alpha = pixels[idx * 4 + 3];
                if alpha == 0 {
                    let byte = (y * mask_row_stride + (x / 8)) as usize;
                    mask[byte] |= 0x80 >> (x % 8);
                }
            }
        }
        let mask_bmp: HBITMAP = CreateBitmap(width, height, 1, 1, Some(mask.as_ptr() as *const _));
        if mask_bmp.is_invalid() {
            let _ = DeleteObject(color_bmp);
            return None;
        }

        let info = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let hicon = CreateIconIndirect(&info).ok();

        let _ = DeleteObject(color_bmp);
        let _ = DeleteObject(mask_bmp);
        hicon
    }
}
