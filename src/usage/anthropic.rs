// Claude (Anthropic) usage provider.
//
// Two HTTP paths: the dedicated `/api/oauth/usage` endpoint (preferred,
// returns structured JSON with ISO 8601 reset times) and a fallback POST
// to `/v1/messages` that exposes rate-limit data via response headers.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::creds::Locator;
use crate::net::Client;
use crate::usage::{headers, Error, ProviderId, UsageProvider, UsageWindows, Window};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const BETA_HEADER: &str = "oauth-2025-04-20";
const API_VERSION: &str = "2023-06-01";

const PROBE_MODELS: &[&str] = &[
    "claude-3-haiku-20240307",
    "claude-haiku-4-5-20251001",
];

pub struct ClaudeProvider {
    locator: Locator,
}

impl ClaudeProvider {
    pub fn new(locator: Locator) -> Self {
        Self { locator }
    }

    pub fn locator(&self) -> &Locator {
        &self.locator
    }
}

impl UsageProvider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn poll(&mut self, http: &Client) -> Result<UsageWindows, Error> {
        let source = self.locator.first_available().ok_or(Error::NoCredentials)?;
        let token = source.read()?;
        if token_is_expired(token.expires_at_unix_ms) {
            return Err(Error::AuthRequired);
        }
        fetch_with_fallback(http, &token.access_token)
    }
}

fn fetch_with_fallback(http: &Client, token: &str) -> Result<UsageWindows, Error> {
    // A monitoring widget must not send a billable model prompt as a probe.
    try_usage_endpoint(http, token)?.ok_or_else(|| Error::BadResponse("Claude usage endpoint unavailable".into()))
}
fn try_usage_endpoint(http: &Client, token: &str) -> Result<Option<UsageWindows>, Error> {
    let resp = match http
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", BETA_HEADER)
        .send()
    {
        Ok(r) => r,
        Err(crate::net::Error::Status(code)) if code == 401 || code == 403 => {
            return Err(Error::AuthRequired);
        }
        Err(_) => return Ok(None),
    };
    if resp.status() == 401 || resp.status() == 403 {
        return Err(Error::AuthRequired);
    }
    if !(200..300).contains(&resp.status()) {
        return Ok(None);
    }
    let body: OauthUsage = match resp.json() {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    let mut primary = body.five_hour.map(bucket_to_window).unwrap_or_default();
    if primary.reported { primary.duration_secs=Some(18000); }
    let mut secondary = body.seven_day.map(bucket_to_window).unwrap_or_default();
    if secondary.reported { secondary.duration_secs=Some(604800); }
    Ok(Some(UsageWindows { primary, secondary }))
}

fn try_messages_endpoint(http: &Client, token: &str) -> Result<UsageWindows, Error> {
    for model in PROBE_MODELS {
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "."}],
        });
        let resp = match http
            .post(MESSAGES_URL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", BETA_HEADER)
            .json_body(&body)
            .and_then(|rb| rb.send())
        {
            Ok(r) => r,
            Err(crate::net::Error::Status(code)) if code == 401 || code == 403 => {
                return Err(Error::AuthRequired);
            }
            Err(_) => continue,
        };
        if resp.status() == 401 || resp.status() == 403 {
            return Err(Error::AuthRequired);
        }
        // Even an error response from Messages can carry rate-limit headers.
        if resp.header("anthropic-ratelimit-unified-5h-utilization").is_some()
            || resp.header("anthropic-ratelimit-unified-7d-utilization").is_some()
        {
            return Ok(headers::parse_anthropic(&resp));
        }
    }
    Err(Error::BadResponse(
        "no rate-limit headers in messages response".into(),
    ))
}

fn bucket_to_window(bucket: Bucket) -> Window {
    Window {
        utilization: bucket.utilization.clamp(0.0, 100.0),
        resets_at: bucket.resets_at.as_deref().and_then(parse_iso8601),
        duration_secs: None,
        reported: true,
    }
}

fn has_reset_times(w: &UsageWindows) -> bool {
    w.primary.resets_at.is_some() && w.secondary.resets_at.is_some()
}

fn merge_resets(mut primary: UsageWindows, fallback: UsageWindows) -> UsageWindows {
    if primary.primary.resets_at.is_none() {
        primary.primary.resets_at = fallback.primary.resets_at;
    }
    if primary.secondary.resets_at.is_none() {
        primary.secondary.resets_at = fallback.secondary.resets_at;
    }
    primary
}

fn token_is_expired(expires_at_unix_ms: Option<i64>) -> bool {
    let Some(exp_ms) = expires_at_unix_ms else {
        return false;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now_ms >= exp_ms
}

// --- ISO 8601 parsing (minimal — handles "YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]") ---

fn parse_iso8601(s: &str) -> Option<SystemTime> {
    let (date, time_with_offset) = s.split_once('T')?;

    // Split the time-and-offset on the first 'Z' / '+' / '-' marker.
    let offset_pos = time_with_offset
        .char_indices()
        .find(|(_, c)| matches!(c, 'Z' | '+' | '-'))
        .map(|(i, _)| i);
    let (time, offset_str) = match offset_pos {
        Some(p) => (&time_with_offset[..p], &time_with_offset[p..]),
        None => (time_with_offset, ""),
    };
    let time = time.split_once('.').map_or(time, |(t, _)| t);

    let date_parts: Vec<&str> = date.split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let y: u64 = date_parts[0].parse().ok()?;
    let mo: u64 = date_parts[1].parse().ok()?;
    let d: u64 = date_parts[2].parse().ok()?;
    if y < 1970 || mo == 0 || mo > 12 || d == 0 {
        return None;
    }
    const DAYS_IN_MONTH: [u64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let max_day = DAYS_IN_MONTH[mo as usize] + if mo == 2 && is_leap(y) { 1 } else { 0 };
    if d > max_day {
        return None;
    }

    let time_parts: Vec<&str> = time.split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let h: u64 = time_parts[0].parse().ok()?;
    let mi: u64 = time_parts[1].parse().ok()?;
    let se: u64 = time_parts[2].parse().ok()?;

    let offset_minutes: i64 = if offset_str.is_empty() || offset_str == "Z" {
        0
    } else {
        let sign: i64 = if offset_str.starts_with('+') {
            1
        } else if offset_str.starts_with('-') {
            -1
        } else {
            return None;
        };
        let (oh_str, om_str) = offset_str[1..].split_once(':')?;
        let oh: i64 = oh_str.parse().ok()?;
        let om: i64 = om_str.parse().ok()?;
        sign * (oh * 60 + om)
    };

    let mut days: u64 = 0;
    for year in 1970..y {
        days += if is_leap(year) { 366 } else { 365 };
    }
    for month in 1..mo {
        days += DAYS_IN_MONTH[month as usize];
        if month == 2 && is_leap(y) {
            days += 1;
        }
    }
    days += d - 1;

    let local_secs = days * 86_400 + h * 3_600 + mi * 60 + se;
    let utc_secs = (local_secs as i64) - offset_minutes * 60;
    if utc_secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(utc_secs as u64))
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// --- JSON shape ---

#[derive(Deserialize)]
struct OauthUsage {
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
}

#[derive(Deserialize)]
struct Bucket {
    utilization: f64,
    resets_at: Option<String>,
}
