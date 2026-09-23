#![windows_subsystem = "windows"]
// Several modules (creds, usage, update, os::dpi, …) expose API that the
// app surface doesn't fully call yet — they're scaffolding for in-progress
// port phases. Allow at the crate root rather than scattering attributes.
#![allow(dead_code)]

// Original infrastructure.
mod codex_tokens;
mod creds;
mod diag;
mod i18n;
mod net;
mod os;
mod tray;
mod update;
mod usage;
mod usage_color;

// Application surface.
mod app;
mod bubble;
mod panel;
mod settings;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let diagnose_enabled = args.iter().any(|a| a == "--diagnose");
    if diagnose_enabled {
        if let Ok(Some(path)) = diag::init(true) {
            log::info!("startup args={args:?} log_path={}", path.display());
        }
    }

    if let Some(exit_code) = update::run_cli(&args) {
        if diagnose_enabled {
            log::info!("cli mode exited with code {exit_code}");
        }
        std::process::exit(exit_code);
    }

    let wait_pid = args
        .iter()
        .position(|a| a == "--wait-pid")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok());
    if let Some(pid) = wait_pid {
        if diagnose_enabled {
            log::info!("waiting up to 5s for parent pid {pid} to exit");
        }
        update::handoff::wait_for_parent_exit(pid, 5_000);
    }

    let updated_to = args
        .iter()
        .position(|a| a == "--updated-to")
        .and_then(|i| args.get(i + 1))
        .cloned();

    if diagnose_enabled {
        log::info!("entering app::run (wait_pid={wait_pid:?} updated_to={updated_to:?})");
    }
    app::run(AppArgs {
        wait_pid_present: wait_pid.is_some(),
        updated_to,
    });
}

pub struct AppArgs {
    pub wait_pid_present: bool,
    pub updated_to: Option<String>,
}
