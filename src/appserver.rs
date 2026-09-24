// Fork (Rafael): Codex app-server client over stdio JSON-RPC.
//
// Same protocol the Tauri dashboard uses: spawn `codex app-server --stdio`,
// handshake with `initialize` + `initialized`, then call
// `account/usage/read` for `{ lifetimeTokens, dailyUsageBuckets[] }`.
// Read-only. Shares this machine's existing Codex auth; never handles,
// logs, or persists tokens.
//
// Runs on its own worker thread with an independent 60s loop; the UI only
// ever reads the last snapshot (never blocks on the child).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    mpsc, Arc, Mutex, OnceLock,
};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const RESP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_SECS: u64 = 60;
/// CREATE_NO_WINDOW so no console window flashes next to the bubble.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Default)]
pub struct UsageSnapshot {
    pub tokens_today: Option<i64>,
    pub bucket_date: Option<String>,
    pub lifetime_tokens: Option<i64>,
}

fn cell() -> &'static Mutex<UsageSnapshot> {
    static C: OnceLock<Mutex<UsageSnapshot>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(UsageSnapshot::default()))
}

/// Last known snapshot (default/empty until the first successful query).
pub fn latest() -> UsageSnapshot {
    cell().lock().expect("appserver snapshot poisoned").clone()
}

/// Fire-and-forget worker. Safe to call once at startup.
pub fn start() {
    std::thread::spawn(worker);
}

fn worker() {
    let client = Client::new();
    loop {
        match cycle(&client) {
            Ok(snap) => {
                *cell().lock().expect("appserver snapshot poisoned") = snap;
            }
            Err(e) => {
                log::warn!("app-server cycle failed: {e}");
                client.reset();
            }
        }
        std::thread::sleep(Duration::from_secs(POLL_SECS));
    }
}

fn cycle(client: &Arc<Client>) -> Result<UsageSnapshot, String> {
    let v = client.request("account/usage/read", None)?;
    Ok(parse_usage(&v))
}

fn as_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|n| n.round() as i64))
        .or_else(|| v.as_str()?.parse().ok())
}

/// Pick the newest daily bucket (by startDate string, YYYY-MM-DD sorts).
fn parse_usage(v: &serde_json::Value) -> UsageSnapshot {
    let lifetime_tokens = v.get("lifetimeTokens").and_then(as_i64);
    let mut best: Option<(String, i64)> = None;
    if let Some(arr) = v.get("dailyUsageBuckets").and_then(|b| b.as_array()) {
        for b in arr {
            let date = b
                .get("startDate")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let tokens = b.get("tokens").and_then(as_i64).unwrap_or(0);
            let newer = best
                .as_ref()
                .map(|(d, _)| date.as_str() > d.as_str())
                .unwrap_or(true);
            if newer {
                best = Some((date, tokens));
            }
        }
    }
    UsageSnapshot {
        tokens_today: best.as_ref().map(|(_, t)| *t),
        bucket_date: best.map(|(d, _)| d),
        lifetime_tokens,
    }
}

fn codex_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for key in ["CODEX_BUBBLE_CODEX_PATH", "CODEX_WIDGET_CODEX_PATH"] {
        if let Ok(p) = std::env::var(key) {
            if !p.trim().is_empty() {
                out.push(PathBuf::from(p));
            }
        }
    }
    // Whatever `codex` resolves to on PATH (npm shim .cmd included).
    if let Ok(res) = Command::new("where.exe").arg("codex").output() {
        for line in String::from_utf8_lossy(&res.stdout).lines() {
            let t = line.trim();
            if !t.is_empty() {
                out.push(PathBuf::from(t));
            }
        }
    }
    // npm global default layout.
    if let Some(home) = dirs::home_dir() {
        out.push(home.join("AppData").join("Roaming").join("npm").join("codex.cmd"));
    }
    out
}

struct Client {
    next_id: AtomicI64,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<HashMap<i64, mpsc::Sender<serde_json::Value>>>,
    child: Mutex<Option<Child>>,
    warned_missing: Mutex<bool>,
}

impl Client {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicI64::new(1),
            stdin: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            warned_missing: Mutex::new(false),
        })
    }

    fn reset(&self) {
        *self.stdin.lock().expect("stdin poisoned") = None;
        self.pending.lock().expect("pending poisoned").clear();
        if let Some(mut child) = self.child.lock().expect("child poisoned").take() {
            let _ = child.kill();
        }
    }

    fn ensure_started(self: &Arc<Self>) -> Result<(), String> {
        if self.stdin.lock().expect("stdin poisoned").is_some() {
            return Ok(());
        }
        let mut failures = Vec::new();
        let mut spawned: Option<Child> = None;
        for exe in codex_candidates() {
            let mut cmd = Command::new(&exe);
            cmd.args(["app-server", "--stdio"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            match cmd.spawn() {
                Ok(c) => {
                    spawned = Some(c);
                    break;
                }
                Err(e) => failures.push(format!("{}: {e}", exe.display())),
            }
        }
        let mut child = spawned.ok_or_else(|| {
            let mut warned = self.warned_missing.lock().expect("warn poisoned");
            let msg = format!("codex app-server nao iniciou. Tentativas: {}", failures.join("; "));
            if !*warned {
                log::warn!("{msg}");
                *warned = true;
            }
            msg
        })?;
        let stdin = child.stdin.take().ok_or("app-server sem stdin")?;
        let stdout = child.stdout.take().ok_or("app-server sem stdout")?;
        let stderr = child.stderr.take().ok_or("app-server sem stderr")?;
        *self.stdin.lock().expect("stdin poisoned") = Some(stdin);
        *self.child.lock().expect("child poisoned") = Some(child);

        // Stdout router: responses (by id) vs notifications (ignored).
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(text) = line else { break };
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
                    if let Some(tx) = me.pending.lock().expect("pending poisoned").remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
            }
            me.reset();
        });
        // Stderr drain (never logged: may carry account-adjacent output).
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if line.is_err() {
                    break;
                }
            }
        });

        // Handshake.
        let version = env!("CARGO_PKG_VERSION");
        self.request(
            "initialize",
            Some(serde_json::json!({
                "clientInfo": {
                    "name": "codex_usage_bubble",
                    "title": "Codex Usage Bubble",
                    "version": version,
                }
            })),
        )?;
        self.notify("initialized", None)?;
        Ok(())
    }

    fn write_message(self: &Arc<Self>, msg: &serde_json::Value) -> Result<(), String> {
        let mut guard = self.stdin.lock().expect("stdin poisoned");
        let stdin = guard.as_mut().ok_or("app-server desconectado")?;
        let mut bytes = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())
    }

    fn notify(self: &Arc<Self>, method: &str, params: Option<serde_json::Value>) -> Result<(), String> {
        let mut msg = serde_json::json!({ "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        self.write_message(&msg)
    }

    fn request(
        self: &Arc<Self>,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        self.ensure_started()?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut msg = serde_json::json!({ "method": method, "id": id });
        if let Some(p) = params {
            msg["params"] = p;
        }
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .expect("pending poisoned")
            .insert(id, tx);
        if let Err(e) = self.write_message(&msg) {
            self.pending
                .lock()
                .expect("pending poisoned")
                .remove(&id);
            self.reset();
            return Err(e);
        }
        let resp = rx.recv_timeout(RESP_TIMEOUT).map_err(|_| {
            self.reset();
            "app-server nao respondeu em 15s".to_string()
        })?;
        if let Some(err) = resp.get("error") {
            let text = err.to_string().to_ascii_lowercase();
            if method.starts_with("account/")
                && (text.contains("unauthor")
                    || text.contains("authentication")
                    || text.contains("token"))
            {
                self.reset();
            }
            return Err(format!("app-server: {err}"));
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| "app-server: resposta vazia".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_newest_bucket() {
        let v: serde_json::Value = serde_json::json!({
            "lifetimeTokens": 999,
            "dailyUsageBuckets": [
                {"startDate": "2026-09-22", "tokens": 100},
                {"startDate": "2026-09-24", "tokens": 55300000},
                {"startDate": "2026-09-23", "tokens": 200}
            ]
        });
        let s = parse_usage(&v);
        assert_eq!(s.tokens_today, Some(55300000));
        assert_eq!(s.bucket_date.as_deref(), Some("2026-09-24"));
        assert_eq!(s.lifetime_tokens, Some(999));
    }

    #[test]
    fn empty_usage_is_none() {
        let v: serde_json::Value = serde_json::json!({});
        let s = parse_usage(&v);
        assert_eq!(s.tokens_today, None);
        assert_eq!(s.lifetime_tokens, None);
    }

    #[test]
    fn as_i64_accepts_shapes() {
        assert_eq!(as_i64(&serde_json::json!(5)), Some(5));
        assert_eq!(as_i64(&serde_json::json!("42")), Some(42));
        assert_eq!(as_i64(&serde_json::json!(7.6)), Some(8));
        assert_eq!(as_i64(&serde_json::json!(true)), None);
    }
}
