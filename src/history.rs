//! Append-only observations for later quota calibration. Never infer tokens
//! from percentages; service aggregates and local counts have separate scopes.
use chrono::{NaiveDate, Utc};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const RETENTION_DAYS: i64 = 35;
#[derive(Default)]
struct State { quota: Option<Value>, account: Option<Value>, prune_day: Option<NaiveDate> }
fn root() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("ClaudeCodeUsageBubble").join("history"))
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let mut s = State::default();
        // Recover the preceding observation across restarts to detect resets
        // that occurred while the widget was closed. Do not invent their time.
        if let Some(root) = root() {
            let mut files: Vec<_> = fs::read_dir(root).into_iter().flatten().flatten()
                .map(|e| e.path()).filter(|p| file_date(p).is_some()).collect();
            files.sort();
            for path in files.iter().rev() {
                if let Ok(text) = fs::read_to_string(path) {
                    s.quota = text.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok())
                        .find(|v| v["kind"] == "weekly_quota").map(|v| v["data"].clone());
                    if s.quota.is_some() { break; }
                }
            }
        }
        Mutex::new(s)
    })
}
fn file_date(path: &Path) -> Option<NaiveDate> {
    if path.extension()?.to_str()? != "jsonl" { return None; }
    NaiveDate::parse_from_str(path.file_stem()?.to_str()?, "%Y-%m-%d").ok()
}
fn expired(date: NaiveDate, today: NaiveDate) -> bool {
    date < today - chrono::Duration::days(RETENTION_DAYS)
}
fn append(kind: &str, data: &Value, s: &mut State) -> std::io::Result<()> {
    let root = root().ok_or_else(|| std::io::Error::other("history directory unavailable"))?;
    fs::create_dir_all(&root)?;
    let now = Utc::now();
    let today = now.date_naive();
    if s.prune_day != Some(today) {
        for entry in fs::read_dir(&root)?.flatten() {
            if file_date(&entry.path()).is_some_and(|d| expired(d, today)) {
                fs::remove_file(entry.path())?;
            }
        }
        s.prune_day = Some(today);
    }
    let row = json!({"schema":1,"observed_at":now.to_rfc3339(),"kind":kind,"data":data});
    let mut file = OpenOptions::new().create(true).append(true)
        .open(root.join(format!("{today}.jsonl")))?;
    writeln!(file, "{row}")?;
    file.flush()
}
fn reset_reason(previous: &Value, next: &Value, now: u64) -> Option<&'static str> {
    let old_deadline = previous["resets_at"].as_u64();
    let new_deadline = next["resets_at"].as_u64();
    let deadline_changed = matches!((old_deadline,new_deadline),(Some(a),Some(b)) if a.abs_diff(b)>60);
    let drop = matches!((previous["used_pct"].as_f64(),next["used_pct"].as_f64()),(Some(a),Some(b)) if a-b>=1.0);
    if !deadline_changed && !drop { return None; }
    Some(if old_deadline.is_some_and(|d| d <= now) { "scheduled_rollover_observed" }
        else { "early_reset_or_quota_adjustment_observed" })
}
pub fn weekly(used: f64, reset: Option<SystemTime>) {
    if !(0.0..=100.0).contains(&used) { return; }
    let now = Utc::now().timestamp().max(0) as u64;
    let local = crate::local_usage::latest();
    let next = json!({"used_pct":used,"resets_at":reset.and_then(|t|t.duration_since(UNIX_EPOCH).ok()).map(|d|d.as_secs()),
        "source":"wham_weekly","observed_unix":now,
        "local_tokens_today":local.tokens,"local_day":local.day.map(|d|d.to_string())});
    let mut s = state().lock().unwrap_or_else(|p|p.into_inner());
    let result = (|| {
        if let Some(old) = &s.quota {
            if let Some(reason) = reset_reason(old, &next, now) {
                let event = json!({"reason":reason,"previous":old,"current":next,
                    "note":"Change observed between samples; exact reset time and manual/automatic cause unknown"});
                append("reset_observed", &event, &mut s)?;
            }
        }
        append("weekly_quota", &next, &mut s)?;
        s.quota = Some(next);
        Ok::<_,std::io::Error>(())
    })();
    if result.is_err() { log::warn!("Could not persist quota history"); }
}
pub fn account(usage: &Value) {
    // Whitelist only numeric usage aggregates. No threads, text, credentials,
    // names, email addresses or account identifiers are written.
    let buckets: Vec<_> = usage["dailyUsageBuckets"].as_array().into_iter().flatten()
        .filter_map(|b| Some(json!({"startDate":b["startDate"].as_str()?,"tokens":b["tokens"].as_u64()?}))).collect();
    let next = json!({"source":"account_usage_read","lifetime_tokens":usage.pointer("/summary/lifetimeTokens").or_else(||usage.get("lifetimeTokens")).and_then(Value::as_u64),"daily_buckets":buckets});
    let mut s = state().lock().unwrap_or_else(|p|p.into_inner());
    if s.account.as_ref() == Some(&next) { return; }
    if append("account_tokens", &next, &mut s).is_ok() { s.account = Some(next); }
    else { log::warn!("Could not persist token history"); }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn resets_and_adjustments_are_not_silently_combined() {
        let old=json!({"used_pct":90,"resets_at":1000});
        assert_eq!(reset_reason(&old,&json!({"used_pct":5,"resets_at":605800}),1001),Some("scheduled_rollover_observed"));
        assert_eq!(reset_reason(&old,&json!({"used_pct":5,"resets_at":605800}),900),Some("early_reset_or_quota_adjustment_observed"));
        assert_eq!(reset_reason(&old,&json!({"used_pct":91,"resets_at":1001}),900),None);
        assert_eq!(reset_reason(&json!({"used_pct":90}),&json!({"used_pct":5}),900),Some("early_reset_or_quota_adjustment_observed"));
    }
    #[test] fn retains_at_least_thirty_complete_days() {
        let today=NaiveDate::from_ymd_opt(2026,11,6).unwrap();
        assert!(!expired(today-chrono::Duration::days(35),today));
        assert!(expired(today-chrono::Duration::days(36),today));
        assert!(file_date(Path::new("other.jsonl")).is_none());
    }
}

