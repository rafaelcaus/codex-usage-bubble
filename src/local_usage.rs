//! Read-only, incremental token activity from the session records on this PC.
//! Counts cumulative deltas, not repeated last-token notifications. Disk I/O
//! stays on a worker; the UI reads only its last complete snapshot.
use chrono::{Local, NaiveDate, TimeZone};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Default)]
pub struct Snapshot {
    pub day: Option<NaiveDate>,
    pub tokens: Option<u64>,
    at: Option<Instant>,
}
fn cell() -> &'static Mutex<Snapshot> {
    static CELL: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Snapshot::default()))
}
pub fn latest() -> Snapshot {
    let s = cell()
        .lock()
        .expect("local usage snapshot poisoned")
        .clone();
    if s.day != Some(Local::now().date_naive())
        || s.at.is_none_or(|at| at.elapsed() > Duration::from_secs(30))
    {
        Snapshot::default()
    } else {
        s
    }
}
pub fn start() {
    std::thread::spawn(|| {
        let root = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".codex")));
        let mut counter = Counter::default();
        loop {
            let day = Local::now().date_naive();
            let tokens = root.as_ref().and_then(|p| counter.poll(p, day).ok());
            *cell().lock().expect("local usage snapshot poisoned") = Snapshot {
                day: Some(day),
                tokens,
                at: Some(Instant::now()),
            };
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

/// Support probe: no UI, authentication, message contents, or settings writes.
pub fn diagnose() -> std::io::Result<()> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
        .ok_or_else(|| std::io::Error::other("Codex home unavailable"))?;
    let mut counter = Counter::default();
    for sample in 0..4 {
        let started = Instant::now();
        let day = Local::now().date_naive();
        let tokens = counter.poll(&root, day)?;
        println!(
            "day={day} local_tokens={tokens} scan_ms={}",
            started.elapsed().as_millis()
        );
        if sample < 3 {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Accumulator {
    high_water: Option<u64>,
    sum: u64,
    session_id: Option<String>,
    created_at: Option<i64>,
}
impl Accumulator {
    fn ingest(&mut self, v: &serde_json::Value, start: i64, end: i64) {
        let timestamp = v
            .get("timestamp")
            .and_then(|s| s.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp());
        if v.get("type").and_then(|s| s.as_str()) == Some("session_meta") {
            self.session_id = v
                .pointer("/payload/id")
                .and_then(|s| s.as_str())
                .map(str::to_owned);
            self.created_at = timestamp;
            return;
        }
        if v.get("type").and_then(|s| s.as_str()) != Some("event_msg")
            || v.pointer("/payload/type").and_then(|s| s.as_str()) != Some("token_count")
        {
            return;
        }
        let Some(ts) = timestamp else { return };
        // A fork can carry copied history from before its own creation.
        if self.created_at.is_some_and(|created| ts < created) {
            return;
        }
        let Some(total) = v
            .pointer("/payload/info/total_token_usage/total_tokens")
            .and_then(|n| n.as_u64())
        else {
            return;
        };
        let delta = match self.high_water {
            Some(previous) => total.saturating_sub(previous),
            // A partial/resumed log may start with lifetime history. Only
            // its last recorded request is attributable without a baseline.
            None => v
                .pointer("/payload/info/last_token_usage/total_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0)
                .min(total),
        };
        self.high_water = Some(self.high_water.unwrap_or(0).max(total));
        if ts >= start && ts < end {
            self.sum = self.sum.saturating_add(delta);
        }
    }
}
#[derive(Default)]
struct FileState {
    offset: u64,
    modified: Option<SystemTime>,
    len: u64,
    count: Accumulator,
}
impl FileState {
    fn scan(&mut self, path: &Path, start: i64, end: i64) -> std::io::Result<()> {
        let meta = fs::metadata(path)?;
        let modified = meta.modified().ok();
        if meta.len() == self.len && modified == self.modified {
            return Ok(());
        }
        if meta.len() < self.len || (meta.len() == self.len && modified != self.modified) {
            *self = Self::default();
        }
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 || !line.ends_with('\n') {
                break;
            }
            self.offset += n as u64;
            // Most bytes are message/tool contents. Never parse those.
            if !(line.contains("\"session_meta\"") || line.contains("\"token_count\"")) {
                continue;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                self.count.ingest(&v, start, end);
            }
        }
        self.len = meta.len();
        self.modified = modified;
        Ok(())
    }
}
#[derive(Default)]
struct Counter {
    day: Option<NaiveDate>,
    files: HashMap<PathBuf, FileState>,
}
impl Counter {
    fn poll(&mut self, root: &Path, day: NaiveDate) -> std::io::Result<u64> {
        let midnight = |d: NaiveDate| {
            Local
                .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .map(|d| d.timestamp())
        };
        let (Some(start), Some(end)) = (midnight(day), day.succ_opt().and_then(midnight)) else {
            return Err(std::io::Error::other("unavailable local day boundary"));
        };
        if self.day != Some(day) {
            self.files.clear();
            self.day = Some(day);
        }
        let mut paths = Vec::new();
        collect(&root.join("sessions"), &mut paths, 0)?;
        let archived = root.join("archived_sessions");
        if archived.exists() {
            collect(&archived, &mut paths, 0)?;
        }
        self.files.retain(|path, _| paths.contains(path));
        let mut sessions: HashMap<String, u64> = HashMap::new();
        for path in paths {
            let state = self.files.entry(path.clone()).or_default();
            state.scan(&path, start, end)?;
            let key = state
                .count
                .session_id
                .clone()
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            // Moving/copying an archived session must not count it twice.
            sessions
                .entry(key)
                .and_modify(|sum| *sum = (*sum).max(state.count.sum))
                .or_insert(state.count.sum);
        }
        Ok(sessions
            .values()
            .fold(0_u64, |sum, n| sum.saturating_add(*n)))
    }
}
fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) -> std::io::Result<()> {
    if depth > 6 {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, out, depth + 1)?;
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(ts: i64, total: u64, last: u64) -> serde_json::Value {
        serde_json::json!({"timestamp":chrono::DateTime::from_timestamp(ts,0).unwrap().to_rfc3339(),
            "type":"event_msg","payload":{"type":"token_count","info":{
                "total_token_usage":{"total_tokens":total},"last_token_usage":{"total_tokens":last}}}})
    }
    #[test]
    fn counts_deltas_once_and_excludes_previous_day() {
        let mut a = Accumulator::default();
        a.ingest(&event(99, 1000, 1000), 100, 200);
        a.ingest(&event(101, 1050, 50), 100, 200);
        a.ingest(&event(102, 1050, 50), 100, 200);
        a.ingest(&event(103, 1100, 50), 100, 200);
        a.ingest(&event(104, 1080, 30), 100, 200);
        a.ingest(&event(105, 1120, 20), 100, 200);
        a.ingest(&event(200, 1200, 80), 100, 200);
        assert_eq!(a.sum, 120);
    }
    #[test]
    fn first_record_does_not_import_lifetime_and_fork_ignores_copied_history() {
        let mut a = Accumulator::default();
        a.ingest(&event(101, 10000, 10), 100, 200);
        assert_eq!(a.sum, 10);
        let mut fork = Accumulator {
            created_at: Some(110),
            ..Default::default()
        };
        fork.ingest(&event(105, 10000, 900), 100, 200);
        fork.ingest(&event(111, 10020, 20), 100, 200);
        assert_eq!(fork.sum, 20);
    }
    #[test]
    fn counter_deduplicates_archives_and_invalidates_day_cache() {
        let root = std::env::temp_dir().join(format!("widget-day-boundary-{}", std::process::id()));
        let sessions = root.join("sessions");
        let archives = root.join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&archives).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let start = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp();
        let next = Local
            .from_local_datetime(&day.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp();
        let header =
            serde_json::json!({"type":"session_meta","payload":{"id":"synthetic-session"}});
        let log = format!(
            "{}\n{}\n{}\n{}\n",
            header,
            event(start - 1, 100, 100),
            event(start + 1, 130, 30),
            event(next + 1, 140, 10)
        );
        let a = sessions.join("a.jsonl");
        let b = archives.join("b.jsonl");
        fs::write(&a, &log).unwrap();
        fs::write(&b, &log).unwrap();
        let mut counter = Counter::default();
        assert_eq!(counter.poll(&root, day).unwrap(), 30);
        assert_eq!(counter.poll(&root, day.succ_opt().unwrap()).unwrap(), 10);
        fs::remove_file(a).unwrap();
        fs::remove_file(b).unwrap();
        fs::remove_dir(sessions).unwrap();
        fs::remove_dir(archives).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn appended_and_partial_records_are_read_without_recounting() {
        use std::io::Write;
        let path =
            std::env::temp_dir().join(format!("widget-local-usage-{}.jsonl", std::process::id()));
        let mut file = File::create(&path).unwrap();
        writeln!(file, "{}", event(99, 100, 100)).unwrap();
        write!(file, "{}", event(101, 120, 20)).unwrap();
        file.flush().unwrap();
        let mut state = FileState::default();
        state.scan(&path, 100, 200).unwrap();
        assert_eq!(state.count.sum, 0);
        writeln!(file).unwrap();
        file.flush().unwrap();
        state.scan(&path, 100, 200).unwrap();
        assert_eq!(state.count.sum, 20);
        state.scan(&path, 100, 200).unwrap();
        assert_eq!(state.count.sum, 20);
        writeln!(file, "{}", event(102, 140, 20)).unwrap();
        file.flush().unwrap();
        state.scan(&path, 100, 200).unwrap();
        assert_eq!(state.count.sum, 40);
        drop(file);
        fs::remove_file(path).unwrap();
    }
}
