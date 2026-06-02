// Token refresh orchestrator.
//
// Each provider's credential source advertises a `RefreshHint` describing
// which CLI to spawn. We invoke that CLI and watch the credential file's
// signature; if it changes within the timeout we declare success.

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::creds::{CredentialSource, RefreshHint};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Refreshed,
    StillExpired,
    CliMissing,
    Timeout,
}

pub struct Orchestrator {
    timeout: Duration,
    poll_interval: Duration,
}

impl Orchestrator {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            poll_interval: Duration::from_millis(500),
        }
    }

    pub fn refresh(&self, source: &dyn CredentialSource) -> Outcome {
        let initial_sig = source.signature();
        let hint = source.refresh_hint();
        if !spawn_cli(&hint) {
            return Outcome::CliMissing;
        }
        let start = Instant::now();
        while start.elapsed() < self.timeout {
            std::thread::sleep(self.poll_interval);
            if source.signature() != initial_sig {
                return Outcome::Refreshed;
            }
        }
        if source.signature() != initial_sig {
            Outcome::Refreshed
        } else {
            Outcome::Timeout
        }
    }
}

fn spawn_cli(hint: &RefreshHint) -> bool {
    match hint {
        RefreshHint::LocalClaudeCli => {
            spawn_local(&["claude.cmd", "claude.exe", "claude"], &["-p", "."])
        }
        RefreshHint::LocalCodexCli => spawn_local(
            &["codex.cmd", "codex.ps1", "codex.exe", "codex"],
            &["exec", "."],
        ),
    }
}

fn spawn_local(candidates: &[&str], args: &[&str]) -> bool {
    for name in candidates {
        let lower = name.to_ascii_lowercase();
        let mut cmd = if lower.ends_with(".ps1") {
            let mut c = Command::new("powershell.exe");
            c.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(name);
            for a in args {
                c.arg(a);
            }
            c
        } else if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let mut c = Command::new("cmd.exe");
            c.arg("/c").arg(name);
            for a in args {
                c.arg(a);
            }
            c
        } else {
            let mut c = Command::new(name);
            for a in args {
                c.arg(a);
            }
            c
        };
        cmd.env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if cmd.spawn().is_ok() {
            return true;
        }
    }
    false
}
