// Embedded TOML-based localisation.
//
// Each supported language lives in `locales/<code>.toml`. At startup we
// `include_str!` every file, parse them with `toml`, and stash them in a
// HashMap keyed by language code. The active language defaults to whatever
// Windows reports for the user's preferred UI language; the menu lets the
// user override that.
//
// Adding a translation: copy `en.toml` to `<code>.toml`, translate the
// strings, then add one `include_str!` entry to `RAW_LOCALES` below.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

pub mod detect;

const FALLBACK_CODE: &str = "en";

/// The strings every UI module needs. Field names map 1:1 to TOML keys.
#[derive(Clone, Debug, Deserialize)]
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
    pub language: String,
    pub system_default: String,
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
    /// title is composed from the provider label + percent so it does not
    /// need to be translated separately.
    pub threshold_80_body: String,
    /// Body text for the 95% threshold balloon.
    pub threshold_95_body: String,
    /// Title for the tray balloon shown on first launch after an auto-update.
    pub update_applied_title: String,
    /// Prefix for the tray balloon body. Call site appends the version (e.g. "0.1.10").
    pub update_applied_body: String,
    /// Prefix for the rollback-failed MessageBox body. Call site appends
    /// the backup path and a separator with the expected target filename.
    pub update_rollback_failed_body: String,
}

#[derive(Deserialize)]
struct LocaleFile {
    code: String,
    native_name: String,
    #[serde(flatten)]
    strings: LocaleStrings,
}

const RAW_LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("locales/en.toml")),
    ("ja", include_str!("locales/ja.toml")),
    ("ko", include_str!("locales/ko.toml")),
    ("vi", include_str!("locales/vi.toml")),
    ("zh-TW", include_str!("locales/zh-TW.toml")),
];

pub struct I18n {
    /// Sorted by code so menus list languages deterministically.
    available: BTreeMap<String, (String, LocaleStrings)>,
    active: String,
}

impl I18n {
    /// Load all embedded TOML files and pick an active language.
    ///
    /// `requested` overrides system detection. `None` means "ask Windows".
    pub fn load(requested: Option<&str>) -> Self {
        let mut available = BTreeMap::new();
        for (code, body) in RAW_LOCALES {
            match toml::from_str::<LocaleFile>(body) {
                Ok(file) => {
                    available.insert(file.code.clone(), (file.native_name, file.strings));
                }
                Err(e) => {
                    log::error!("failed to parse locale {code}: {e}");
                }
            }
        }
        if !available.contains_key(FALLBACK_CODE) {
            // Embedded TOMLs are validated by tests; this should never
            // happen in practice. Fall through with whatever we have.
            log::error!("fallback locale '{FALLBACK_CODE}' missing");
        }

        let active = requested
            .and_then(|c| normalise(c, &available))
            .or_else(|| detect::detect_system_locale().and_then(|c| normalise(&c, &available)))
            .unwrap_or_else(|| FALLBACK_CODE.to_string());

        Self { available, active }
    }

    pub fn strings(&self) -> &LocaleStrings {
        self.available
            .get(&self.active)
            .map(|(_, s)| s)
            .unwrap_or_else(|| {
                // Defensive: if `active` was set to something unavailable
                // (shouldn't happen given `load` validates) — fall back.
                &self
                    .available
                    .get(FALLBACK_CODE)
                    .expect("fallback locale must exist")
                    .1
            })
    }

    pub fn active_code(&self) -> &str {
        &self.active
    }

    /// Iterate `(code, native_name)` pairs in stable order.
    pub fn available(&self) -> impl Iterator<Item = (&str, &str)> {
        self.available
            .iter()
            .map(|(code, (name, _))| (code.as_str(), name.as_str()))
    }

    pub fn set_active(&mut self, requested: Option<&str>) {
        let new_active = requested
            .and_then(|c| normalise(c, &self.available))
            .or_else(|| detect::detect_system_locale().and_then(|c| normalise(&c, &self.available)))
            .unwrap_or_else(|| FALLBACK_CODE.to_string());
        self.active = new_active;
    }
}

/// Resolve a user-supplied or system-supplied locale code to one we have.
///
/// Handles `en_US`, `en-US`, `EN`, `zh-Hant-TW`, etc. by progressive
/// fallback: exact → ASCII-lower exact → prefix match.
fn normalise(input: &str, available: &BTreeMap<String, (String, LocaleStrings)>) -> Option<String> {
    let cleaned = input.trim().replace('_', "-");
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("system") {
        return None;
    }
    // Exact (case-insensitive)
    for key in available.keys() {
        if key.eq_ignore_ascii_case(&cleaned) {
            return Some(key.clone());
        }
    }
    // Special-case: Traditional Chinese variants → zh-TW
    let lower = cleaned.to_ascii_lowercase();
    if lower.starts_with("zh")
        && (lower.contains("tw") || lower.contains("hk") || lower.contains("hant"))
    {
        if available.contains_key("zh-TW") {
            return Some("zh-TW".to_string());
        }
    }
    // Prefix fallback (e.g. "en-US" → "en")
    let prefix = lower.split('-').next().unwrap_or("");
    if !prefix.is_empty() {
        for key in available.keys() {
            if key
                .split('-')
                .next()
                .map(str::to_ascii_lowercase)
                .as_deref()
                == Some(prefix)
            {
                return Some(key.clone());
            }
        }
    }
    None
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
    fn embedded_locales_parse_and_include_required_control_strings() {
        let mut has_fallback = false;
        for (expected_code, body) in RAW_LOCALES {
            let file = toml::from_str::<LocaleFile>(body)
                .unwrap_or_else(|e| panic!("locale {expected_code} failed to parse: {e}"));
            assert_eq!(file.code, *expected_code);
            has_fallback |= file.code == FALLBACK_CODE;

            let strings = file.strings;
            for (name, value) in [
                ("size_smaller", strings.size_smaller.as_str()),
                ("providers", strings.providers.as_str()),
                ("size_larger", strings.size_larger.as_str()),
                ("opencode_go_label", strings.opencode_go_label.as_str()),
                ("reset_size", strings.reset_size.as_str()),
                ("controls", strings.controls.as_str()),
                ("control_left_click", strings.control_left_click.as_str()),
                ("control_right_click", strings.control_right_click.as_str()),
                ("control_drag", strings.control_drag.as_str()),
                ("control_ctrl_wheel", strings.control_ctrl_wheel.as_str()),
                ("control_tray_click", strings.control_tray_click.as_str()),
                ("tray_left_click", strings.tray_left_click.as_str()),
            ] {
                assert!(
                    !value.trim().is_empty(),
                    "locale {expected_code} has empty {name}"
                );
            }
        }
        assert!(has_fallback, "fallback locale {FALLBACK_CODE} missing");
    }

    #[test]
    fn locale_schema_rejects_missing_or_malformed_control_strings() {
        let (_, fallback_body) = RAW_LOCALES
            .iter()
            .find(|(code, _)| *code == FALLBACK_CODE)
            .expect("fallback locale fixture must exist");

        let missing_control =
            fallback_body.replace("tray_left_click = \"Left-click: show/hide\"\n", "");
        assert!(
            toml::from_str::<LocaleFile>(&missing_control).is_err(),
            "missing tray_left_click should fail locale deserialization"
        );

        let malformed_control = fallback_body.replace(
            "control_tray_click = \"Tray click: show/hide\"",
            "control_tray_click = [\"Tray click: show/hide\"]",
        );
        assert!(
            toml::from_str::<LocaleFile>(&malformed_control).is_err(),
            "malformed control_tray_click should fail locale deserialization"
        );
    }

    #[test]
    fn locale_schema_rejects_missing_provider_strings() {
        let (_, fallback_body) = RAW_LOCALES
            .iter()
            .find(|(code, _)| *code == FALLBACK_CODE)
            .expect("fallback locale fixture must exist");

        let missing_providers = fallback_body.replace("providers = \"Providers\"\n", "");
        assert!(
            toml::from_str::<LocaleFile>(&missing_providers).is_err(),
            "missing providers should fail locale deserialization"
        );

        let missing_opencode = fallback_body.replace("opencode_go_label = \"OpenCode Go\"\n", "");
        assert!(
            toml::from_str::<LocaleFile>(&missing_opencode).is_err(),
            "missing opencode_go_label should fail locale deserialization"
        );
    }
}
