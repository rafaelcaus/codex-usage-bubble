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
    Available,
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
    update_release: Option<update::Release>,
    last_balloon_at: Option<Instant>,
}

#[derive(Clone, Default)]
struct ProviderUiState {
    windows: UsageWindows,
    primary_text: String,
    secondary_text: String,
    /// Fork: "Falta para resetar: 6d 3h" (weekly window, precise).
    reset_text: String,
    /// Fork: "Tokens neste PC desde o reset: 1.234.567" (local sessions).
    tokens_text: String,
    /// Fork: daily budget "%/dia" from exact remaining time.
    budget_text: String,
    /// Fork: burn-rate projection line.
    pace_text: String,
    /// Fork: tokens since local midnight.
    tokens_today_text: String,
    /// Fork: compact "55,3M" for the bubble micro-line (API value only).
    today_micro: String,
    /// Fork: "Consumo hoje 24/09:" title line for the bubble.
    today_title: String,
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
        update_release: None,
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
    let title_w = os::to_utf16_nul("Claude Code Usage Bubble");
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
        is_dark,
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

fn on_bubble_click(hwnd: HWND, model: ProviderId) {
    log::info!("bubble click (no drag): toggle panel for {model:?}");
    let data = build_panel_data(model);
    panel::toggle(data, hwnd);
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
        IDM_VERSION_ACTION => version_action(),
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
        TIMER_COUNTDOWN => refresh_countdowns(),
        TIMER_UPDATE_CHECK => {
            unsafe {
                let _ = KillTimer(hwnd, TIMER_UPDATE_CHECK);
            }
            begin_update_check(hwnd);
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
                    // Fork: thresholds watch the PRIMARY window, which IS the
                    // weekly quota on the Pro plan (verified vs desktop app).
                    let old_pct = entry.windows.primary.utilization;
                    let new_pct = windows.primary.utilization;
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
                        crate::samples::record(entry.windows.primary.utilization);
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
                    entry.primary_text = "!".into();
                    entry.secondary_text = "!".into();
                }
                Err(e) => {
                    log::warn!("provider {id:?} poll failed: {e}");
                    let entry = state.snapshots.entry(id).or_default();
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

/// Fork (Rafael): weekly precise-reset + local-tokens lines for the
/// expanded panel and tray tooltip. Token counting runs only for Codex and
/// is fingerprint-cached inside `codex_tokens`, so the per-minute refresh
/// stays cheap.
fn refresh_detail_texts(entry: &mut ProviderUiState, id: ProviderId, strings: &LocaleStrings) {
    // Fork: boundaries use the PRIMARY window = weekly quota on Pro.
    let cd = i18n::format_precise_countdown(entry.windows.primary.resets_at, strings);
    entry.reset_text = if cd.is_empty() {
        format!("{}: -", strings.reset_prefix)
    } else {
        format!("{}: {}", strings.reset_prefix, cd)
    };
    entry.tokens_text = if id == ProviderId::ChatGpt {
        match entry
            .windows
            .primary
            .resets_at
            .and_then(crate::codex_tokens::tokens_since_reset)
        {
            Some(n) => format!(
                "{}: {}",
                strings.tokens_prefix,
                crate::codex_tokens::format_tokens(n)
            ),
            None => format!("{}: -", strings.tokens_prefix),
        }
    } else {
        format!("{}: -", strings.tokens_prefix)
    };

    // Daily budget: remaining % spread over the exact days left.
    entry.budget_text = match entry.windows.primary.resets_at
        .and_then(|r| r.duration_since(SystemTime::now()).ok())
        .map(|d| d.as_secs_f64() / 86_400.0)
    {
        Some(days) if days > 0.0 => {
            let used = entry.windows.primary.utilization.clamp(0.0, 100.0);
            format!(
                "{}: ~{}%/dia",
                strings.budget_prefix,
                crate::samples::fmt_1((100.0 - used) / days)
            )
        }
        _ => format!("{}: -", strings.budget_prefix),
    };

    // Burn-rate projection from recent samples.
    let remaining = (100.0 - entry.windows.primary.utilization).clamp(0.0, 100.0);
    entry.pace_text = match crate::samples::project(remaining) {
        crate::samples::Pace::Collecting => {
            format!("{}: coletando dados...", strings.pace_prefix)
        }
        crate::samples::Pace::Idle => format!("{}: parado", strings.pace_prefix),
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
                "{}: ~{}%/h -> acaba em {}",
                strings.pace_prefix,
                crate::samples::fmt_1(per_hour),
                when
            )
        }
    };

    // Tokens today: account API first (covers desktop app + CLI on this
    // machine), local CLI sessions as fallback, dash when neither.
    let api = crate::appserver::latest();
    // "Consumo hoje DD/MM:" title for the bubble (local date).
    let today_title = {
        use chrono::Local;
        let d = Local::now().format("%d/%m").to_string();
        format!("Consumo hoje {d}:")
    };
    if id == ProviderId::ChatGpt {
        if let Some(t) = api.tokens_today {
            let n = (t.max(0) as u64).min(u64::MAX);
            entry.tokens_today_text = format!(
                "Tokens hoje: {}",
                crate::codex_tokens::format_tokens(n)
            );
            let (num, unit) = crate::codex_tokens::format_tokens_words(t);
            entry.today_micro = format!("{num} {unit}");
            entry.today_title = today_title;
        } else {
            entry.today_micro.clear();
            entry.today_title.clear();
            match local_midnight().and_then(crate::codex_tokens::tokens_since_reset) {
                Some(n) if n > 0 => {
                    entry.tokens_today_text = format!(
                        "Tokens hoje (so CLI neste PC): {}",
                        crate::codex_tokens::format_tokens(n)
                    )
                }
                _ => {
                    entry.tokens_today_text =
                        format!("{}: -", strings.tokens_today_prefix)
                }
            }
        }
    } else {
        entry.today_micro.clear();
        entry.today_title.clear();
        entry.tokens_today_text = format!("{}: -", strings.tokens_today_prefix);
    };
}

/// Fork: today 00:00 local time as SystemTime (for "tokens today").
fn local_midnight() -> Option<SystemTime> {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    let midnight = now.date_naive().and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&midnight)
        .single()
        .map(|dt| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(dt.timestamp() as u64))
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
        // Fork (Rafael): on the Pro plan the PRIMARY window IS the weekly
        // quota (verified against the desktop app: 3% used = 97% left).
        // The bubble shows primary only; the unused secondary slot is fed
        // the same values so every consumer stays consistent.
        let session_pct = entry.map(|s| s.windows.primary.utilization);
        let weekly_pct = entry.map(|s| s.windows.primary.utilization);
        // The bubble paints the percent inline inside the ring/bar fill, so
        // the caption only needs the PRECISE countdown ("6d 3h").
        // The panel shows the combined "X% usada · Y% resta · Zh" strings.
        let session_text = entry
            .map(|s| {
                i18n::format_precise_countdown(s.windows.primary.resets_at, &snap.i18n_strings)
            })
            .unwrap_or_default();
        let weekly_text = entry
            .map(|s| i18n::format_countdown(s.windows.primary.resets_at, &snap.i18n_strings))
            .unwrap_or_default();
        let session_resets_at = entry.and_then(|s| s.windows.primary.resets_at);
        let weekly_resets_at = entry.and_then(|s| s.windows.primary.resets_at);
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
    // Fork: the panel's single usage row shows the PRIMARY window, which IS
    // the weekly quota on the Pro plan (label stays "7d").
    PanelData {
        model,
        session_pct: provider_state.windows.primary.utilization,
        session_text: provider_state.primary_text.clone(),
        weekly_pct: provider_state.windows.primary.utilization,
        weekly_text: provider_state.primary_text.clone(),
        reset_text: provider_state.reset_text,
        tokens_text: provider_state.tokens_text,
        budget_text: provider_state.budget_text,
        pace_text: provider_state.pace_text,
        tokens_today_text: provider_state.tokens_today_text,
        is_dark: s.is_dark,
        strings,
    }
}

fn build_panel_data_from(snap: &UiSnapshot, model: ProviderId, p: &ProviderUiState) -> PanelData {
    PanelData {
        model,
        session_pct: p.windows.primary.utilization,
        session_text: p.primary_text.clone(),
        weekly_pct: p.windows.primary.utilization,
        weekly_text: p.primary_text.clone(),
        reset_text: p.reset_text.clone(),
        tokens_text: p.tokens_text.clone(),
        budget_text: p.budget_text.clone(),
        pace_text: p.pace_text.clone(),
        tokens_today_text: p.tokens_today_text.clone(),
        is_dark: snap.is_dark,
        strings: snap.i18n_strings.clone(),
    }
}

fn placeholder_panel(model: ProviderId) -> PanelData {
    let strings = i18n::I18n::load().strings().clone();
    PanelData {
        model,
        session_pct: 0.0,
        session_text: String::new(),
        weekly_pct: 0.0,
        weekly_text: String::new(),
        reset_text: String::new(),
        tokens_text: String::new(),
        budget_text: String::new(),
        pace_text: String::new(),
        tokens_today_text: String::new(),
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
                // Fork: tray badge tracks the PRIMARY window = weekly quota.
                entry.map(|e| e.windows.primary.utilization)
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
    // Fork: weekly-only tooltip with precise reset + local tokens.
    let weekly = entry
        .map(|e| e.secondary_text.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("...");
    let reset = entry
        .map(|e| e.reset_text.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("...");
    let tokens = entry
        .map(|e| e.tokens_text.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("...");
    let budget = entry
        .map(|e| e.budget_text.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("...");
    format!(
        "{label}\n{}: {weekly}\n{reset}\n{tokens}\n{budget}\n{}",
        strings.weekly_window, strings.tray_left_click
    )
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
        let title = format!("{provider_label} · {threshold}%");
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
                strings.opencode_go_label.clone(),
                strings.update_failed.clone(),
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
}

fn show_context_menu(owner_hwnd: HWND) {
    let snap = match lock_state().as_ref() {
        Some(s) => ContextMenuSnapshot {
            strings: s.i18n.strings().clone(),
            current_interval: s.settings.poll_interval_ms,
            update_check_interval_secs: s.settings.update_check_interval_secs,
            show_claude: s.settings.show_claude_code,
            show_chatgpt: s.settings.show_codex,
            show_opencode_go: s.settings.show_opencode_go,
            widget_visible: s.settings.widget_visible,
            only_over_chatgpt: s.settings.only_over_chatgpt,
            install_channel: s.install_channel,
            update_status: s.update_status,
            bubble_size_logical: s.settings.bubble_size_logical,
        },
        None => return,
    };

    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        append_item(menu, IDM_REFRESH, &snap.strings.refresh, MENU_ITEM_FLAGS(0));

        let Ok(freq) = CreatePopupMenu() else {
            log::error!("CreatePopupMenu(freq) failed");
            let _ = DestroyMenu(menu);
            return;
        };
        for (id, interval, label) in [
            (IDM_FREQ_1MIN, POLL_1_MIN, &snap.strings.one_minute),
            (IDM_FREQ_5MIN, POLL_5_MIN, &snap.strings.five_minutes),
            (IDM_FREQ_15MIN, POLL_15_MIN, &snap.strings.fifteen_minutes),
            (IDM_FREQ_1HOUR, POLL_1_HOUR, &snap.strings.one_hour),
        ] {
            let flags = if interval == snap.current_interval {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            };
            append_item(freq, id, label, flags);
        }
        append_submenu(menu, freq, &snap.strings.update_frequency);

        let Ok(providers) = CreatePopupMenu() else {
            log::error!("CreatePopupMenu(providers) failed");
            let _ = DestroyMenu(menu);
            return;
        };
        append_item(
            providers,
            IDM_MODEL_CLAUDE,
            &snap.strings.claude_label,
            if snap.show_claude {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            providers,
            IDM_MODEL_CHATGPT,
            &snap.strings.chatgpt_label,
            if snap.show_chatgpt {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            providers,
            IDM_MODEL_OPENCODE_GO,
            &snap.strings.opencode_go_label,
            if snap.show_opencode_go {
                MF_CHECKED | MF_GRAYED
            } else {
                MF_GRAYED
            },
        );
        append_submenu(menu, providers, &snap.strings.providers);

        let Ok(settings_menu) = CreatePopupMenu() else {
            log::error!("CreatePopupMenu(settings_menu) failed");
            let _ = DestroyMenu(menu);
            return;
        };
        append_item(
            settings_menu,
            IDM_START_WITH_WINDOWS,
            &snap.strings.start_with_windows,
            if is_startup_enabled() {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            settings_menu,
            IDM_ONLY_CHATGPT,
            &snap.strings.only_over_chatgpt,
            if snap.only_over_chatgpt {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            settings_menu,
            IDM_RESET_POSITION,
            &snap.strings.reset_position,
            MENU_ITEM_FLAGS(0),
        );

        let version_label = version_action_label(&snap);
        let version_flags = if matches!(
            snap.update_status,
            UpdateStatus::Checking | UpdateStatus::Applying
        ) {
            MF_GRAYED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        append_item(
            settings_menu,
            IDM_VERSION_ACTION,
            &version_label,
            version_flags,
        );

        let Ok(auto_update) = CreatePopupMenu() else {
            log::error!("CreatePopupMenu(auto_update) failed");
            let _ = DestroyMenu(settings_menu);
            let _ = DestroyMenu(menu);
            return;
        };
        for (id, value, label) in [
            (IDM_UPDATE_AUTO_OFF, None, &snap.strings.auto_check_disabled),
            (
                IDM_UPDATE_AUTO_HOURLY,
                Some(settings::UPDATE_CHECK_HOURLY_SECS),
                &snap.strings.auto_check_hourly,
            ),
            (
                IDM_UPDATE_AUTO_DAILY,
                Some(settings::UPDATE_CHECK_DAILY_SECS),
                &snap.strings.auto_check_daily,
            ),
            (
                IDM_UPDATE_AUTO_WEEKLY,
                Some(settings::UPDATE_CHECK_WEEKLY_SECS),
                &snap.strings.auto_check_weekly,
            ),
        ] {
            let flags = if value == snap.update_check_interval_secs {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            };
            append_item(auto_update, id, label, flags);
        }
        append_submenu(settings_menu, auto_update, &snap.strings.auto_update_check);
        append_submenu(menu, settings_menu, &snap.strings.settings);

        let Ok(controls) = CreatePopupMenu() else {
            log::error!("CreatePopupMenu(controls) failed");
            let _ = DestroyMenu(menu);
            return;
        };
        append_item(
            controls,
            IDM_SIZE_SMALLER,
            &snap.strings.size_smaller,
            if snap.bubble_size_logical <= bubble::MIN_BUBBLE_SIZE {
                MF_GRAYED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            controls,
            IDM_SIZE_LARGER,
            &snap.strings.size_larger,
            if snap.bubble_size_logical >= bubble::MAX_BUBBLE_SIZE {
                MF_GRAYED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        append_item(
            controls,
            IDM_RESET_SIZE,
            &snap.strings.reset_size,
            if snap.bubble_size_logical == bubble::DEFAULT_BUBBLE_SIZE {
                MF_GRAYED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        let _ = AppendMenuW(controls, MF_SEPARATOR, 0, PCWSTR::null());
        append_item(controls, 0, &snap.strings.control_left_click, MF_GRAYED);
        append_item(controls, 0, &snap.strings.control_right_click, MF_GRAYED);
        append_item(controls, 0, &snap.strings.control_drag, MF_GRAYED);
        append_item(controls, 0, &snap.strings.control_ctrl_wheel, MF_GRAYED);
        append_item(controls, 0, &snap.strings.control_tray_click, MF_GRAYED);
        append_submenu(menu, controls, &snap.strings.controls);

        append_item(
            menu,
            tray::IDM_TOGGLE_WIDGET,
            &snap.strings.show_widget,
            if snap.widget_visible {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            },
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        append_item(menu, IDM_RESTART, &snap.strings.restart, MENU_ITEM_FLAGS(0));
        append_item(menu, IDM_EXIT, &snap.strings.exit, MENU_ITEM_FLAGS(0));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(owner_hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, owner_hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn append_item(menu: HMENU, id: u16, label: &str, flags: MENU_ITEM_FLAGS) {
    let w = os::to_utf16_nul(label);
    unsafe {
        let _ = AppendMenuW(menu, flags, id as usize, PCWSTR::from_raw(w.as_ptr()));
    }
}

fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) {
    let w = os::to_utf16_nul(label);
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            submenu.0 as usize,
            PCWSTR::from_raw(w.as_ptr()),
        );
    }
}

fn version_action_label(snap: &ContextMenuSnapshot) -> String {
    let base = match snap.update_status {
        UpdateStatus::Idle => snap.strings.check_for_updates.clone(),
        UpdateStatus::Checking => snap.strings.checking_for_updates.clone(),
        UpdateStatus::UpToDate => snap.strings.up_to_date.clone(),
        UpdateStatus::Available => snap.strings.update_available.clone(),
        UpdateStatus::Applying => snap.strings.applying_update.clone(),
        UpdateStatus::Failed => snap.strings.update_failed.clone(),
    };
    // Append the running binary's version so the user can see what
    // they are on without opening an About dialog. Using middle-dot as
    // the separator matches the bubble's countdown formatting.
    let with_version = format!("{base} \u{00b7} v{}", env!("CARGO_PKG_VERSION"));
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

fn version_action() {
    enum Act {
        Apply(update::Release, InstallChannel),
        Check(SendHwnd),
    }
    let act = match lock_state().as_ref() {
        Some(s) => match (s.update_status, s.update_release.as_ref()) {
            (UpdateStatus::Available, Some(r)) => Act::Apply(r.clone(), s.install_channel),
            _ => Act::Check(s.msg_hwnd),
        },
        None => return,
    };
    match act {
        Act::Apply(release, channel) => {
            // Set the Applying status synchronously so the menu reflects
            // it immediately, then move the (potentially several-second)
            // download to a worker thread. Holding the UI thread here
            // would freeze paints and menus until the download finished.
            let (http, msg_hwnd) = {
                let mut guard = lock_state();
                let Some(s) = guard.as_mut() else {
                    return;
                };
                s.update_status = UpdateStatus::Applying;
                (s.http.clone(), s.msg_hwnd)
            };
            std::thread::spawn(move || {
                let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = match channel {
                    InstallChannel::Winget => {
                        // Winget channel is reserved for future use; until a
                        // winget package ships, this branch is unreachable.
                        Err("winget channel not supported yet".into())
                    }
                    InstallChannel::Portable => {
                        update::install::begin(&http, &release).map_err(|e| e.into())
                    }
                };
                match result {
                    Ok(()) => unsafe {
                        let _ = PostMessageW(
                            msg_hwnd.to_hwnd(),
                            WM_APP_UPDATE_APPLIED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    },
                    Err(e) => {
                        log::error!("update apply failed: {e}");
                        if let Some(s) = lock_state().as_mut() {
                            s.update_status = UpdateStatus::Failed;
                        }
                    }
                }
            });
        }
        Act::Check(hwnd) => begin_update_check(hwnd.to_hwnd()),
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
    if let Some(s) = lock_state().as_mut() {
        s.update_status = UpdateStatus::Checking;
    }
    let send_hwnd = SendHwnd::from_hwnd(hwnd);
    std::thread::spawn(move || {
        let result = match net::Client::new(HTTP_USER_AGENT) {
            Ok(c) => update::release::fetch_latest(&c),
            Err(e) => Err(update::Error::Network(e)),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let snap_opt = {
            let mut s = lock_state();
            s.as_mut().map(|s| {
                s.settings.last_update_check_unix = Some(now);
                match result {
                    Ok(CheckOutcome::UpToDate) => {
                        s.update_status = UpdateStatus::UpToDate;
                        s.update_release = None;
                    }
                    Ok(CheckOutcome::Available(r)) => {
                        s.update_status = UpdateStatus::Available;
                        s.update_release = Some(r);
                    }
                    Err(_) => {
                        s.update_status = UpdateStatus::Failed;
                    }
                }
                (s.settings.clone(), s.settings.update_check_interval_secs)
            })
        };
        let next_interval = snap_opt.as_ref().and_then(|(_, iv)| *iv);
        if let Some((snap, _)) = snap_opt {
            settings::save(&snap);
        }
        if let Some(interval) = next_interval {
            let ms = (interval.saturating_mul(1000).min(u32::MAX as u64)) as u32;
            unsafe {
                SetTimer(send_hwnd.to_hwnd(), TIMER_UPDATE_CHECK, ms, None);
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
