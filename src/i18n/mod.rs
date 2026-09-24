// English UI strings and formatting helpers.

use std::time::{Duration, SystemTime};

/// The strings every UI module needs.
#[derive(Clone, Debug)]
pub struct LocaleStrings {
    pub window_title: String,
    pub refresh: String,
    pub update_frequency: String,
    pub one_minute: String,
    pub five_minutes: String,
    pub fifteen_minutes: String,
    pub one_hour: String,
    pub providers: String,
    pub claude_label: String,
    pub chatgpt_label: String,
    pub opencode_go_label: String,
    pub settings: String,
    pub start_with_windows: String,
    pub reset_position: String,
    pub size_smaller: String,
    pub size_larger: String,
    pub reset_size: String,
    pub controls: String,
    pub control_left_click: String,
    pub control_right_click: String,
    pub control_drag: String,
    pub control_ctrl_wheel: String,
    pub control_tray_click: String,
    pub tray_left_click: String,
    pub check_for_updates: String,
    pub checking_for_updates: String,
    pub up_to_date: String,
    pub update_failed: String,
    pub applying_update: String,
    pub update_available: String,
    pub update_via_winget: String,
    pub auto_update_check: String,
    pub auto_check_disabled: String,
    pub auto_check_hourly: String,
    pub auto_check_daily: String,
    pub auto_check_weekly: String,
    pub exit: String,
    pub restart: String,
    pub show_widget: String,
    pub only_over_chatgpt: String,
    pub used_word: String,
    pub remaining_word: String,
    pub reset_prefix: String,
    pub tokens_prefix: String,
    pub session_window: String,
    pub weekly_window: String,
    pub now: String,
    pub day_suffix: String,
    pub hour_suffix: String,
    pub minute_suffix: String,
    pub second_suffix: String,
    pub token_expired_title: String,
    pub token_expired_body: String,
    pub chatgpt_token_expired_title: String,
    pub chatgpt_token_expired_body: String,
    /// Body text for "your usage just crossed 80% of the 5h limit". The
    /// title is composed from the provider label + percent.
    pub threshold_80_body: String,
    /// Body text for the 95% threshold balloon.
    pub threshold_95_body: String,
    /// Title for the tray balloon shown on first launch after an auto-update.
    pub update_applied_title: String,
    /// Prefix for the tray balloon body. Call site appends the version.
    pub update_applied_body: String,
    /// Prefix for the rollback-failed MessageBox body. Call site appends
    /// the backup path and a separator with the expected target filename.
    pub update_rollback_failed_body: String,
}

pub struct I18n {
    strings: LocaleStrings,
}

impl I18n {
    pub fn load() -> Self {
        Self {
            strings: english_strings(),
        }
    }

    pub fn strings(&self) -> &LocaleStrings {
        &self.strings
    }
}

fn english_strings() -> LocaleStrings {
    LocaleStrings {
        window_title: "Claude Code Usage Bubble".into(),
        refresh: "Refresh".into(),
        update_frequency: "Update frequency".into(),
        one_minute: "1 minute".into(),
        five_minutes: "5 minutes".into(),
        fifteen_minutes: "15 minutes".into(),
        one_hour: "1 hour".into(),
        providers: "Providers".into(),
        claude_label: "Claude Code".into(),
        chatgpt_label: "Codex".into(),
        opencode_go_label: "OpenCode Go".into(),
        settings: "Settings".into(),
        start_with_windows: "Start with Windows".into(),
        reset_position: "Reset position".into(),
        size_smaller: "Make smaller".into(),
        size_larger: "Make larger".into(),
        reset_size: "Reset size".into(),
        controls: "Controls".into(),
        control_left_click: "Left-click: details".into(),
        control_right_click: "Right-click: menu".into(),
        control_drag: "Drag: move/snap".into(),
        control_ctrl_wheel: "Ctrl+Wheel: resize".into(),
        control_tray_click: "Tray click: show/hide".into(),
        tray_left_click: "Left-click: show/hide".into(),
        check_for_updates: "Check for updates".into(),
        checking_for_updates: "Checking for updates...".into(),
        up_to_date: "Up to date".into(),
        update_failed: "Update failed".into(),
        applying_update: "Applying update...".into(),
        update_available: "Update available".into(),
        update_via_winget: "via WinGet".into(),
        auto_update_check: "Auto-update check".into(),
        auto_check_disabled: "Disabled".into(),
        auto_check_hourly: "Hourly".into(),
        auto_check_daily: "Daily".into(),
        auto_check_weekly: "Weekly".into(),
        exit: "Exit".into(),
        restart: "Restart".into(),
        show_widget: "Show widget".into(),
        only_over_chatgpt: "Somente sobre o ChatGPT".into(),
        used_word: "usada".into(),
        remaining_word: "resta".into(),
        reset_prefix: "Falta para resetar".into(),
        tokens_prefix: "Tokens neste PC desde o reset".into(),
        session_window: "5h".into(),
        weekly_window: "7d".into(),
        now: "now".into(),
        day_suffix: "d".into(),
        hour_suffix: "h".into(),
        minute_suffix: "m".into(),
        second_suffix: "s".into(),
        token_expired_title: "Claude Code session expired".into(),
        token_expired_body: "Sign in again to keep tracking your usage.".into(),
        chatgpt_token_expired_title: "Codex session expired".into(),
        chatgpt_token_expired_body: "Sign in again to keep tracking your usage.".into(),
        threshold_80_body: "Approaching the weekly limit.".into(),
        threshold_95_body: "Weekly limit is close - consider easing up.".into(),
        update_applied_title: "Update applied".into(),
        update_applied_body: "Updated to v".into(),
        update_rollback_failed_body: "Update failed. Your original binary is saved at: ".into(),
    }
}

// ---------- Free-function helpers ----------

/// Format a `usage::Window` percentage + countdown as `"73% · 2h"`-style text.
/// Returns just the percentage when no reset time is available.
pub fn format_window(window: &crate::usage::Window, strings: &LocaleStrings) -> String {
    let pct = format!("{:.0}%", window.utilization);
    let cd = format_countdown(window.resets_at, strings);
    if cd.is_empty() {
        pct
    } else {
        format!("{pct} \u{00b7} {cd}")
    }
}

/// Fork (Rafael): explicit "used x remaining" window text, e.g.
/// "3% usada · 97% resta · 6d". Used by the expanded panel and tray
/// tooltip so it is always clear what was consumed vs. what is left.
pub fn format_window_remaining(window: &crate::usage::Window, strings: &LocaleStrings) -> String {
    let used = window.utilization.clamp(0.0, 100.0);
    let left = (100.0 - used).clamp(0.0, 100.0);
    let base = format!(
        "{:.0}% {} · {:.0}% {}",
        used, strings.used_word, left, strings.remaining_word
    );
    let cd = format_countdown(window.resets_at, strings);
    if cd.is_empty() {
        base
    } else {
        format!("{base} · {cd}")
    }
}

/// Fork (Rafael): precise two-unit countdown in full PT words —
/// "6 dias e 19 horas", "5 horas e 12 minutos", "3 minutos e 20 segundos".
/// Singular/plural handled; zero parts omitted ("6 dias", not "6 dias e 0 horas").
pub fn format_precise_countdown(resets_at: Option<SystemTime>, strings: &LocaleStrings) -> String {
    let Some(reset) = resets_at else {
        return String::new();
    };
    let secs = match reset.duration_since(SystemTime::now()) {
        Ok(d) => d.as_secs(),
        Err(_) => return strings.now.clone(),
    };
    let d = secs / 86_400;
    let h = secs % 86_400 / 3_600;
    let m = secs % 3_600 / 60;
    let s = secs % 60;
    fn qty(n: u64, one: &str, many: &str) -> String {
        format!("{n} {}", if n == 1 { one } else { many })
    }
    if d >= 1 {
        if h >= 1 {
            format!("{} e {}", qty(d, "dia", "dias"), qty(h, "hora", "horas"))
        } else {
            qty(d, "dia", "dias")
        }
    } else if h >= 1 {
        if m >= 1 {
            format!("{} e {}", qty(h, "hora", "horas"), qty(m, "minuto", "minutos"))
        } else {
            qty(h, "hora", "horas")
        }
    } else if m >= 1 {
        if s >= 1 {
            format!(
                "{} e {}",
                qty(m, "minuto", "minutos"),
                qty(s, "segundo", "segundos")
            )
        } else {
            qty(m, "minuto", "minutos")
        }
    } else {
        qty(s, "segundo", "segundos")
    }
}

/// Countdown only — used by the bubble, which renders the percent inside the
/// bar fill and only needs the time-to-reset on the right.
pub fn format_countdown(resets_at: Option<SystemTime>, strings: &LocaleStrings) -> String {
    let Some(reset) = resets_at else {
        return String::new();
    };
    let remaining = match reset.duration_since(SystemTime::now()) {
        Ok(d) => d,
        Err(_) => return strings.now.clone(),
    };
    format_countdown_secs(remaining.as_secs(), strings)
}

fn format_countdown_secs(total_secs: u64, strings: &LocaleStrings) -> String {
    let days = total_secs / 86_400;
    let hours = total_secs / 3_600;
    let mins = total_secs / 60;
    if days >= 1 {
        format!("{days}{}", strings.day_suffix)
    } else if hours >= 1 {
        format!("{hours}{}", strings.hour_suffix)
    } else if mins >= 1 {
        format!("{mins}{}", strings.minute_suffix)
    } else {
        format!("{total_secs}{}", strings.second_suffix)
    }
}

/// How long before `format_window`'s string would change.
/// Used by the countdown timer to refresh exactly when needed.
pub fn time_until_display_change(resets_at: Option<SystemTime>) -> Option<Duration> {
    let reset = resets_at?;
    let remaining = reset.duration_since(SystemTime::now()).ok()?;
    let secs = remaining.as_secs();
    let bucket_start = if secs / 86_400 >= 1 {
        (secs / 86_400) * 86_400
    } else if secs / 3_600 >= 1 {
        (secs / 3_600) * 3_600
    } else if secs / 60 >= 1 {
        (secs / 60) * 60
    } else {
        secs
    };
    Some(Duration::from_secs(secs.saturating_sub(bucket_start) + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_strings_include_required_menu_labels() {
        let strings = english_strings();
        for (name, value) in [
            ("refresh", strings.refresh.as_str()),
            ("providers", strings.providers.as_str()),
            ("settings", strings.settings.as_str()),
            ("size_smaller", strings.size_smaller.as_str()),
            ("size_larger", strings.size_larger.as_str()),
            ("reset_size", strings.reset_size.as_str()),
            ("controls", strings.controls.as_str()),
            ("control_left_click", strings.control_left_click.as_str()),
            ("control_right_click", strings.control_right_click.as_str()),
            ("control_drag", strings.control_drag.as_str()),
            ("control_ctrl_wheel", strings.control_ctrl_wheel.as_str()),
            ("control_tray_click", strings.control_tray_click.as_str()),
            ("tray_left_click", strings.tray_left_click.as_str()),
        ] {
            assert!(!value.trim().is_empty(), "empty string: {name}");
        }
    }
}
