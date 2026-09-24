// Fork (Rafael): "tokens neste PC desde o reset".
//
// Sums per-turn `total_tokens` from the local Codex CLI session rollouts
// (`~/.codex/sessions/**/*.jsonl`, `event_msg` lines carrying
// `payload.info.last_token_usage.total_tokens` with an RFC3339 `timestamp`),
// counting only events at/after the given weekly-reset boundary.
//
// Notes & limits (shown in the UI wording — "neste PC"):
// - Only sessions recorded ON THIS MACHINE count. Usage from the desktop app
//   on other machines / remote hosts is NOT included.
// - Read-only: never writes, never touches auth.json.
// - Results are cached per file (len + mtime); only new/changed files are
//   re-scanned, so the per-minute poll stays cheap.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct FileEntry {
    len: u64,
    mtime_secs: u64,
    sum: u64,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, FileEntry>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, FileEntry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Total tokens consumed on this machine since `reset`. `None` when the
/// sessions directory is missing/unreadable (UI shows a dash).
pub fn tokens_since_reset(reset: SystemTime) -> Option<u64> {
    let sessions = dirs::home_dir()?.join(".codex").join("sessions");
    if !sessions.is_dir() {
        return None;
    }
    let reset_unix = reset
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)?;

    // Collect rollout files (recursive, bounded depth).
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = vec![(sessions, 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
    if files.is_empty() {
        return None;
    }

    let mut total: u64 = 0;
    let mut guard = cache().lock().expect("token cache mutex poisoned");
    for path in files {
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let len = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(hit) = guard.get(&path) {
            if hit.len == len && hit.mtime_secs == mtime {
                total = total.saturating_add(hit.sum);
                continue;
            }
        }
        let sum = scan_file(&path, reset_unix);
        guard.insert(
            path,
            FileEntry {
                len,
                mtime_secs: mtime,
                sum,
            },
        );
        total = total.saturating_add(sum);
    }
    Some(total)
}

fn scan_file(path: &PathBuf, reset_unix: i64) -> u64 {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut sum: u64 = 0;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts_ok = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(rfc3339_to_unix)
            .map(|t| t >= reset_unix)
            .unwrap_or(false);
        if !ts_ok {
            continue;
        }
        if let Some(n) = v
            .pointer("/payload/info/last_token_usage/total_tokens")
            .and_then(|n| n.as_u64())
        {
            sum = sum.saturating_add(n);
        }
    }
    sum
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.sss](Z|±HH:MM)` to unix seconds.
/// Hand-rolled so no extra date crate is needed; returns None on any
/// malformed input (the line is then skipped).
fn rfc3339_to_unix(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |i: usize, n: usize| -> Option<i64> {
        let mut v: i64 = 0;
        for k in i..i + n {
            let c = *b.get(k)?;
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    if b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || b.get(10) != Some(&b'T')
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
    {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, se) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let days = days_from_civil(y, mo, d)?;
    let mut i = 19;
    while i < b.len() && (b[i] == b'.' || b[i].is_ascii_digit()) {
        i += 1;
    }
    let mut off_secs: i64 = 0;
    if i < b.len() {
        match b[i] {
            b'Z' | b'z' => {}
            b'+' | b'-' => {
                // "+HH:MM" = local ahead of UTC → subtract the offset.
                let sign = if b[i] == b'+' { -1 } else { 1 };
                let oh = num(i + 1, 2)?;
                let om = if b.get(i + 3) == Some(&b':') {
                    num(i + 4, 2)?
                } else {
                    num(i + 3, 2)?
                };
                off_secs = sign * (oh * 3600 + om * 60);
            }
            _ => return None,
        }
    }
    Some(days * 86400 + h * 3600 + mi * 60 + se + off_secs)
}

/// Howard Hinnant's days-from-civil; days since 1970-01-01. None for
/// out-of-range month/day.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// Format like 1234567 → "1.234.567" (pt-BR thousands separator).
pub fn format_tokens(n: u64) -> String {    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Compact form for tiny surfaces: 55300000 -> "55,3M", 12400 -> "12,4K".
pub fn format_compact(n: i64) -> String {
    let n = n.max(0);
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0).replace('.', ",")
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0).replace('.', ",")
    } else {
        n.to_string()
    }
}

/// Full-words PT form for the bubble value line, e.g. "55,3 Milhões tokens".
/// Returns (number_part, unit_word).
pub fn format_tokens_words(n: i64) -> (String, &'static str) {
    let n = n.max(0);
    if n >= 1_000_000 {
        let v = n as f64 / 1_000_000.0;
        let num = format!("{v:.1}").replace('.', ",");
        let unit = if (v - 1.0).abs() < 0.049 {
            "Milhão tokens"
        } else {
            "Milhões tokens"
        };
        (num, unit)
    } else if n >= 1_000 {
        (
            format!("{:.1}", n as f64 / 1_000.0).replace('.', ","),
            "mil tokens",
        )
    } else if n == 1 {
        ("1".into(), "token")
    } else {
        (n.to_string(), "tokens")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_utc_parses() {
        // 2026-08-26T17:32:12Z == 1787765532.
        assert_eq!(rfc3339_to_unix("2026-08-26T17:32:12.811Z"), Some(1787765532));
        assert_eq!(rfc3339_to_unix("2026-08-26T17:32:12Z"), Some(1787765532));
    }

    #[test]
    fn rfc3339_offset_parses() {
        assert_eq!(
            rfc3339_to_unix("2026-09-23T12:00:00-03:00"),
            rfc3339_to_unix("2026-09-23T15:00:00Z")
        );
    }

    #[test]
    fn rfc3339_rejects_garbage() {
        assert_eq!(rfc3339_to_unix("not-a-date"), None);
        assert_eq!(rfc3339_to_unix("2026-13-99T99:99:99Z"), None);
        assert_eq!(rfc3339_to_unix(""), None);
    }

    #[test]
    fn tokens_format_pt() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(97), "97");
        assert_eq!(format_tokens(17208), "17.208");
        assert_eq!(format_tokens(1234567), "1.234.567");
        assert_eq!(format_compact(55300000), "55,3M");
        assert_eq!(format_compact(12400), "12,4K");
        assert_eq!(format_compact(999), "999");
        assert_eq!(format_tokens_words(55288967), ("55,3".into(), "Milhões tokens"));
        assert_eq!(format_tokens_words(1_000_000), ("1,0".into(), "Milhão tokens"));
        assert_eq!(format_tokens_words(2500), ("2,5".into(), "mil tokens"));
        assert_eq!(format_tokens_words(1), ("1".into(), "token"));
        assert_eq!(format_tokens_words(0), ("0".into(), "tokens"));
    }

    #[test]
    fn civil_epoch_is_zero() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2026, 9, 23), Some(20719));
        assert_eq!(days_from_civil(2026, 13, 1), None);
    }
}
