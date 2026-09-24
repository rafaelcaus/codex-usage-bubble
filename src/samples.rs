// Fork (Rafael): burn-rate sampling for the "pace" projection.
//
// Mirrors the dashboard widget's idea: observe the weekly-used % over a
// rolling window of up to 15 minutes and project when the quota runs out at
// the recent burn rate. States: Collecting (not enough history yet),
// Idle (no measurable burn), Live (projection).
//
// Samples persist in %APPDATA%\ClaudeCodeUsageBubble\samples.json so a
// restart doesn't reset the observation window. File holds at most a day
// of minute-spaced samples (tiny).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SAMPLES: usize = 1440;
const WINDOW_SECS: u64 = 15 * 60;
const MIN_SPAN_SECS: u64 = 4 * 60;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub unix_secs: u64,
    pub used_pct: f64,
}

fn samples_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| {
        d.join("ClaudeCodeUsageBubble")
            .join("samples.json")
    })
}

fn store() -> &'static Mutex<Vec<Sample>> {
    static S: OnceLock<Mutex<Vec<Sample>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(load()))
}

fn load() -> Vec<Sample> {
    let path = match samples_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let raw: Vec<(u64, f64)> = serde_json::from_str(&text).unwrap_or_default();
    raw.into_iter()
        .filter(|(t, p)| *t > 0 && *p >= 0.0 && *p <= 100.0)
        .map(|(unix_secs, used_pct)| Sample {
            unix_secs,
            used_pct,
        })
        .collect()
}

fn save_locked(samples: &[Sample]) {
    let Some(path) = samples_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let raw: Vec<(u64, f64)> = samples
        .iter()
        .map(|s| (s.unix_secs, s.used_pct))
        .collect();
    if let Ok(json) = serde_json::to_string(&raw) {
        let _ = std::fs::write(&path, json);
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record one observation (call per successful poll). Prunes to the newest
/// MAX_SAMPLES and persists.
pub fn record(used_pct: f64) {
    if !(0.0..=100.0).contains(&used_pct) {
        return;
    }
    let now = now_unix();
    if now == 0 {
        return;
    }
    let mut guard = store().lock().expect("samples mutex poisoned");
    // Avoid stuffing duplicates when polls repeat the same value back to
    // back; keep the newest timestamp only if the value moved or a minute
    // passed since the last sample.
    if let Some(last) = guard.last() {
        if (last.used_pct - used_pct).abs() < f64::EPSILON && now - last.unix_secs < 60 {
            return;
        }
    }
    guard.push(Sample {
        unix_secs: now,
        used_pct,
    });
    if guard.len() > MAX_SAMPLES {
        let excess = guard.len() - MAX_SAMPLES;
        guard.drain(..excess);
    }
    save_locked(&guard);
}

#[derive(Clone, Debug)]
pub enum Pace {
    /// Not enough history yet (< 4 min span or < 2 samples).
    Collecting,
    /// No measurable burn in the window.
    Idle,
    /// (percent_per_hour, hours_until_empty)
    Live {
        per_hour: f64,
        hours_left: f64,
    },
}

/// Project from samples inside the rolling 15-minute window.
pub fn project(remaining_pct: f64) -> Pace {
    let guard = store().lock().expect("samples mutex poisoned");
    let now = now_unix();
    let cutoff = now.saturating_sub(WINDOW_SECS);
    let mut first: Option<&Sample> = None;
    let mut last: Option<&Sample> = None;
    for s in guard.iter() {
        if s.unix_secs < cutoff {
            continue;
        }
        if first.is_none() {
            first = Some(s);
        }
        last = Some(s);
    }
    let (Some(a), Some(b)) = (first, last) else {
        return Pace::Collecting;
    };
    let span = b.unix_secs.saturating_sub(a.unix_secs);
    if span < MIN_SPAN_SECS {
        return Pace::Collecting;
    }
    let burned = b.used_pct - a.used_pct;
    let per_hour = burned / (span as f64 / 3600.0);
    if per_hour <= 0.005 {
        return Pace::Idle;
    }
    Pace::Live {
        per_hour,
        hours_left: remaining_pct / per_hour,
    }
}

/// Format "1,5" style pt-BR decimals.
pub fn fmt_1(n: f64) -> String {
    format!("{n:.1}").replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_collecting_live_idle() {
        // Deterministic: drive the store directly (same process, own lock).
        let now = now_unix();
        {
            let mut guard = store().lock().unwrap();
            guard.clear();
        }
        assert!(matches!(project(100.0), Pace::Collecting));

        // 1% burned over 5 minutes => 12%/h; 90% left => 7.5h.
        {
            let mut guard = store().lock().unwrap();
            guard.clear();
            guard.push(Sample {
                unix_secs: now - 300,
                used_pct: 10.0,
            });
            guard.push(Sample {
                unix_secs: now,
                used_pct: 11.0,
            });
        }
        match project(90.0) {
            Pace::Live {
                per_hour,
                hours_left,
            } => {
                assert!((per_hour - 12.0).abs() < 0.01);
                assert!((hours_left - 7.5).abs() < 0.01);
            }
            other => panic!("expected Live, got {other:?}"),
        }

        // Flat usage => Idle.
        {
            let mut guard = store().lock().unwrap();
            guard.clear();
            guard.push(Sample {
                unix_secs: now - 300,
                used_pct: 10.0,
            });
            guard.push(Sample {
                unix_secs: now,
                used_pct: 10.0,
            });
        }
        assert!(matches!(project(90.0), Pace::Idle));

        // Too short a span => Collecting.
        {
            let mut guard = store().lock().unwrap();
            guard.clear();
            guard.push(Sample {
                unix_secs: now - 60,
                used_pct: 10.0,
            });
            guard.push(Sample {
                unix_secs: now,
                used_pct: 11.0,
            });
        }
        assert!(matches!(project(90.0), Pace::Collecting));

        // Leave the store clean for other tests / real runs.
        store().lock().unwrap().clear();
    }

    #[test]
    fn fmt_pt_decimal() {
        assert_eq!(fmt_1(13.75), "13,8");
        assert_eq!(fmt_1(0.0), "0,0");
    }
}
