// Codex (ChatGPT) usage provider.
//
// Single endpoint: `/backend-api/wham/usage`. Response shape includes
// `rate_limit.{primary_window,secondary_window}.{used_percent,reset_at}`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::creds::Locator;
use crate::net::Client;
use crate::usage::{Error, ProviderId, UsageProvider, UsageWindows, Window};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub struct ChatGptProvider {
    locator: Locator,
}

impl ChatGptProvider {
    pub fn new(locator: Locator) -> Self {
        Self { locator }
    }

    pub fn locator(&self) -> &Locator {
        &self.locator
    }
}

impl UsageProvider for ChatGptProvider {
    fn id(&self) -> ProviderId {
        ProviderId::ChatGpt
    }

    fn poll(&mut self, http: &Client) -> Result<UsageWindows, Error> {
        let source = self.locator.first_available().ok_or(Error::NoCredentials)?;
        let token = source.read()?;
        let mut req = http
            .get(USAGE_URL)
            .header("Authorization", &format!("Bearer {}", token.access_token))
            .header("User-Agent", "codex-cli");
        if let Some(account_id) = token.account_id.as_deref().filter(|s| !s.is_empty()) {
            req = req.header("ChatGPT-Account-Id", account_id);
        }
        let resp = match req.send() {
            Ok(r) => r,
            Err(crate::net::Error::Status(code)) if code == 401 || code == 403 => {
                return Err(Error::AuthRequired);
            }
            Err(e) => return Err(Error::Network(e)),
        };
        if resp.status() == 401 || resp.status() == 403 {
            return Err(Error::AuthRequired);
        }
        if !(200..300).contains(&resp.status()) {
            return Err(Error::BadResponse(format!(
                "Codex usage endpoint returned {}",
                resp.status()
            )));
        }
        let body: Envelope = resp
            .json()
            .map_err(|e| Error::BadResponse(format!("JSON parse: {e}")))?;
        // Fork discovery: log top-level field NAMES only (never values —
        // values may contain account data). Shows whether token buckets
        // ride along in this response.
        let mut extra_keys: Vec<&str> = body.extra.keys().map(String::as_str).collect();
        extra_keys.sort_unstable();
        log::info!("wham/usage extra top-level keys: {extra_keys:?}");
        for probe in ["model_usage", "credits", "additional_rate_limits"] {
            if let Some(v) = body.extra.get(probe) {
                log::info!("wham/usage shape {probe}: {}", shape_of(v, 0));
            }
        }
        envelope_to_windows(body)
            .ok_or_else(|| Error::BadResponse("missing rate_limit section".into()))
    }
}

fn envelope_to_windows(envelope: Envelope) -> Option<UsageWindows> {
    let rl = envelope.rate_limit.flatten_box()?;
    Some(UsageWindows {
        primary: rl
            .primary_window
            .flatten_box()
            .map(window_from)
            .unwrap_or_default(),
        secondary: rl
            .secondary_window
            .flatten_box()
            .map(window_from)
            .unwrap_or_default(),
    })
}

fn window_from(w: ApiWindow) -> Window {
    Window {
        utilization: w.used_percent.clamp(0.0, 100.0),
        resets_at: unix_to_systemtime(Some(w.reset_at)),
        duration_secs: w.limit_window_seconds.filter(|n| *n > 0),
        reported: true,
    }
}

fn unix_to_systemtime(secs: Option<i64>) -> Option<SystemTime> {
    let s = secs?;
    if s < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(s as u64))
}

/// Fork discovery helper: describe a JSON value's SHAPE (key names and
/// scalar types) without revealing any values. Depth-capped; arrays show
/// their first element's shape only.
fn shape_of(v: &serde_json::Value, depth: usize) -> String {
    if depth > 3 {
        return "…".into();
    }
    match v {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(_) => "bool".into(),
        serde_json::Value::Number(_) => "num".into(),
        serde_json::Value::String(_) => "str".into(),
        serde_json::Value::Array(items) => {
            let first = items
                .first()
                .map(|x| shape_of(x, depth + 1))
                .unwrap_or_default();
            format!("[{}; len={}]", first, items.len())
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .take(25)
                .map(|k| format!("{k}:{}", shape_of(&map[*k], depth + 1)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    rate_limit: Option<Option<Box<RateLimit>>>,
    /// Fork: capture-but-ignore everything else so we can discover new
    /// fields (e.g. daily token buckets) by logging KEYS only, never values.
    #[serde(flatten, default)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RateLimit {
    primary_window: Option<Option<Box<ApiWindow>>>,
    secondary_window: Option<Option<Box<ApiWindow>>>,
}

#[derive(Deserialize)]
struct ApiWindow {
    used_percent: f64,
    reset_at: i64,
    limit_window_seconds: Option<u64>,
}

// Helpers used to make `Option<Option<Box<…>>>` flatten cleanly. We can't
// reuse the std `Option::flatten` name — the inherent method (which returns
// `Option<Box<T>>`) would shadow this trait method.
trait FlattenBoxed<T> {
    fn flatten_box(self) -> Option<T>;
}

#[cfg(test)]
mod quota_tests {
    use super::*;
    fn parse(value:serde_json::Value)->UsageWindows {
        envelope_to_windows(serde_json::from_value(value).unwrap()).unwrap()
    }
    #[test]
    fn single_weekly_limit_is_not_duplicated_or_guessed_as_five_hours() {
        let w=parse(serde_json::json!({"rate_limit":{
            "primary_window":{"used_percent":40,"reset_at":2000000000_i64,"limit_window_seconds":604800},
            "secondary_window":null}}));
        let shown=w.displayed();
        assert_eq!(shown.len(),1);assert_eq!(shown[0].label(),"7d");
        assert_eq!(shown[0].utilization,40.0);assert!(w.weekly().is_some());
    }
    #[test]
    fn plus_limits_keep_independent_percentages_and_resets_even_if_slots_swap() {
        for swapped in [false,true] {
            let short=serde_json::json!({"used_percent":20,"reset_at":1900000000_i64,"limit_window_seconds":18000});
            let long=serde_json::json!({"used_percent":80,"reset_at":2000000000_i64,"limit_window_seconds":604800});
            let w=parse(serde_json::json!({"rate_limit":{
                "primary_window":if swapped{&long}else{&short},"secondary_window":if swapped{&short}else{&long}}}));
            let shown=w.displayed();assert_eq!(shown.len(),2);
            assert_eq!(shown[0].label(),"5h");assert_eq!(shown[0].utilization,20.0);
            assert_eq!(shown[0].resets_at,unix_to_systemtime(Some(1900000000)));
            assert_eq!(shown[1].label(),"7d");assert_eq!(shown[1].utilization,80.0);
            assert_eq!(shown[1].resets_at,unix_to_systemtime(Some(2000000000)));
            assert_eq!(w.pressure(),Some(80.0));assert_eq!(w.weekly().unwrap().utilization,80.0);
        }
    }
    #[test]
    fn absent_is_not_free_quota_and_unknown_duration_is_not_inferred_from_reset() {
        let empty=parse(serde_json::json!({"rate_limit":{"primary_window":null,"secondary_window":null}}));
        assert!(empty.displayed().is_empty());assert_eq!(empty.pressure(),None);
        let unknown=parse(serde_json::json!({"rate_limit":{"primary_window":{"used_percent":0,"reset_at":2000000000_i64}}}));
        assert_eq!(unknown.displayed()[0].label(),"Cota");assert_eq!(unknown.pressure(),Some(0.0));
        assert!(unknown.weekly().is_none());
    }
}
impl<T> FlattenBoxed<T> for Option<Option<Box<T>>> {
    fn flatten_box(self) -> Option<T> {
        self.and_then(|inner| inner.map(|b| *b))
    }
}
