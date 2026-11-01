// Parse Anthropic rate-limit headers into `UsageWindows`.
//
// The Messages API returns the user's remaining quota in response headers
// when the dedicated usage endpoint isn't available. Header names:
//   anthropic-ratelimit-unified-5h-utilization (0.0–1.0)
//   anthropic-ratelimit-unified-5h-reset       (Unix seconds)
//   anthropic-ratelimit-unified-7d-utilization
//   anthropic-ratelimit-unified-7d-reset

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::net::Response;
use crate::usage::{UsageWindows, Window};

pub fn parse_anthropic(response: &Response) -> UsageWindows {
    UsageWindows {
        primary: Window {
            duration_secs: Some(18000),
            reported: response.header("anthropic-ratelimit-unified-5h-utilization").is_some(),
            utilization: (header_f64(response, "anthropic-ratelimit-unified-5h-utilization")
                * 100.0)
                .clamp(0.0, 100.0),
            resets_at: unix_to_systemtime(header_i64(
                response,
                "anthropic-ratelimit-unified-5h-reset",
            )),
        },
        secondary: Window {
            duration_secs: Some(604800),
            reported: response.header("anthropic-ratelimit-unified-7d-utilization").is_some(),
            utilization: (header_f64(response, "anthropic-ratelimit-unified-7d-utilization")
                * 100.0)
                .clamp(0.0, 100.0),
            resets_at: unix_to_systemtime(header_i64(
                response,
                "anthropic-ratelimit-unified-7d-reset",
            )),
        },
    }
}

fn header_f64(response: &Response, name: &str) -> f64 {
    response
        .header(name)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

fn header_i64(response: &Response, name: &str) -> Option<i64> {
    response.header(name).and_then(|s| s.parse().ok())
}

fn unix_to_systemtime(secs: Option<i64>) -> Option<SystemTime> {
    let s = secs?;
    if s < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(s as u64))
}
