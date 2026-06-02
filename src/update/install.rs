// Download a release asset and swap it in via a detached helper process.
//
// The running process cannot reliably replace its own mapped image on
// Windows. Instead, it writes the new .exe to a staging path, copies the
// current executable to a helper path, starts that helper with
// `--apply-update`, and then exits. The helper waits for the parent process
// to exit before replacing the install target with the staged new binary.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_COPY_ALLOWED, MOVEFILE_REPLACE_EXISTING, MOVE_FILE_FLAGS,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use crate::net::Client;
use crate::os::to_utf16_nul;

pub fn begin(http: &Client, release: &super::Release) -> Result<(), super::Error> {
    let current = std::env::current_exe()?;
    ensure_writable(&current)?;
    let staging = stage_path()?;
    // Defense in depth: `MoveFileExW` itself is immune to `%`-expansion
    // (no shell parses our paths), but the existing rejection guards
    // future code paths that might invoke external tools, so keep it.
    reject_unsafe_path(&current)?;
    reject_unsafe_path(&staging)?;
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)?;
    }
    download(
        http,
        &release.asset_url,
        &staging,
        release.asset_sha256.as_ref(),
    )?;
    let helper = helper_path()?;
    reject_unsafe_path(&helper)?;
    prepare_update_helper(&current, &helper)?;
    spawn_update_helper(&helper, &staging, &current, &release.version)?;
    Ok(())
}

/// CLI entry point for `--apply-update <target> <source> <parent-pid> <version>`.
/// Runs from the staged new binary, waits for the old UI process to exit,
/// replaces the installed exe, and starts the installed copy.
pub fn run_cli(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("--apply-update") {
        return None;
    }
    let Some(target) = args.get(2).map(PathBuf::from) else {
        return Some(2);
    };
    let Some(source) = args.get(3).map(PathBuf::from) else {
        return Some(2);
    };
    let Some(parent_pid) = args.get(4).and_then(|s| s.parse::<u32>().ok()) else {
        return Some(2);
    };
    let Some(version) = args.get(5).cloned() else {
        return Some(2);
    };

    super::handoff::wait_for_parent_exit(parent_pid, 15_000);
    match replace_from_helper(&source, &target, &version) {
        Ok(()) => Some(0),
        Err(e) => {
            log::error!("apply-update failed: {e}");
            Some(1)
        }
    }
}

fn download(
    http: &Client,
    url: &str,
    to: &Path,
    expected_sha256: Option<&[u8; 32]>,
) -> Result<(), super::Error> {
    let resp = http
        .get(url)
        .header("User-Agent", super::release::user_agent())
        .send()?;
    if !(200..300).contains(&resp.status()) {
        return Err(super::Error::Network(crate::net::Error::Status(
            resp.status(),
        )));
    }
    let body = resp.body();
    if let Some(expected) = expected_sha256 {
        let mut hasher = Sha256::new();
        hasher.update(body);
        let actual = hasher.finalize();
        if actual.as_slice() != expected {
            return Err(super::Error::ChecksumMismatch {
                expected: hex_encode(expected),
                actual: hex_encode(&actual),
            });
        }
    }
    std::fs::write(to, body)?;
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn reject_unsafe_path(p: &Path) -> Result<(), super::Error> {
    let s = p.to_string_lossy();
    if s.contains('%') {
        return Err(super::Error::UnsafePath(format!("path contains '%': {s}")));
    }
    Ok(())
}

fn spawn_update_helper(
    helper: &Path,
    staging: &Path,
    target: &Path,
    version: &super::release::Version,
) -> Result<(), super::Error> {
    let pid = unsafe { GetCurrentProcessId() };
    let version_str = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let args = vec![
        OsString::from("--apply-update"),
        target.as_os_str().to_os_string(),
        staging.as_os_str().to_os_string(),
        OsString::from(pid.to_string()),
        OsString::from(version_str),
    ];
    super::handoff::spawn_detached(helper, &args).map_err(super::Error::Io)
}

fn replace_from_helper(source: &Path, target: &Path, version: &str) -> Result<(), super::Error> {
    let backup = backup_path(target);
    // Parent has exited, so the install target is no longer mapped.
    move_file(target, &backup, MOVE_FILE_FLAGS(0))?;

    let swap_flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED;
    if let Err(swap_err) = move_file(source, target, swap_flags) {
        // Compatibility for users updating from a release that invoked
        // the downloaded binary itself as the helper. A mapped source exe
        // may not be movable, but it can usually still be copied.
        let copy_result = std::fs::copy(source, target);
        if copy_result.is_err() {
            log::error!("source move failed before copy fallback: {swap_err}");
        }
        if let Err(copy_err) = copy_result {
            if let Err(revert_err) = move_file(&backup, target, MOVEFILE_REPLACE_EXISTING) {
                log::error!("rollback also failed: {revert_err}; surfacing modal");
                let target_name = target
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "claude-code-usage-bubble.exe".to_string());
                surface_rollback_failure(&backup, &target_name);
            }
            return Err(super::Error::Io(copy_err));
        }
    }

    let args = vec![OsString::from("--updated-to"), OsString::from(version)];
    if let Err(spawn_err) = super::handoff::spawn_detached(target, &args) {
        log::error!("spawn_detached failed after swap: {spawn_err}; attempting revert");
        let _ = std::fs::remove_file(target);
        if let Err(revert_err) = move_file(&backup, target, MOVEFILE_REPLACE_EXISTING) {
            log::error!("post-spawn revert failed: {revert_err}");
        }
        return Err(super::Error::Io(spawn_err));
    }

    let _ = std::fs::remove_file(source);

    Ok(())
}

fn move_file(src: &Path, dst: &Path, flags: MOVE_FILE_FLAGS) -> Result<(), super::Error> {
    let src_w = to_utf16_nul(&src.to_string_lossy());
    let dst_w = to_utf16_nul(&dst.to_string_lossy());
    let result = unsafe {
        MoveFileExW(
            PCWSTR::from_raw(src_w.as_ptr()),
            PCWSTR::from_raw(dst_w.as_ptr()),
            flags,
        )
    };
    result.map_err(|e| {
        super::Error::SwapFailed(format!(
            "MoveFileExW({} -> {}): {e}",
            src.display(),
            dst.display()
        ))
    })
}

fn backup_path(target: &Path) -> PathBuf {
    let pid = unsafe { GetCurrentProcessId() };
    let fname = target
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "exe".to_string());
    let mut p = target.to_owned();
    p.set_file_name(format!("{fname}.old.{pid}"));
    p
}

fn surface_rollback_failure(backup: &Path, target_name: &str) {
    // Pull the localized body from i18n; the caller passes the
    // user-meaningful filename so we can format it in-place.
    let strings = crate::i18n::I18n::load(None).strings().clone();
    let body = format!(
        "{}{}\n\n{}",
        strings.update_rollback_failed_body,
        backup.display(),
        target_name
    );
    let title_w = to_utf16_nul(&strings.update_failed);
    let body_w = to_utf16_nul(&body);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR::from_raw(body_w.as_ptr()),
            PCWSTR::from_raw(title_w.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn stage_path() -> Result<PathBuf, super::Error> {
    let base = dirs::data_local_dir().ok_or_else(|| {
        super::Error::NotWritable("no local data directory available".to_string())
    })?;
    Ok(base
        .join("ClaudeCodeUsageBubble")
        .join("updates")
        .join("update.exe"))
}

fn helper_path() -> Result<PathBuf, super::Error> {
    let base = dirs::data_local_dir().ok_or_else(|| {
        super::Error::NotWritable("no local data directory available".to_string())
    })?;
    let pid = unsafe { GetCurrentProcessId() };
    Ok(base
        .join("ClaudeCodeUsageBubble")
        .join("updates")
        .join(format!("updater-helper-{pid}.exe")))
}

fn prepare_update_helper(current: &Path, helper: &Path) -> Result<(), super::Error> {
    if let Some(parent) = helper.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(current, helper)?;
    Ok(())
}

pub fn cleanup_staged_update_files() {
    let Ok(stage) = stage_path() else {
        return;
    };
    let Some(dir) = stage.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "update.exe" || (name.starts_with("updater-helper-") && name.ends_with(".exe")) {
            if let Err(e) = std::fs::remove_file(&path) {
                log::debug!("cleanup_staged_update_files: remove {:?} failed: {e}", path);
            }
        }
    }
}

fn ensure_writable(target: &Path) -> Result<(), super::Error> {
    let parent = target.parent().ok_or_else(|| {
        super::Error::NotWritable("could not resolve install directory".to_string())
    })?;
    let probe = parent.join(".__bubble_update_probe");
    std::fs::write(&probe, b"").map_err(|e| super::Error::NotWritable(e.to_string()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_and_helper_paths_are_distinct_exes() {
        let stage = stage_path().expect("stage path");
        let helper = helper_path().expect("helper path");

        assert_eq!(stage.file_name().unwrap(), "update.exe");
        assert!(helper
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("updater-helper-"));
        assert_ne!(stage, helper);
    }
}
