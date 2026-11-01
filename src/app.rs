// App orchestrator.
//
// One `Mutex<Option<AppState>>` is the single source of truth. The UI
// thread runs the message loop; a background thread polls the provider
// registry and posts `WM_APP_USAGE_UPDATED` back via a hidden
// message-only window owned by this module.

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId, Sleep};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::bubble;
use crate::i18n::{self, I18n, LocaleStrings};
// Win32 message + timer IDs moved inline; see constants below.
use crate::net;
use crate::os;
use crate::panel::{self, PanelData};
use crate::settings::{self, Settings, POLL_15_MIN, POLL_1_HOUR, POLL_1_MIN, POLL_5_MIN};
use crate::tray::{self, TrayAction, TrayIcon as TrayIconData, WM_APP_TRAY};
use crate::update::{self, Channel as InstallChannel, CheckOutcome};
use crate::usage::{self, ProviderId, Registry, UsageWindows};

// Win32 message IDs owned by this module.
pub const WM_APP_USAGE_UPDATED: u32 = 0x8001;
// Posted from the update worker thread after `install::begin` has
// already swapped the binary on disk and spawned the new detached
// child. The UI thread responds with PostQuitMessage(0) so the old
// instance exits cleanly and releases the singleton mutex for the
// child waiting on `--wait-pid`.
pub const WM_APP_UPDATE_APPLIED: u32 = 0x8002;

// Timer IDs used with `SetTimer(msg_hwnd, …)`.
const TIMER_POLL: usize = 1;
const TIMER_COUNTDOWN: usize = 2;
const TIMER_RESET_POLL: usize = 3;
const TIMER_UPDATE_CHECK: usize = 4;
const TIMER_LOCAL_USAGE: usize = 5;

const APP_MUTEX_NAME: &str = r"Global\ClaudeCodeUsageBubble";
const STARTUP_REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_VALUE_NAME: &str = "ClaudeCodeUsageBubble";
const APP_CLASS_NAME: &str = "ClaudeCodeUsageBubbleApp";
const HTTP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
// The auto-check interval is user-configurable (see
// `Settings::update_check_interval_secs`). `None` means auto-check is disabled.
const BALLOON_COOLDOWN: Duration = Duration::from_secs(30 * 60);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(8);

// ---------- Menu IDs ----------

const IDM_REFRESH: u16 = 1;
const IDM_EXIT: u16 = 2;
const IDM_FREQ_1MIN: u16 = 10;
const IDM_FREQ_5MIN: u16 = 11;
const IDM_FREQ_15MIN: u16 = 12;
const IDM_FREQ_1HOUR: u16 = 13;
const IDM_MODEL_CLAUDE: u16 = 20;
const IDM_MODEL_CHATGPT: u16 = 21;
const IDM_MODEL_OPENCODE_GO: u16 = 22;
const IDM_START_WITH_WINDOWS: u16 = 30;
const IDM_RESET_POSITION: u16 = 31;
const IDM_VERSION_ACTION: u16 = 32;
const IDM_HELP: u16 = 39;
const IDM_COMPACT: u16 = 40;
const IDM_DETAILS: u16 = 41;
const IDM_RESTART: u16 = 33;
const IDM_SIZE_SMALLER: u16 = 34;
const IDM_SIZE_LARGER: u16 = 35;
const IDM_RESET_SIZE: u16 = 36;
// Fork (Rafael): focus-follow toggle — bubbles only over the ChatGPT app.
const IDM_ONLY_CHATGPT: u16 = 37;
// 50 is reserved by tray::IDM_TOGGLE_WIDGET — keep the auto-update range
// clear of it (and any future tray ids in the 5x band).
const IDM_UPDATE_AUTO_OFF: u16 = 60;
const IDM_UPDATE_AUTO_HOURLY: u16 = 61;
const IDM_UPDATE_AUTO_DAILY: u16 = 62;
const IDM_UPDATE_AUTO_WEEKLY: u16 = 63;

// ---------- State ----------

#[derive(Clone, Copy, Default)]
struct SendHwnd(isize);
unsafe impl Send for SendHwnd {}
impl SendHwnd {
    fn from_hwnd(h: HWND) -> Self {
        Self(h.0 as isize)
    }
    fn to_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Applying,
    Failed,
}

struct AppState {
    msg_hwnd: SendHwnd,
    bubbles: HashMap<ProviderId, SendHwnd>,
    settings: Settings,
    i18n: I18n,
    is_dark: bool,
    install_channel: InstallChannel,
    // http and registry live behind Arc so worker threads can hold them
    // without keeping the global state mutex locked across blocking I/O.
    // The registry's own mutex is only contended by other workers
    // (the in-flight gate ensures at most one poll runs at a time), so
    // the UI thread never waits on it.
    http: Arc<net::Client>,
    registry: Arc<Mutex<Registry>>,
    snapshots: HashMap<ProviderId, ProviderUiState>,
    last_poll_ok: bool,
    update_status: UpdateStatus,
    last_balloon_at: Option<Instant>,
}

#[derive(Clone, Default)]
struct ProviderUiState {
    windows: UsageWindows,
    primary_text: String,
    secondary_text: String,
    /// Fork: "Falta para resetar: 6d 3h" (weekly window, precise).
    reset_text: String,
    /// Completed seven-day account average, including its date range.
    tokens_text: String,
    /// Fork: daily budget "%/dia" from exact remaining time.
    budget_text: String,
    /// Fork: burn-rate projection line.
    pace_text: String,
    /// Fork: tokens since local midnight.
    tokens_today_text: String,
    /// Exact local token count, so every recorded increase is visible.
    today_micro: String,
    /// Local date and explicit machine scope for the bubble.
    today_title: String,
    average_micro: String,
    credits_title:String,
    credits_value:String,
    credits_count_text:String,
    credits_expiry_text:String,
}

fn state() -> &'static Mutex<Option<AppState>> {
    static S: OnceLock<Mutex<Option<AppState>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn lock_state() -> MutexGuard<'static, Option<AppState>> {
    state().lock().expect("app state mutex poisoned")
}

/// Run `f` with mutable access to `AppState`, then snapshot `Settings` and
/// persist it to disk. The lock is released before the disk write so the UI
/// thread doesn't block on I/O. No-op if state hasn't been initialised yet.
fn update_settings(f: impl FnOnce(&mut AppState)) {
    let snap = {
        let mut guard = lock_state();
        let Some(s) = guard.as_mut() else {
            return;
        };
        f(s);
        s.settings.clone()
    };
    settings::save(&snap);
}

// ---------- Entry ----------

/// Acquire the singleton mutex, optionally retrying for ~3s if the
/// caller passed `--wait-pid` (i.e. we just spawned from an exiting
/// parent that has not yet released its handle).
fn acquire_singleton_mutex(retry: bool) -> Option<HANDLE> {
    let mutex_name_w = os::to_utf16_nul(APP_MUTEX_NAME);
    let max_attempts = if retry { 15 } else { 1 };
    for attempt in 0..max_attempts {
        let handle = unsafe { CreateMutexW(None, false, PCWSTR::from_raw(mutex_name_w.as_ptr())) };
        match handle {
            Ok(h) => {
                let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
                if !already {
                    return Some(h);
                }
                // Mutex still held by parent. Close this handle and retry.
                unsafe {
                    let _ = CloseHandle(h);
                }
                if attempt + 1 == max_attempts {
                    log::info!("another instance already running; exiting");
                    return None;
                }
                log::debug!(
                    "mutex still held by parent (attempt {}/{}), waiting 200ms",
                    attempt + 1,
                    max_attempts
                );
                unsafe { Sleep(200) };
            }
            Err(e) => {
                log::error!("CreateMutex failed: {e}");
                return None;
            }
        }
    }
    None
}

pub fn run(args: crate::AppArgs) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let _mutex = match acquire_singleton_mutex(args.wait_pid_present) {
        Some(h) => h,
        None => return,
    };

    let settings = settings::load();
    let i18n = I18n::load();
    let is_dark = os::theme::is_dark();
    let install_channel = update::current_channel();
    let http = match net::Client::new(HTTP_USER_AGENT) {
        Ok(c) => c,
        Err(e) => {
            log::error!("HTTP client init failed: {e}");
            return;
        }
    };
    let msg_hwnd = match create_message_window() {
        Some(h) => h,
        None => {
            log::error!("failed to create app message window");
            return;
        }
    };

    *lock_state() = Some(AppState {
        msg_hwnd: SendHwnd::from_hwnd(msg_hwnd),
        bubbles: HashMap::new(),
        settings,
        i18n,
        is_dark,
        install_channel,
        http: Arc::new(http),
        registry: Arc::new(Mutex::new(Registry::with_defaults())),
        snapshots: HashMap::new(),
        last_poll_ok: false,
        update_status: UpdateStatus::Idle,
        last_balloon_at: None,
    });

    bubble::install_callbacks(bubble::Callbacks {
        on_click: on_bubble_click,
        on_right_click: on_bubble_right_click,
        on_moved: on_bubble_moved,
        on_resized: on_bubble_resized,
        on_menu_command,
        on_settings_changed: recheck_theme,
    });

    create_initial_bubbles();
    // Fork: account data worker (daily tokens via app-server). Independent
    // thread with its own 60s loop; the UI only reads its last snapshot.
    crate::appserver::start();
    crate::local_usage::start();
    unsafe { SetTimer(msg_hwnd, TIMER_LOCAL_USAGE, 5_000, None); }
    // Fork (Rafael): push the persisted focus-follow flag into the bubble
    // module so the 0.35s foreground check enforces it from the first tick.
    bubble::set_only_over_chatgpt(
        lock_state()
            .as_ref()
            .map(|s| s.settings.only_over_chatgpt)
            .unwrap_or(true),
    );
    refresh_tray_icons();

    // Post-update tasks: show "Updated to vX.Y.Z" balloon (driven by
    // --updated-to passed by the previous instance) and sweep any
    // stale `<exe>.old.<pid>` siblings left by past in-place swaps.
    if let Some(v) = args.updated_to.as_ref() {
        announce_update_applied(msg_hwnd, v);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        update::handoff::cleanup_stale_old_exes(&exe_path);
    }
    update::install::cleanup_staged_update_files();

    let poll_interval = lock_state()
        .as_ref()
        .map(|s| s.settings.poll_interval_ms)
        .unwrap_or(POLL_5_MIN);
    unsafe {
        SetTimer(msg_hwnd, TIMER_POLL, poll_interval, None);
    }
    schedule_update_check_timer(msg_hwnd);
    spawn_poll_thread();

    log::info!("app::run entered message loop");
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn create_message_window() -> Option<HWND> {
    let class_w = os::to_utf16_nul(APP_CLASS_NAME);
    let title_w = os::to_utf16_nul("Consumo de IA");
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(msg_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: PCWSTR::from_raw(class_w.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_w.as_ptr()),
            PCWSTR::from_raw(title_w.as_ptr()),
            WS_POPUP,
            -1000,
            -1000,
            1,
            1,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .ok()
    }
}

fn create_initial_bubbles() {
    let (settings, is_dark) = match lock_state().as_ref() {
        Some(s) => (s.settings.clone(), s.is_dark),
        None => return,
    };
    for provider in ProviderId::LIVE_USAGE {
        if settings.is_provider_enabled(provider) {
            spawn_bubble(provider, &settings, is_dark);
        }
    }
}

fn spawn_bubble(kind: ProviderId, settings: &Settings, is_dark: bool) {
    // "…" matches the in-flight/transient-error placeholder used by
    // `apply_results`, so the bubble has visible feedback during the first
    // poll rather than rendering with two empty grey tracks.
    let placeholder = "…".to_string();
    let hwnd = bubble::create(bubble::BubbleConfig {
        model: kind,
        size_logical: settings.bubble_size_logical,
        position: settings.bubble_positions.get(kind),
        session_pct: None,
        session_text: placeholder.clone(),
        session_resets_at: None,
        weekly_pct: None,
        weekly_text: placeholder,
        weekly_resets_at: None,
        today_text: String::new(),
        today_title: String::new(),
        average_text: String::new(),
        credits_title:String::new(),credits_text:String::new(),
        quota_label:"Cota".into(),secondary_label:String::new(),quota_duration_secs:None,
        is_dark,
        compact: settings.compact_mode,
    });
    if hwnd != HWND::default() {
        if let Some(s) = lock_state().as_mut() {
            s.bubbles.insert(kind, SendHwnd::from_hwnd(hwnd));
        }
    }
}

// ---------- Message-only window proc ----------

unsafe extern "system" fn msg_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_USAGE_UPDATED => {
            propagate_to_ui();
            LRESULT(0)
        }
        WM_APP_UPDATE_APPLIED => {
            // Persist the most recent UI preferences before the successor starts.
            let latest=lock_state().as_ref().map(|s|s.settings.clone());
            if let Some(settings)=latest { settings::save(&settings); }
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let action = tray::callback::handle(lparam);
            handle_tray_action(action);
            LRESULT(0)
        }
        WM_TIMER => {
            on_timer(hwnd, wparam.0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------- Bubble callbacks ----------

fn on_bubble_click(hwnd: HWND, model: ProviderId, point:POINT) {
    if bubble::is_toggle_point(hwnd,point) { toggle_compact_mode();return; }
    log::info!("bubble click (no drag): toggle panel for {model:?}");
    let data = build_panel_data(model);
    panel::toggle(data, hwnd);
}

fn toggle_compact_mode() {
    update_settings(|s|s.settings.compact_mode=!s.settings.compact_mode);
    let snapshot=lock_state().as_ref().map(|s|(s.settings.compact_mode,s.bubbles.values().copied().collect::<Vec<_>>()));
    if let Some((compact,windows))=snapshot {
        panel::hide();
        for hwnd in windows { bubble::set_compact(hwnd.to_hwnd(),compact); }
    }
}

fn on_bubble_right_click(hwnd: HWND, _model: ProviderId, _pt: POINT) {
    show_context_menu(hwnd);
}

fn on_bubble_moved(model: ProviderId, pos: (i32, i32)) {
    update_settings(|s| s.settings.bubble_positions.set(model, pos));
}

fn on_bubble_resized(_model: ProviderId, size_logical: i32) {
    set_bubble_size(size_logical);
}

fn on_menu_command(id: u32, _owner_hwnd: HWND) {
    let id = (id & 0xFFFF) as u16;
    match id {
        IDM_REFRESH => spawn_poll_thread(),
        IDM_EXIT => unsafe { PostQuitMessage(0) },
        IDM_FREQ_1MIN => set_poll_interval(POLL_1_MIN),
        IDM_FREQ_5MIN => set_poll_interval(POLL_5_MIN),
        IDM_FREQ_15MIN => set_poll_interval(POLL_15_MIN),
        IDM_FREQ_1HOUR => set_poll_interval(POLL_1_HOUR),
        IDM_MODEL_CLAUDE => toggle_model(ProviderId::Claude),
        IDM_MODEL_CHATGPT => toggle_model(ProviderId::ChatGpt),
        IDM_MODEL_OPENCODE_GO => {}
        IDM_START_WITH_WINDOWS => toggle_startup(),
        IDM_ONLY_CHATGPT => toggle_only_over_chatgpt(),
        IDM_RESET_POSITION => reset_positions(),
        IDM_VERSION_ACTION => check_updates_now(),
        IDM_HELP => show_help(_owner_hwnd),
        IDM_COMPACT => toggle_compact_mode(),
        IDM_DETAILS => {
            if let Some(model)=bubble::model(_owner_hwnd) { panel::toggle(build_panel_data(model),_owner_hwnd); }
        },
        IDM_SIZE_SMALLER => resize_bubbles(-bubble::RESIZE_STEP_LOGICAL),
        IDM_SIZE_LARGER => resize_bubbles(bubble::RESIZE_STEP_LOGICAL),
        IDM_RESET_SIZE => set_bubble_size(bubble::DEFAULT_BUBBLE_SIZE),
        IDM_UPDATE_AUTO_OFF => set_update_check_interval(None),
        IDM_UPDATE_AUTO_HOURLY => {
            set_update_check_interval(Some(settings::UPDATE_CHECK_HOURLY_SECS))
        }
        IDM_UPDATE_AUTO_DAILY => set_update_check_interval(Some(settings::UPDATE_CHECK_DAILY_SECS)),
        IDM_UPDATE_AUTO_WEEKLY => {
            set_update_check_interval(Some(settings::UPDATE_CHECK_WEEKLY_SECS))
        }
        tray::IDM_TOGGLE_WIDGET => toggle_widget_visibility(),
        IDM_RESTART => restart_app(),
        _ => {}
    }
}

// ---------- Timers ----------

fn on_timer(hwnd: HWND, id: usize) {
    match id {
        TIMER_POLL | TIMER_RESET_POLL => spawn_poll_thread(),
        TIMER_COUNTDOWN | TIMER_LOCAL_USAGE => refresh_countdowns(),
        TIMER_UPDATE_CHECK => {
            unsafe {
                let _ = KillTimer(hwnd, TIMER_UPDATE_CHECK);
            }
            // Re-read the current schedule: a stale timer must not update
            // after the user disabled or postponed automatic checks.
            schedule_update_check_timer(hwnd);
        }
        _ => {}
    }
}

// ---------- Poll thread ----------

/// At-most-one-in-flight gate for the poll worker. Spam-clicking Refresh
/// would otherwise stack concurrent HTTPS calls onto the same registry,
/// which both wastes bandwidth and (before the registry mutex landed)
/// could let two poll cycles interleave their writes to the snapshot map.
static POLL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

fn spawn_poll_thread() {
    if POLL_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let msg_hwnd = match lock_state().as_ref() {
        Some(s) => s.msg_hwnd,
        None => {
            POLL_IN_FLIGHT.store(false, Ordering::Release);
            return;
        }
    };
    std::thread::spawn(move || {
        do_poll();
        POLL_IN_FLIGHT.store(false, Ordering::Release);
        unsafe {
            let _ = PostMessageW(
                msg_hwnd.to_hwnd(),
                WM_APP_USAGE_UPDATED,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}

fn do_poll() {
    // Snapshot the inputs we need, then DROP the global lock before any
    // HTTPS call. Holding `lock_state()` through `poll_enabled` would block
    // the UI thread on every paint/menu for the duration of the request.
    let (http, registry, settings) = {
        let s = lock_state();
        let Some(s) = s.as_ref() else {
            return;
        };
        (s.http.clone(), s.registry.clone(), s.settings.clone())
    };
    let results = {
        let mut reg = registry.lock().expect("registry mutex poisoned");
        reg.poll_enabled(&http, &settings)
    };
    let auth_failures = apply_results(results);
    if !auth_failures.is_empty() {
        attempt_refresh(auth_failures, &registry);
    }
}

fn apply_results(
    results: Vec<(ProviderId, Result<UsageWindows, usage::Error>)>,
) -> Vec<ProviderId> {
    let mut auth_failures = Vec::new();
    let mut crossings: Vec<(ProviderId, u8)> = Vec::new();
    {
        let mut guard = lock_state();
        let Some(state) = guard.as_mut() else {
            return auth_failures;
        };
        if results.is_empty() {
            return auth_failures;
        }
        let strings = state.i18n.strings().clone();
        let mut any_ok = false;
        for (id, outcome) in results {
            match outcome {
                Ok(windows) => {
                    let entry = state.snapshots.entry(id).or_default();
                    // Warn when either reported quota approaches its limit.
                    let old_pct = entry.windows.pressure().unwrap_or(0.0);
                    let new_pct = windows.pressure().unwrap_or(0.0);
                    // Fire a balloon only the cycle a provider CROSSES a
                    // threshold so the user is nudged once, not on every
                    // subsequent poll while parked above it.
                    for threshold in [80u8, 95u8] {
                        if old_pct < threshold as f64 && new_pct >= threshold as f64 {
                            crossings.push((id, threshold));
                        }
                    }
                    entry.windows = windows;
                    // Fork: feed the burn-rate sampler (Codex weekly only).
                    if id == ProviderId::ChatGpt {
                        if let Some(weekly)=entry.windows.weekly() { crate::samples::record(weekly.utilization); }
                    }
                    entry.primary_text =
                        i18n::format_window_remaining(&windows.primary, &strings);
                    entry.secondary_text =
                        i18n::format_window_remaining(&windows.secondary, &strings);
                    refresh_detail_texts(entry, id, &strings);
                    any_ok = true;
                }
                Err(usage::Error::AuthRequired | usage::Error::TokenExpired) => {
                    auth_failures.push(id);
                    let entry = state.snapshots.entry(id).or_default();
                    entry.windows = UsageWindows::default();
                    entry.primary_text = "!".into();
                    entry.secondary_text = "!".into();
                }
                Err(e) => {
                    log::warn!("provider {id:?} poll failed: {e}");
                    let entry = state.snapshots.entry(id).or_default();
                    entry.windows = UsageWindows::default();
                    entry.primary_text = "…".into();
                    entry.secondary_text = "…".into();
                }
            }
        }
        state.last_poll_ok = any_ok;
    }
    // Lock released. Fire any threshold balloons outside the critical
    // section so tray::notify and i18n cloning don't block the UI thread.
    for (id, threshold) in crossings {
        show_threshold_balloon(id, threshold);
    }
    auth_failures
}

fn attempt_refresh(failures: Vec<ProviderId>, registry: &Arc<Mutex<Registry>>) {
    let orchestrator = usage::refresh::Orchestrator::new(REFRESH_TIMEOUT);
    // Pick the first provider whose refresh did not succeed and balloon
    // for it specifically. If both providers fail in the same cycle the
    // second will resurface on the next poll once the first is re-auth'd.
    let mut balloon_for: Option<ProviderId> = None;
    for id in failures {
        let outcome = {
            let reg = registry.lock().expect("registry mutex poisoned");
            reg.try_refresh(id, &orchestrator)
        };
        log::info!("refresh for {id:?}: {outcome:?}");
        if !matches!(outcome, usage::refresh::Outcome::Refreshed) {
            balloon_for.get_or_insert(id);
        }
    }
    if let Some(provider) = balloon_for {
        show_token_expired_balloon(provider);
    }
}

/// UI formatting only: local disk scans and account reads run on workers.
fn refresh_detail_texts(entry: &mut ProviderUiState, id: ProviderId, strings: &LocaleStrings) {
    let displayed=entry.windows.displayed();
    let first=displayed.first().copied();
    let weekly=entry.windows.weekly();
    entry.primary_text=first.map(|w|i18n::format_window_remaining(&w,strings)).unwrap_or_else(||"Cota indisponível".into());
    entry.secondary_text=displayed.get(1).map(|w|i18n::format_window_remaining(w,strings)).unwrap_or_default();
    let cd = i18n::format_precise_countdown(first.and_then(|w|w.resets_at), strings);
    entry.reset_text = if cd.is_empty() {
        format!("{}: -", strings.reset_prefix)
    } else {
        format!("{}: {}", strings.reset_prefix, cd)
    };
    entry.tokens_text.clear();

    // Daily budget: remaining % spread over the exact days left.
    entry.budget_text = match weekly.and_then(|w|w.resets_at)
        .and_then(|r| r.duration_since(SystemTime::now()).ok())
        .map(|d| d.as_secs_f64() / 86_400.0)
    {
        Some(days) if days > 0.0 => {
            let used = weekly.map(|w|w.utilization).unwrap_or(0.0).clamp(0.0, 100.0);
            format!(
                "{}: ~{}%/dia",
                strings.budget_prefix,
                crate::samples::fmt_1((100.0 - used) / days)
            )
        }
        _ => format!("{}: -", strings.budget_prefix),
    };

    // Burn-rate projection from recent samples.
    let remaining = (100.0 - weekly.map(|w|w.utilization).unwrap_or(100.0)).clamp(0.0, 100.0);
    entry.pace_text = match crate::samples::project(remaining) {
        crate::samples::Pace::Collecting => {
            format!("{}: coletando dados...", strings.pace_prefix)
        }
        crate::samples::Pace::Idle => format!("{}: sem consumo recente", strings.pace_prefix),
        crate::samples::Pace::Live {
            per_hour,
            hours_left,
        } => {
            let when = if hours_left >= 48.0 {
                format!("~{}d {}h", hours_left as u64 / 24, hours_left as u64 % 24)
            } else {
                format!("~{}h", hours_left.round() as u64)
            };
            format!(
                "{}: ~{}%/h · cota acaba em {}",
                strings.pace_prefix,
                crate::samples::fmt_1(per_hour),
                when
            )
        }
    };

    if weekly.is_none() { entry.budget_text.clear(); entry.pace_text.clear(); }
    if id != ProviderId::ChatGpt { entry.pace_text.clear(); }
    if id == ProviderId::ChatGpt {
        let today = chrono::Local::now().date_naive();
        let local = crate::local_usage::latest();
        entry.today_title = format!("Hoje {} neste PC:", today.format("%d/%m"));
        if let Some(n) = local.tokens {
            entry.tokens_today_text = format!("Hoje neste PC: {} tokens", crate::codex_tokens::format_tokens(n));
            // Full integer: activity remains visible even below 0.1 million.
            entry.today_micro = format!("{} tokens", crate::codex_tokens::format_tokens(n));
        } else {
            entry.today_micro = "Indisponível".into();
            entry.tokens_today_text = "Hoje neste PC: leitura indisponível".into();
        }
        let credits=crate::reset_credits::latest();
        entry.credits_title=credits.title;entry.credits_value=credits.value;
        entry.credits_count_text=credits.count_text;entry.credits_expiry_text=credits.expiry_text;
        let api = crate::appserver::latest();
        let from = today - chrono::Duration::days(7);
        let to = today - chrono::Duration::days(1);
        if let Some(n) = api.average_previous_week(today) {
            entry.average_micro = format!("{} tokens/dia", crate::codex_tokens::format_tokens(n));
            entry.tokens_text = format!("Média da conta {}-{}: {}/dia", from.format("%d/%m"), to.format("%d/%m"), crate::codex_tokens::format_tokens(n));
        } else {
            entry.average_micro = "Indisponível".into();
            entry.tokens_text = "Média da conta (7 dias): dados incompletos".into();
        }
    } else {
        entry.today_micro.clear();
        entry.today_title.clear();
        entry.average_micro.clear();
        entry.credits_title.clear();entry.credits_value.clear();entry.credits_count_text.clear();entry.credits_expiry_text.clear();
        entry.tokens_today_text = format!("{}: -", strings.tokens_today_prefix);
    };
}

fn refresh_countdowns() {
    {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        let strings = s.i18n.strings().clone();
        for (id, entry) in s.snapshots.iter_mut() {
            entry.primary_text =
                i18n::format_window_remaining(&entry.windows.primary, &strings);
            entry.secondary_text =
                i18n::format_window_remaining(&entry.windows.secondary, &strings);
            refresh_detail_texts(entry, *id, &strings);
        }
    }
    propagate_to_ui();
}

fn propagate_to_ui() {
    let snapshot = {
        let s = lock_state();
        s.as_ref().map(|s| UiSnapshot {
            bubbles: s.bubbles.clone(),
            snapshots: s.snapshots.clone(),
            settings: s.settings.clone(),
            i18n_strings: s.i18n.strings().clone(),
            is_dark: s.is_dark,
            msg_hwnd: s.msg_hwnd,
            last_poll_ok: s.last_poll_ok,
        })
    };
    let Some(snap) = snapshot else {
        return;
    };

    for (kind, hwnd) in snap.bubbles.iter() {
        let entry = snap.snapshots.get(kind);
        let windows=entry.map(|e|e.windows.displayed()).unwrap_or_default();
        let first=windows.first();let second=windows.get(1);
        let session_pct=first.map(|w|w.utilization);
        let weekly_pct=second.map(|w|w.utilization);
        let session_text=first.map(|w|i18n::format_precise_countdown(w.resets_at,&snap.i18n_strings)).unwrap_or_default();
        let weekly_text=second.map(|w|i18n::format_precise_countdown(w.resets_at,&snap.i18n_strings)).unwrap_or_default();
        let session_resets_at=first.and_then(|w|w.resets_at);
        let weekly_resets_at=second.and_then(|w|w.resets_at);
        let first_label=first.map(|w|w.label()).unwrap_or_else(||"Cota".into());
        let second_label=second.map(|w|w.label()).unwrap_or_default();
        let duration_secs=first.and_then(|w|w.duration_secs);
        log::info!(
            "bubble data: pct={session_pct:?} countdown='{session_text}' weekly_reset={weekly_resets_at:?}"
        );
        if let Some(e) = entry {
            log::info!(
                "detail data: reset='{}' budget='{}' pace='{}' tokens='{}' today='{}' micro='{}'",
                e.reset_text,
                e.budget_text,
                e.pace_text,
                e.tokens_text,
                e.tokens_today_text,
                e.today_micro
            );
        }
        bubble::update_data(
            hwnd.to_hwnd(),
            session_pct,
            session_text,
            session_resets_at,
            weekly_pct,
            weekly_text,
            weekly_resets_at,
            entry
                .map(|s| s.today_micro.clone())
                .unwrap_or_default(),
            entry
                .map(|s| s.today_title.clone())
                .unwrap_or_default(),
            entry.map(|s| s.average_micro.clone()).unwrap_or_default(),
            first_label, second_label, duration_secs,
            entry.map(|s|s.credits_title.clone()).unwrap_or_default(),
            entry.map(|s|s.credits_value.clone()).unwrap_or_default(),
        );
    }
    refresh_tray_icons_with(&snap);

    if panel::is_visible() {
        if let Some(model) = panel::current_model() {
            if let Some(provider_state) = snap.snapshots.get(&model) {
                panel::refresh_data(build_panel_data_from(&snap, model, provider_state));
            }
        }
    }
    schedule_countdown_timer(&snap);
}

#[derive(Clone)]
struct UiSnapshot {
    bubbles: HashMap<ProviderId, SendHwnd>,
    snapshots: HashMap<ProviderId, ProviderUiState>,
    settings: Settings,
    i18n_strings: LocaleStrings,
    is_dark: bool,
    msg_hwnd: SendHwnd,
    last_poll_ok: bool,
}

fn build_panel_data(model: ProviderId) -> PanelData {
    let s = lock_state();
    let Some(s) = s.as_ref() else {
        return placeholder_panel(model);
    };
    let strings = s.i18n.strings().clone();
    let provider_state = s.snapshots.get(&model).cloned().unwrap_or_default();
    // The panel renders every reported quota with its own duration and reset.
    PanelData {
        model,
        windows: provider_state.windows,
        reset_text: provider_state.reset_text,
        tokens_text: provider_state.tokens_text,
        budget_text: provider_state.budget_text,
        pace_text: provider_state.pace_text,
        tokens_today_text: provider_state.tokens_today_text,
        credits_count_text:provider_state.credits_count_text,credits_expiry_text:provider_state.credits_expiry_text,
        is_dark: s.is_dark,
        strings,
    }
}

fn build_panel_data_from(snap: &UiSnapshot, model: ProviderId, p: &ProviderUiState) -> PanelData {
    PanelData {
        model,
        windows: p.windows,
        reset_text: p.reset_text.clone(),
        tokens_text: p.tokens_text.clone(),
        budget_text: p.budget_text.clone(),
        pace_text: p.pace_text.clone(),
        tokens_today_text: p.tokens_today_text.clone(),
        credits_count_text:p.credits_count_text.clone(),credits_expiry_text:p.credits_expiry_text.clone(),
        is_dark: snap.is_dark,
        strings: snap.i18n_strings.clone(),
    }
}

fn placeholder_panel(model: ProviderId) -> PanelData {
    let strings = i18n::I18n::load().strings().clone();
    PanelData {
        model,
        windows: UsageWindows::default(),
        reset_text: String::new(),
        tokens_text: String::new(),
        budget_text: String::new(),
        pace_text: String::new(),
        tokens_today_text: String::new(),
        credits_count_text:String::new(),credits_expiry_text:String::new(),
        is_dark: false,
        strings,
    }
}

fn schedule_countdown_timer(snap: &UiSnapshot) {
    let mut min_ttl: Option<Duration> = None;
    for entry in snap.snapshots.values() {
        for w in [&entry.windows.primary, &entry.windows.secondary] {
            if let Some(d) = i18n::time_until_display_change(w.resets_at) {
                min_ttl = Some(min_ttl.map_or(d, |prev| prev.min(d)));
            }
        }
    }
    if let Some(d) = min_ttl {
        let ms = (d.as_millis().min(u32::MAX as u128) as u32).max(1_000);
        unsafe {
            let _ = KillTimer(snap.msg_hwnd.to_hwnd(), TIMER_COUNTDOWN);
            SetTimer(snap.msg_hwnd.to_hwnd(), TIMER_COUNTDOWN, ms, None);
        }
    }
}

// ---------- Tray icons ----------

fn refresh_tray_icons() {
    let snap = {
        let s = lock_state();
        s.as_ref().map(|s| UiSnapshot {
            bubbles: s.bubbles.clone(),
            snapshots: s.snapshots.clone(),
            settings: s.settings.clone(),
            i18n_strings: s.i18n.strings().clone(),
            is_dark: s.is_dark,
            msg_hwnd: s.msg_hwnd,
            last_poll_ok: s.last_poll_ok,
        })
    };
    if let Some(snap) = snap {
        refresh_tray_icons_with(&snap);
    }
}

fn refresh_tray_icons_with(snap: &UiSnapshot) {
    let mut icons = Vec::new();
    for provider in ProviderId::LIVE_USAGE {
        if !snap.settings.is_provider_enabled(provider) {
            continue;
        }
        let entry = snap.snapshots.get(&provider);
        icons.push(TrayIconData {
            kind: provider,
            percent: if snap.last_poll_ok {
                // The tray tracks the highest used percentage among reported quotas.
                entry.and_then(|e| e.windows.pressure())
            } else {
                None
            },
            tooltip: tray_tooltip(
                &provider_label(provider, &snap.i18n_strings),
                entry,
                &snap.i18n_strings,
            ),
        });
    }
    tray::sync(snap.msg_hwnd.to_hwnd(), &icons);
}

fn tray_tooltip(label: &str, entry: Option<&ProviderUiState>, strings: &LocaleStrings) -> String {
    let quota = entry.map(|e| e.windows.displayed().into_iter().map(|w|format!("{}: {:.0}% restantes",w.label(),100.0-w.utilization)).collect::<Vec<_>>().join(" · ")).filter(|s|!s.is_empty()).unwrap_or_else(||"Consultando a cota…".into());
    let local = entry.map(|e| e.tokens_today_text.as_str()).filter(|s| !s.is_empty()).unwrap_or("Consumo local: aguardando leitura");
    format!("{label}\n{quota}\n{local}\n{}", strings.tray_left_click)
}

fn provider_label(provider: ProviderId, strings: &LocaleStrings) -> String {
    match provider {
        ProviderId::Claude => strings.claude_label.clone(),
        ProviderId::ChatGpt => strings.chatgpt_label.clone(),
        ProviderId::OpenCodeGo => strings.opencode_go_label.clone(),
    }
}

fn handle_tray_action(action: TrayAction) {
    match action {
        TrayAction::None => {}
        TrayAction::ToggleWidget => toggle_widget_visibility(),
        TrayAction::ShowContextMenu => {
            let owner = lock_state()
                .as_ref()
                .and_then(|s| s.bubbles.values().next().copied())
                .map(|h| h.to_hwnd())
                .unwrap_or_default();
            if owner != HWND::default() {
                show_context_menu(owner);
            }
        }
    }
}

/// Re-read the system theme and, if it changed, push the new value into
/// the UI. Called from each bubble's WM_SETTINGCHANGE handler — Windows
/// posts that to every top-level window when the user toggles light/dark
/// in Settings, so this naturally fires once per change.
fn recheck_theme() {
    let now_dark = os::theme::is_dark();
    let changed = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        if s.is_dark == now_dark {
            false
        } else {
            s.is_dark = now_dark;
            true
        }
    };
    if changed {
        propagate_to_ui();
        refresh_tray_icons();
    }
}

fn show_threshold_balloon(provider: ProviderId, threshold: u8) {
    let payload = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        // Reuse the same cooldown as the token-expired balloon: any one
        // balloon at a time keeps notifications calm.
        if let Some(last) = s.last_balloon_at {
            if last.elapsed() < BALLOON_COOLDOWN {
                return;
            }
        }
        s.last_balloon_at = Some(Instant::now());
        let strings = s.i18n.strings();
        let provider_label = provider_label(provider, strings);
        let title = format!("{provider_label}: {threshold}% da cota usados");
        let body = if threshold >= 95 {
            strings.threshold_95_body.clone()
        } else {
            strings.threshold_80_body.clone()
        };
        (s.msg_hwnd, provider, title, body)
    };
    tray::notify_warning(payload.0.to_hwnd(), payload.1, &payload.2, &payload.3);
}

fn show_token_expired_balloon(failed: ProviderId) {
    let payload = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        if let Some(last) = s.last_balloon_at {
            if last.elapsed() < BALLOON_COOLDOWN {
                return;
            }
        }
        s.last_balloon_at = Some(Instant::now());
        let strings = s.i18n.strings();
        let (title, body) = match failed {
            ProviderId::Claude => (
                strings.token_expired_title.clone(),
                strings.token_expired_body.clone(),
            ),
            ProviderId::ChatGpt => (
                strings.chatgpt_token_expired_title.clone(),
                strings.chatgpt_token_expired_body.clone(),
            ),
            ProviderId::OpenCodeGo => (
                "OpenCode Go indisponível".into(),
                "A consulta deste serviço ainda não está disponível.".into(),
            ),
        };
        (s.msg_hwnd, failed, title, body)
    };
    tray::notify_warning(payload.0.to_hwnd(), payload.1, &payload.2, &payload.3);
}

/// Show the "Updated to vX.Y.Z" balloon on first launch after an
/// auto-update. Picks Claude as the host icon if it's registered;
/// otherwise falls back to Codex. If neither is registered the
/// notification silently drops — better than crashing.
fn announce_update_applied(_msg_hwnd: HWND, version: &str) {
    let payload = {
        let s = lock_state();
        let Some(s) = s.as_ref() else {
            return;
        };
        let strings = s.i18n.strings();
        let title = strings.update_applied_title.clone();
        let body = format!("{}{}", strings.update_applied_body, version);
        let host = if s.settings.show_claude_code {
            ProviderId::Claude
        } else {
            ProviderId::ChatGpt
        };
        (s.msg_hwnd, host, title, body)
    };
    tray::notify_info(payload.0.to_hwnd(), payload.1, &payload.2, &payload.3);
}

// ---------- Context menu ----------

struct ContextMenuSnapshot {
    strings: LocaleStrings,
    current_interval: u32,
    update_check_interval_secs: Option<u64>,
    show_claude: bool,
    show_chatgpt: bool,
    show_opencode_go: bool,
    widget_visible: bool,
    only_over_chatgpt: bool,
    install_channel: InstallChannel,
    update_status: UpdateStatus,
    bubble_size_logical: i32,
    reset_credits:crate::reset_credits::View,
    compact:bool,
}

/// Owns the entire native menu tree, including on construction failures.
struct PopupMenu(HMENU);
impl Drop for PopupMenu {
    fn drop(&mut self) { unsafe { let _ = DestroyMenu(self.0); } }
}
fn add_submenu(parent: HMENU, label: &str) -> Option<HMENU> {
    let child = unsafe { CreatePopupMenu().ok()? };
    let label = os::to_utf16_nul(label);
    if unsafe { AppendMenuW(parent, MF_POPUP, child.0 as usize, PCWSTR(label.as_ptr())) }.is_err() {
        unsafe { let _ = DestroyMenu(child); }
        return None;
    }
    Some(child)
}
fn separator(menu: HMENU) {
    unsafe { let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); }
}
fn checked(value: bool) -> MENU_ITEM_FLAGS {
    if value { MF_CHECKED } else { MENU_ITEM_FLAGS(0) }
}

/// Builds menus without reading settings, starting requests or displaying UI.
/// All state is supplied explicitly so every submenu and update state can be audited.
fn build_context_menu(snap: &ContextMenuSnapshot, startup_enabled: bool) -> Option<PopupMenu> {
    let menu = PopupMenu(unsafe { CreatePopupMenu().ok()? });
    let root = menu.0;
    let text = &snap.strings;
    append_item(root, IDM_REFRESH, &text.refresh, MENU_ITEM_FLAGS(0));
    append_item(root,IDM_DETAILS,"Ver detalhes do consumo",MENU_ITEM_FLAGS(0));
    append_item(root, tray::IDM_TOGGLE_WIDGET, &text.show_widget, checked(snap.widget_visible));
    separator(root);

    let display = add_submenu(root, &text.appearance)?;
    append_item(display,IDM_COMPACT,"Modo compacto",checked(snap.compact));
    append_item(display, IDM_ONLY_CHATGPT, &text.only_over_chatgpt, checked(snap.only_over_chatgpt));
    separator(display);
    append_item(display, IDM_SIZE_SMALLER, &text.size_smaller,
        if snap.bubble_size_logical <= bubble::MIN_BUBBLE_SIZE { MF_GRAYED } else { MENU_ITEM_FLAGS(0) });
    append_item(display, IDM_SIZE_LARGER, &text.size_larger,
        if snap.bubble_size_logical >= bubble::MAX_BUBBLE_SIZE { MF_GRAYED } else { MENU_ITEM_FLAGS(0) });
    append_item(display, IDM_RESET_SIZE, &text.reset_size,
        if snap.bubble_size_logical == bubble::DEFAULT_BUBBLE_SIZE { MF_GRAYED } else { MENU_ITEM_FLAGS(0) });
    append_item(display, IDM_RESET_POSITION, &text.reset_position, MENU_ITEM_FLAGS(0));

    let providers = add_submenu(root, &text.providers)?;
    append_item(providers, IDM_MODEL_CHATGPT, &text.chatgpt_label, checked(snap.show_chatgpt));
    append_item(providers, IDM_MODEL_CLAUDE, &text.claude_label, checked(snap.show_claude));
    append_item(providers, IDM_MODEL_OPENCODE_GO,
        &format!("{} (indisponível)", text.opencode_go_label), MF_GRAYED | checked(snap.show_opencode_go));

    let settings_menu = add_submenu(root, &text.settings)?;
    append_item(settings_menu, IDM_START_WITH_WINDOWS, &text.start_with_windows, checked(startup_enabled));
    let frequency = add_submenu(settings_menu, &text.update_frequency)?;
    for (id, interval, label) in [
        (IDM_FREQ_1MIN, POLL_1_MIN, &text.one_minute),
        (IDM_FREQ_5MIN, POLL_5_MIN, &text.five_minutes),
        (IDM_FREQ_15MIN, POLL_15_MIN, &text.fifteen_minutes),
        (IDM_FREQ_1HOUR, POLL_1_HOUR, &text.one_hour),
    ] { append_item(frequency, id, label, checked(interval == snap.current_interval)); }
    separator(frequency);
    append_item(frequency, 0, &text.local_refresh_note, MF_GRAYED);
    append_item(frequency, 0, &text.history_refresh_note, MF_GRAYED);

    let updates = add_submenu(root, &text.updates_menu)?;
    let busy = matches!(snap.update_status, UpdateStatus::Checking | UpdateStatus::Applying);
    append_item(updates, IDM_VERSION_ACTION, &version_action_label(snap),
        if busy { MF_GRAYED } else { MENU_ITEM_FLAGS(0) });
    let status = match snap.update_status {
        UpdateStatus::Idle => None,
        UpdateStatus::Checking => Some(&text.checking_for_updates),
        UpdateStatus::UpToDate => Some(&text.up_to_date),
        UpdateStatus::Applying => Some(&text.applying_update),
        UpdateStatus::Failed => Some(&text.update_failed),
    };
    if let Some(status) = status { append_item(updates, 0, status, MF_GRAYED); }
    separator(updates);
    let automatic = add_submenu(updates, &text.auto_update_check)?;
    for (id, value, label) in [
        (IDM_UPDATE_AUTO_OFF, None, &text.auto_check_disabled),
        (IDM_UPDATE_AUTO_HOURLY, Some(settings::UPDATE_CHECK_HOURLY_SECS), &text.auto_check_hourly),
        (IDM_UPDATE_AUTO_DAILY, Some(settings::UPDATE_CHECK_DAILY_SECS), &text.auto_check_daily),
        (IDM_UPDATE_AUTO_WEEKLY, Some(settings::UPDATE_CHECK_WEEKLY_SECS), &text.auto_check_weekly),
    ] { append_item(automatic, id, label, checked(value == snap.update_check_interval_secs)); }

    let resets=add_submenu(root,"Resets do Codex")?;
    for detail in &snap.reset_credits.details { append_item(resets,0,detail,MF_GRAYED); }
    separator(resets);
    append_item(resets,0,"Consulta a cada minuto · datas no horário deste PC",MF_GRAYED);
    append_item(root, IDM_HELP, &text.controls, MENU_ITEM_FLAGS(0));
    separator(root);
    append_item(root, IDM_RESTART, &text.restart, MENU_ITEM_FLAGS(0));
    append_item(root, IDM_EXIT, &text.exit, MENU_ITEM_FLAGS(0));
    Some(menu)
}

fn show_context_menu(owner_hwnd: HWND) {
    let snap = match lock_state().as_ref() {
        Some(s) => ContextMenuSnapshot {
            strings:s.i18n.strings().clone(),current_interval:s.settings.poll_interval_ms,
            update_check_interval_secs:s.settings.update_check_interval_secs,
            show_claude:s.settings.show_claude_code,show_chatgpt:s.settings.show_codex,
            show_opencode_go:s.settings.show_opencode_go,widget_visible:s.settings.widget_visible,
            only_over_chatgpt:s.settings.only_over_chatgpt,install_channel:s.install_channel,
            update_status:s.update_status,bubble_size_logical:s.settings.bubble_size_logical,
            reset_credits:crate::reset_credits::latest(),
            compact:s.settings.compact_mode,
        },
        None => return,
    };
    let Some(menu) = build_context_menu(&snap,is_startup_enabled()) else { return };
    unsafe {
        let mut pt=POINT::default(); let _=GetCursorPos(&mut pt);
        let _=SetForegroundWindow(owner_hwnd);
        let _=TrackPopupMenu(menu.0,TPM_RIGHTBUTTON,pt.x,pt.y,0,owner_hwnd,None);
    }
}

fn show_help(owner: HWND) {
    let strings = i18n::I18n::load().strings().clone();
    let title = os::to_utf16_nul(&format!("{} (v{})",strings.window_title,env!("CARGO_PKG_VERSION")));
    let body = os::to_utf16_nul(&strings.help_body);
    unsafe { MessageBoxW(owner,PCWSTR(body.as_ptr()),PCWSTR(title.as_ptr()),MB_OK | MB_ICONINFORMATION); }
}

fn check_updates_now() {
    let hwnd = lock_state().as_ref().and_then(|s| {
        if matches!(s.update_status,UpdateStatus::Checking | UpdateStatus::Applying) { None }
        else { Some(s.msg_hwnd) }
    });
    if let Some(hwnd)=hwnd { begin_update_check(hwnd.to_hwnd()); }
}

fn append_item(menu: HMENU, id: u16, label: &str, flags: MENU_ITEM_FLAGS) {
    let w = os::to_utf16_nul(label);
    unsafe {
        let _ = AppendMenuW(menu, flags, id as usize, PCWSTR::from_raw(w.as_ptr()));
    }
}

fn version_action_label(snap: &ContextMenuSnapshot) -> String {
    let base = &snap.strings.check_for_updates;
    // Append the running binary's version so the user can see what
    // they are on without opening an About dialog.
    let with_version = format!("{base} (v{})", env!("CARGO_PKG_VERSION"));
    match snap.install_channel {
        InstallChannel::Winget => format!("{with_version} ({})", snap.strings.update_via_winget),
        InstallChannel::Portable => with_version,
    }
}

// ---------- Menu actions ----------

fn set_poll_interval(ms: u32) {
    let (snap, msg_hwnd) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        s.settings.poll_interval_ms = ms;
        (s.settings.clone(), s.msg_hwnd)
    };
    settings::save(&snap);
    unsafe {
        let _ = KillTimer(msg_hwnd.to_hwnd(), TIMER_POLL);
        SetTimer(msg_hwnd.to_hwnd(), TIMER_POLL, ms, None);
    }
}

fn toggle_model(model: ProviderId) {
    let (settings, is_dark) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        if !model.metadata().live_usage {
            return;
        }
        s.settings.toggle_provider(model);
        if !s.settings.has_enabled_live_provider() {
            s.settings.set_provider_enabled(model, true);
        }
        (s.settings.clone(), s.is_dark)
    };
    settings::save(&settings);

    let want = settings.is_provider_enabled(model);
    let existing = lock_state()
        .as_ref()
        .and_then(|s| s.bubbles.get(&model).copied());
    match (want, existing) {
        (true, None) => spawn_bubble(model, &settings, is_dark),
        (false, Some(h)) => {
            bubble::destroy(h.to_hwnd());
            if let Some(s) = lock_state().as_mut() {
                s.bubbles.remove(&model);
            }
        }
        _ => {}
    }
    refresh_tray_icons();
    spawn_poll_thread();
}

/// Fork (Rafael): focus-follow toggle. Persists to settings and pushes the
/// flag into the bubble module (which enforces it on its 0.35s check timer).
fn toggle_only_over_chatgpt() {
    let (enabled, snap) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        s.settings.only_over_chatgpt = !s.settings.only_over_chatgpt;
        (s.settings.only_over_chatgpt, s.settings.clone())
    };
    settings::save(&snap);
    bubble::set_only_over_chatgpt(enabled);
}

fn toggle_widget_visibility() {
    let (visible, snap) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        s.settings.widget_visible = !s.settings.widget_visible;
        (s.settings.widget_visible, s.settings.clone())
    };
    settings::save(&snap);
    let hwnds: Vec<HWND> = lock_state()
        .as_ref()
        .map(|s| s.bubbles.values().map(|h| h.to_hwnd()).collect())
        .unwrap_or_default();
    for h in hwnds {
        bubble::set_user_visible(h, visible);
    }
}

fn reset_positions() {
    let snap = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        s.settings.bubble_positions.reset_all();
        s.settings.clone()
    };
    settings::save(&snap);
    let hwnds: Vec<HWND> = lock_state()
        .as_ref()
        .map(|s| s.bubbles.values().map(|h| h.to_hwnd()).collect())
        .unwrap_or_default();
    for h in hwnds {
        bubble::destroy(h);
    }
    if let Some(s) = lock_state().as_mut() {
        s.bubbles.clear();
    }
    create_initial_bubbles();
    // The freshly-spawned bubbles boot with a "…" placeholder. Push the
    // cached snapshot so they render the last-known data immediately, and
    // kick a poll for users who used Reset Position to recover from
    // staleness.
    propagate_to_ui();
    spawn_poll_thread();
}

fn resize_bubbles(delta: i32) {
    let current = lock_state()
        .as_ref()
        .map(|s| s.settings.bubble_size_logical)
        .unwrap_or(bubble::DEFAULT_BUBBLE_SIZE);
    set_bubble_size(current + delta);
}

fn set_bubble_size(size_logical: i32) {
    let (hwnds, snap) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        let new_size = size_logical.clamp(bubble::MIN_BUBBLE_SIZE, bubble::MAX_BUBBLE_SIZE);
        if new_size == s.settings.bubble_size_logical {
            return;
        }
        s.settings.bubble_size_logical = new_size;
        let hwnds = s.bubbles.values().map(|h| h.to_hwnd()).collect::<Vec<_>>();
        (hwnds, s.settings.clone())
    };
    settings::save(&snap);
    for hwnd in hwnds {
        bubble::set_size_logical(hwnd, snap.bubble_size_logical);
    }
}

/// Claim the full check/install cycle before spawning any worker. Both the
/// timer and the menu go through this gate; downloading/installing owns it too.
fn claim_update_cycle(status: &mut UpdateStatus) -> bool {
    if matches!(status,UpdateStatus::Checking|UpdateStatus::Applying) { return false; }
    *status=UpdateStatus::Checking;
    true
}

/// One pipeline for both triggers. There is no pending-confirmation state.
/// Installer injection allows tests to exercise the automatic transition
/// without replacing a real user installation.
fn automatic_update_flow(
    result:Result<CheckOutcome,update::Error>,
    mut status:impl FnMut(UpdateStatus),
    install:impl FnOnce(&update::Release)->Result<(),update::Error>,
)->Result<bool,update::Error> {
    match result {
        Ok(CheckOutcome::UpToDate)=> { status(UpdateStatus::UpToDate); Ok(false) }
        Ok(CheckOutcome::Available(release))=> {
            status(UpdateStatus::Applying);
            match install(&release) {
                Ok(())=>Ok(true),
                Err(e)=>{status(UpdateStatus::Failed);Err(e)}
            }
        }
        Err(e)=>{status(UpdateStatus::Failed);Err(e)}
    }
}

#[cfg(test)]
mod automatic_update_tests {
    use super::*;
    use std::cell::RefCell;
    fn release()->update::Release {
        update::Release {
            version:update::release::Version {major:9,minor:0,patch:0},
            asset_url:"https://example.invalid/fixture.exe".into(),asset_sha256:Some([7;32]),
        }
    }
    #[test]
    fn detecting_a_release_installs_it_without_a_second_action() {
        // The menu and timer both invoke the same claimed cycle.
        for _trigger in ["menu","timer"] {
            let mut status=UpdateStatus::Idle;
            assert!(claim_update_cycle(&mut status));
            assert!(!claim_update_cycle(&mut status));
            let events=RefCell::new(Vec::new());
            let installed=automatic_update_flow(Ok(CheckOutcome::Available(release())),
                |next| { assert_eq!(next,UpdateStatus::Applying);events.borrow_mut().push("installing"); },
                |r| {
                    assert_eq!(r.version,release().version);assert_eq!(r.asset_sha256,Some([7;32]));
                    events.borrow_mut().push("installed");Ok(())
                }).unwrap();
            assert!(installed);assert_eq!(*events.borrow(),["installing","installed"]);
        }
    }
    #[test]
    fn current_or_failed_checks_never_invoke_the_installer() {
        let state=RefCell::new(UpdateStatus::Checking);
        assert!(!automatic_update_flow(Ok(CheckOutcome::UpToDate),|s|*state.borrow_mut()=s,
            |_|panic!("must not install the current version")).unwrap());
        assert_eq!(*state.borrow(),UpdateStatus::UpToDate);
        assert!(automatic_update_flow(Err(update::Error::NoAsset),|s|*state.borrow_mut()=s,
            |_|panic!("must not install after a failed check")).is_err());
        assert_eq!(*state.borrow(),UpdateStatus::Failed);
    }
    #[test]
    fn install_failure_releases_the_cycle_for_a_retry() {
        let state=RefCell::new(UpdateStatus::Checking);
        let result=automatic_update_flow(Ok(CheckOutcome::Available(release())),
            |s|*state.borrow_mut()=s, |_| {
                assert_eq!(*state.borrow(),UpdateStatus::Applying);
                assert!(!claim_update_cycle(&mut state.borrow_mut()));
                Err(update::Error::ChecksumMismatch {expected:"fixture".into(),actual:"different".into()})
            });
        assert!(result.is_err());assert_eq!(*state.borrow(),UpdateStatus::Failed);
        assert!(claim_update_cycle(&mut state.borrow_mut()));
        assert_eq!(*state.borrow(),UpdateStatus::Checking);
    }
}

fn schedule_update_check_timer(hwnd: HWND) {
    let (last, interval) = match lock_state().as_ref() {
        Some(s) => (
            s.settings.last_update_check_unix,
            s.settings.update_check_interval_secs,
        ),
        None => return,
    };
    let Some(interval) = interval else {
        // Auto-check disabled. Manual "Check for updates" still works via
        // the menu; this just suppresses the timer.
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let due = last.map_or(true, |t| now.saturating_sub(t) >= interval);
    if due {
        begin_update_check(hwnd);
    } else {
        let remaining = interval.saturating_sub(now.saturating_sub(last.unwrap_or(0)));
        let ms = (remaining.saturating_mul(1000).min(u32::MAX as u64)) as u32;
        unsafe {
            SetTimer(hwnd, TIMER_UPDATE_CHECK, ms, None);
        }
    }
}

fn begin_update_check(hwnd: HWND) {
    let (http,channel)= {
        let mut guard=lock_state();
        let Some(s)=guard.as_mut() else { return };
        if !claim_update_cycle(&mut s.update_status) { return; }
        (s.http.clone(),s.install_channel)
    };
    let send_hwnd=SendHwnd::from_hwnd(hwnd);
    std::thread::spawn(move || {
        let result=update::release::fetch_latest(&http);
        let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let settings=lock_state().as_mut().map(|s| {
            s.settings.last_update_check_unix=Some(now);
            s.settings.clone()
        });
        if let Some(ref settings)=settings { settings::save(settings); }
        let installed=automatic_update_flow(result,
            |status| { if let Some(s)=lock_state().as_mut() { s.update_status=status; } },
            |release|match channel {
                InstallChannel::Portable=>update::install::begin(&http,release),
                InstallChannel::Winget=>Err(update::Error::NotWritable("Atualização por WinGet ainda indisponível".into())),
            });
        match installed {
            Ok(true)=>unsafe {
                // The verified replacement already launched its successor.
                // Quit this process so the child can acquire the widget mutex.
                let _=PostMessageW(send_hwnd.to_hwnd(),WM_APP_UPDATE_APPLIED,WPARAM(0),LPARAM(0));
            },
            outcome=> {
                if let Err(e)=outcome { log::error!("automatic update failed: {e}"); }
                // A user can change/disable the schedule while downloading.
                // Use the current setting, not a stale copy from the check.
                let interval=lock_state().as_ref().and_then(|s|s.settings.update_check_interval_secs);
                if let Some(interval)=interval {
                    let ms=(interval.saturating_mul(1000).min(u32::MAX as u64)) as u32;
                    unsafe { SetTimer(send_hwnd.to_hwnd(),TIMER_UPDATE_CHECK,ms,None); }
                }
            }
        }
    });
}

fn set_update_check_interval(value: Option<u64>) {
    let (snap, msg_hwnd) = {
        let mut s = lock_state();
        let Some(s) = s.as_mut() else {
            return;
        };
        s.settings.update_check_interval_secs = value;
        (s.settings.clone(), s.msg_hwnd)
    };
    settings::save(&snap);
    let hwnd = msg_hwnd.to_hwnd();
    unsafe {
        let _ = KillTimer(hwnd, TIMER_UPDATE_CHECK);
    }
    if value.is_some() {
        schedule_update_check_timer(hwnd);
    }
}

// ---------- Restart ----------

/// Relaunch the running binary by spawning a detached child via
/// `CreateProcessW`. The child waits on our PID before acquiring the
/// singleton mutex, so no shell handoff or timer is required.
fn restart_app() {
    // Defensive flush — bubble positions and most settings already persist
    // on change, but a final save is cheap insurance. Snapshot then drop the
    // lock before the disk write so the UI thread doesn't block on I/O.
    let snap = lock_state().as_ref().map(|s| s.settings.clone());
    if let Some(s) = snap {
        settings::save(&s);
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::error!("restart: current_exe failed: {e}");
            return;
        }
    };

    let pid = unsafe { GetCurrentProcessId() };
    let args = vec![
        OsString::from("--wait-pid"),
        OsString::from(pid.to_string()),
    ];
    match update::handoff::spawn_detached(&exe, &args) {
        Ok(()) => {
            log::info!("restart: spawned detached child (parent pid={pid}), posting quit");
            unsafe { PostQuitMessage(0) };
        }
        Err(e) => log::error!("restart: spawn_detached failed: {e}"),
    }
}

// ---------- Start-with-Windows ----------

fn is_startup_enabled() -> bool {
    os::registry::value_exists(STARTUP_REGISTRY_PATH, STARTUP_VALUE_NAME)
}

fn toggle_startup() {
    if is_startup_enabled() {
        let _ = os::registry::delete_value(STARTUP_REGISTRY_PATH, STARTUP_VALUE_NAME);
    } else if let Ok(exe) = std::env::current_exe() {
        let quoted = format!("\"{}\"", exe.to_string_lossy());
        let _ = os::registry::write_string(STARTUP_REGISTRY_PATH, STARTUP_VALUE_NAME, &quoted);
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;
    #[derive(Debug)]
    struct Entry { path:String, id:u16, flags:u32 }
    fn inventory(menu:HMENU, prefix:&str, out:&mut Vec<Entry>) {
        unsafe {
            for pos in 0..GetMenuItemCount(menu) {
                let mut buf=[0_u16;512];
                let n=GetMenuStringW(menu,pos as u32,Some(&mut buf),MF_BYPOSITION);
                let label=String::from_utf16_lossy(&buf[..n as usize]);
                if label.is_empty() { continue; }
                let path=if prefix.is_empty(){label}else{format!("{prefix} > {label}")};
                let child=GetSubMenu(menu,pos);
                if child.0.is_null() {
                    out.push(Entry {path,id:GetMenuItemID(menu,pos) as u16,flags:GetMenuState(menu,pos as u32,MF_BYPOSITION)});
                } else { inventory(child,&path,out); }
            }
        }
    }
    fn snapshot(status:UpdateStatus)->ContextMenuSnapshot {
        ContextMenuSnapshot {
            strings:i18n::I18n::load().strings().clone(),current_interval:POLL_5_MIN,
            update_check_interval_secs:Some(settings::UPDATE_CHECK_DAILY_SECS),
            show_claude:false,show_chatgpt:true,show_opencode_go:false,widget_visible:true,
            only_over_chatgpt:true,install_channel:InstallChannel::Portable,
            update_status:status,bubble_size_logical:bubble::DEFAULT_BUBBLE_SIZE,
            reset_credits:crate::reset_credits::Snapshot::default().view(),
            compact:false,
        }
    }
    #[test]
    fn every_menu_state_preserves_actions_without_an_extra_install_button() {
        let mut report=String::new();
        for status in [UpdateStatus::Idle,UpdateStatus::Checking,UpdateStatus::UpToDate,
            UpdateStatus::Applying,UpdateStatus::Failed] {
            let snap=snapshot(status);
            let menu=build_context_menu(&snap,true).unwrap();
            let mut items=Vec::new();inventory(menu.0,"",&mut items);
            let ids:std::collections::HashSet<_>=items.iter().filter(|e|e.id!=0).map(|e|e.id).collect();
            assert_eq!(ids.len(),items.iter().filter(|e|e.id!=0).count(),"duplicate actionable ID");
            for id in [IDM_REFRESH,IDM_EXIT,IDM_RESTART,IDM_HELP,IDM_COMPACT,IDM_DETAILS,IDM_FREQ_1MIN,IDM_FREQ_5MIN,
                IDM_FREQ_15MIN,IDM_FREQ_1HOUR,IDM_MODEL_CLAUDE,IDM_MODEL_CHATGPT,IDM_MODEL_OPENCODE_GO,
                IDM_START_WITH_WINDOWS,IDM_RESET_POSITION,IDM_VERSION_ACTION,IDM_ONLY_CHATGPT,
                IDM_SIZE_SMALLER,IDM_SIZE_LARGER,IDM_RESET_SIZE,IDM_UPDATE_AUTO_OFF,IDM_UPDATE_AUTO_HOURLY,
                IDM_UPDATE_AUTO_DAILY,IDM_UPDATE_AUTO_WEEKLY,tray::IDM_TOGGLE_WIDGET] { assert!(ids.contains(&id),"missing action {id}"); }
            let search=items.iter().find(|e|e.id==IDM_VERSION_ACTION).unwrap();
            assert!(search.path.ends_with(&format!("Buscar nova atualização (v{})",env!("CARGO_PKG_VERSION"))));
            let busy=matches!(status,UpdateStatus::Checking|UpdateStatus::Applying);
            assert_eq!(search.flags & MF_GRAYED.0 != 0,busy);
            assert!(!ids.contains(&38),"the former extra installation action must not be required");
            for id in [IDM_MODEL_CHATGPT,IDM_FREQ_5MIN,IDM_UPDATE_AUTO_DAILY,IDM_START_WITH_WINDOWS,tray::IDM_TOGGLE_WIDGET,IDM_ONLY_CHATGPT] {
                assert!(items.iter().find(|e|e.id==id).unwrap().flags & MF_CHECKED.0 != 0);
            }
            assert!(items.iter().find(|e|e.id==IDM_MODEL_OPENCODE_GO).unwrap().flags & MF_GRAYED.0 != 0);
            for e in &items {
                for old in ["Refresh","Settings","Providers","Controls","Check for updates","Update frequency","Start with Windows","Restart","Exit","Update failed"] {
                    assert!(!e.path.contains(old),"untranslated label: {}",e.path);
                }
            }
            report.push_str(&format!("\n{status:?}\n"));
            for e in &items { report.push_str(&format!("{} [id={}, flags={}]\n",e.path,e.id,e.flags)); }
        }
        if let Ok(path)=std::env::var("CODEX_WIDGET_MENU_AUDIT_PATH") { std::fs::write(path,report).unwrap(); }
    }
    #[test]
    fn reset_credit_details_are_read_only_and_preserve_the_reported_count() {
        let mut snap=snapshot(UpdateStatus::Idle);
        snap.reset_credits=crate::reset_credits::Snapshot::parse(&serde_json::json!({
            "rateLimitResetCredits":{"availableCount":2,"credits":[
                {"id":"fixture-a","status":"available","expiresAt":2000000000_i64},
                {"id":"fixture-b","status":"available","expiresAt":null}
            ]}})).view();
        let menu=build_context_menu(&snap,false).unwrap();
        let mut items=Vec::new();inventory(menu.0,"",&mut items);
        let resets:Vec<_>=items.iter().filter(|e|e.path.starts_with("Resets do Codex >")).collect();
        assert!(resets.len()>=4);
        assert!(resets.iter().all(|e|e.id==0 && e.flags&MF_GRAYED.0!=0));
        assert!(resets.iter().any(|e|e.path.contains("Resets disponíveis na conta: 2")));
        assert!(resets.iter().any(|e|e.path.contains("sem vencimento")));
        assert!(resets.iter().any(|e|e.path.contains("hora local")));
    }

    #[test]
    fn changed_preferences_and_size_limits_are_reflected_in_the_native_menu() {
        let mut snap=snapshot(UpdateStatus::Idle);
        snap.show_chatgpt=false;snap.show_claude=true;snap.only_over_chatgpt=false;snap.widget_visible=false;
        snap.current_interval=POLL_1_HOUR;snap.update_check_interval_secs=None;snap.compact=true;
        for width in [bubble::MIN_BUBBLE_SIZE,bubble::MAX_BUBBLE_SIZE] {
            snap.bubble_size_logical=width;
            let menu=build_context_menu(&snap,false).unwrap();
            let mut items=Vec::new();inventory(menu.0,"",&mut items);
            let flags=|id|items.iter().find(|e|e.id==id).unwrap().flags;
            for id in [IDM_MODEL_CHATGPT,IDM_ONLY_CHATGPT,IDM_START_WITH_WINDOWS,tray::IDM_TOGGLE_WIDGET] { assert_eq!(flags(id)&MF_CHECKED.0,0); }
            for id in [IDM_MODEL_CLAUDE,IDM_FREQ_1HOUR,IDM_UPDATE_AUTO_OFF,IDM_COMPACT] { assert_ne!(flags(id)&MF_CHECKED.0,0); }
            assert_eq!(flags(IDM_SIZE_SMALLER)&MF_GRAYED.0!=0,width==bubble::MIN_BUBBLE_SIZE);
            assert_eq!(flags(IDM_SIZE_LARGER)&MF_GRAYED.0!=0,width==bubble::MAX_BUBBLE_SIZE);
        }
    }
}
