// Native Win32 process + file handoff primitives used by the in-app
// restart path and the auto-update install path. The main binary uses
// `windows_subsystem = "windows"`, so spawning the child directly via
// `CreateProcessW` allocates no console — nothing can flash.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, FALSE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateProcessW, OpenProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_INFORMATION, PROCESS_SYNCHRONIZE,
    STARTUPINFOW,
};

/// Spawn `exe` with the supplied args as a detached, console-less child.
///
/// Caller is fire-and-forget: the child's handles are closed immediately
/// so no zombie wait is required.
pub fn spawn_detached(exe: &Path, args: &[OsString]) -> io::Result<()> {
    let mut cmdline = build_command_line(exe, args);
    let exe_w: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();

    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    let flags = CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    let ok = unsafe {
        CreateProcessW(
            PCWSTR::from_raw(exe_w.as_ptr()),
            windows::core::PWSTR(cmdline.as_mut_ptr()),
            None,
            None,
            FALSE,
            flags,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
    };

    if ok.is_err() {
        return Err(io::Error::last_os_error());
    }

    unsafe {
        if !pi.hThread.is_invalid() {
            let _ = CloseHandle(pi.hThread);
        }
        if !pi.hProcess.is_invalid() {
            let _ = CloseHandle(pi.hProcess);
        }
    }
    // Suppress the unused-variable warning until si.lpReserved fields ever matter.
    let _ = &si;
    Ok(())
}

/// Wait up to `timeout_ms` for `pid` to exit. Silent on any failure —
/// caller treats this as a best-effort barrier before acquiring the
/// singleton mutex.
pub fn wait_for_parent_exit(pid: u32, timeout_ms: u32) {
    let handle: HANDLE = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, FALSE, pid) } {
        Ok(h) if !h.is_invalid() => h,
        _ => return,
    };
    unsafe {
        let res = WaitForSingleObject(handle, timeout_ms);
        if res != WAIT_OBJECT_0 {
            log::debug!("wait_for_parent_exit pid={pid} timeout/err res={:?}", res.0);
        }
        let _ = CloseHandle(handle);
    }
}

/// Remove leftover `<exe>.old.<pid>` siblings from previous in-place updates.
/// Filled in by phase 4; stubbed here so phase 1 can wire the call sites.
pub fn cleanup_stale_old_exes(current_exe: &Path) {
    let Some(dir) = current_exe.parent() else {
        return;
    };
    let Some(stem) = current_exe.file_name() else {
        return;
    };
    let prefix = format!("{}.old.", stem.to_string_lossy());
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                log::debug!(
                    "cleanup_stale_old_exes: remove {:?} failed: {e}",
                    entry.path()
                );
            }
        }
    }
}

fn build_command_line(exe: &Path, args: &[OsString]) -> Vec<u16> {
    // CreateProcessW parses argv[0] from a quoted exe path. We wrap the
    // exe in `"…"` and join args separated by spaces. Args are quoted
    // only when they contain whitespace; our callers pass simple tokens
    // (--wait-pid <number>, --updated-to <version>) so naive quoting is
    // sufficient.
    let mut line = String::new();
    line.push('"');
    line.push_str(&exe.to_string_lossy());
    line.push('"');
    for a in args {
        line.push(' ');
        let s = a.to_string_lossy();
        if s.chars().any(|c| c.is_whitespace()) {
            line.push('"');
            line.push_str(&s);
            line.push('"');
        } else {
            line.push_str(&s);
        }
    }
    let mut wide: Vec<u16> = std::ffi::OsString::from(line).encode_wide().collect();
    wide.push(0);
    wide
}
