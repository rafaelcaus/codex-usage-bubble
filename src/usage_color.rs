// Shared usage→color ramp used by the floating bubble, the expanded panel,
// and the tray badge. Keeping the function in one place ensures the three
// surfaces never disagree about what "78% used" looks like.

use crate::os::Rgb as Color;
use crate::usage::ProviderId;

/// Per-provider identity color. Claude = warm orange `#D97757`. Codex tracks
/// the OpenAI Codex monochrome palette — near-white `#E5E5E5` on dark themes,
/// near-black `#1A1A1A` on light — so the accent stays readable against both
/// the dark bubble surface and the `#F3F3F3` light background.
pub fn accent_color_for(model: ProviderId, is_dark: bool) -> Color {
    match model {
        ProviderId::Claude => Color::from_hex("#D97757"),
        ProviderId::ChatGpt => {
            if is_dark {
                Color::from_hex("#E5E5E5")
            } else {
                Color::from_hex("#1A1A1A")
            }
        }
        ProviderId::OpenCodeGo => Color::from_hex("#3BAE75"),
    }
}

/// Discrete 4-band fill color. The "safe" band uses the provider's identity
/// color so Codex bars render monochrome (light-on-dark / dark-on-light) while
/// Claude bars stay orange; the warning bands are theme-aware so light-mode
/// amber stays readable against the `#F3F3F3` background.
///
/// - <60%   → provider accent
/// - 60–80% → amber (dark `#E0A040`, light `#B47A20` for WCAG AA contrast)
/// - 80–95% → red `#C45020`
/// - ≥95%   → deep red `#A01818` — paired with pulse animation
pub fn bar_fill_color(model: ProviderId, is_dark: bool, percent: f64) -> Color {
    if percent < 60.0 {
        accent_color_for(model, is_dark)
    } else if percent < 80.0 {
        if is_dark {
            Color::from_hex("#E0A040")
        } else {
            Color::from_hex("#B47A20")
        }
    } else if percent < 95.0 {
        Color::from_hex("#C45020")
    } else {
        Color::from_hex("#A01818")
    }
}
