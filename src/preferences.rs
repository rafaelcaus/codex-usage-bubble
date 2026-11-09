//! Native settings window. Draft changes are applied only by Save; Cancel
//! never changes the live widget. API secrets are not part of Settings JSON.
use crate::external_usage::{ApiConfig, Source};
use crate::settings::Settings;
use crate::usage::ProviderId;
use std::sync::{Mutex, OnceLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetScrollInfo, EM_SETLIMITTEXT};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "CodexClaudeWidgetSettings";
const SAVE: u16 = 1;
const CANCEL: u16 = 2;
const UPDATE: u16 = 3;
const API_HELP: u16 = 4;
const NAV: u16 = 1000;
const PROVIDER: u16 = 300;
const SOURCE: u16 = 301;
const URL: u16 = 302;
const KEY: u16 = 303;
const USED: u16 = 304;
const LIMIT: u16 = 305;
const RESET: u16 = 306;
const PERIOD: u16 = 307;
const UNITS: u16 = 308;
const HEADER: u16 = 309;
const SIZES: [i32; 6] = [80, 110, 130, 160, 200, 240];
const POLLS: [u32; 4] = [60000, 300000, 900000, 3600000];
const UPDATES: [Option<u64>; 4] = [Some(86400), Some(3600), Some(604800), None];
const PERIODS: [u64; 4] = [0, 86400, 604800, 2592000];
const UNITS_VALUES: [&str; 3] = ["tokens", "créditos", "requisições"];
const HEADERS: [&str; 3] = ["bearer", "x-api-key", "none"];

#[derive(Clone, Copy)]
struct Control {
    hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    page: i32,
    kind: u8,
}
struct Dialog {
    hwnd: HWND,
    controls: Vec<Control>,
    fonts: [HFONT; 3],
    dpi: u32,
    settings: Settings,
    startup: bool,
    page: i32,
    offset: i32,
    api_index: usize,
    api: [ApiConfig; 2],
    keys: [String; 2],
    apply: fn(Settings, bool),
    action: fn(u32, HWND),
}
unsafe impl Send for Dialog {}
fn state() -> &'static Mutex<Option<Dialog>> {
    static S: OnceLock<Mutex<Option<Dialog>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}
fn lock() -> std::sync::MutexGuard<'static, Option<Dialog>> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}
fn wide(s: &str) -> Vec<u16> {
    crate::os::to_utf16_nul(s)
}
fn scale(n: i32, dpi: u32) -> i32 {
    crate::os::dpi::scale(n, dpi)
}
fn font(height: i32, weight: i32) -> HFONT {
    let name = wide("Segoe UI");
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            (FF_SWISS.0 | DEFAULT_PITCH.0) as u32,
            PCWSTR(name.as_ptr()),
        )
    }
}
pub fn route_message(msg: &MSG) -> bool {
    let hwnd = lock().as_ref().map(|s| s.hwnd);
    hwnd.is_some_and(|h| unsafe { IsDialogMessageW(h, msg).as_bool() })
}
pub fn show(settings: Settings, startup: bool, apply: fn(Settings, bool), action: fn(u32, HWND)) {
    let existing = lock().as_ref().map(|s| s.hwnd);
    if let Some(hwnd) = existing {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Some(hwnd) = create(settings, startup, apply, action) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
fn create(
    settings: Settings,
    startup: bool,
    apply: fn(Settings, bool),
    action: fn(u32, HWND),
) -> Option<HWND> {
    static REGISTER: OnceLock<()> = OnceLock::new();
    REGISTER.get_or_init(|| unsafe {
        let class = wide(CLASS);
        let module = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: HINSTANCE(module.0),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
    unsafe {
        let dpi = crate::os::dpi::for_system().max(96);
        let mut work = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let _ = GetMonitorInfoW(monitor, &mut work);
        let w = scale(700, dpi);
        let h = scale(720, dpi).min((work.rcWork.bottom - work.rcWork.top - 24).max(360));
        let x = work.rcWork.left + ((work.rcWork.right - work.rcWork.left - w) / 2).max(0);
        let y = work.rcWork.top + ((work.rcWork.bottom - work.rcWork.top - h) / 2).max(0);
        let class = wide(CLASS);
        let title = wide("Configurações · CODEX + CLAUDE");
        let module = GetModuleHandleW(PCWSTR::null()).ok()?;
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VSCROLL,
            x,
            y,
            w,
            h,
            None,
            None,
            HINSTANCE(module.0),
            None,
        )
        .ok()?;
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&round) as u32,
        );
        let api = [settings.codex_api.clone(), settings.claude_api.clone()];
        *lock() = Some(Dialog {
            hwnd,
            controls: Vec::new(),
            fonts: [
                font(scale(14, dpi), 400),
                font(scale(16, dpi), 600),
                font(scale(25, dpi), 700),
            ],
            dpi,
            settings,
            startup,
            page: 0,
            offset: 0,
            api_index: 0,
            api,
            keys: Default::default(),
            apply,
            action,
        });
        build(hwnd);
        load_api(hwnd, 0);
        layout(hwnd);
        Some(hwnd)
    }
}
fn child(
    hwnd: HWND,
    class: &str,
    text: &str,
    id: u16,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    page: i32,
    kind: u8,
    font_index: usize,
) -> HWND {
    let (dpi, f) = lock()
        .as_ref()
        .map(|s| (s.dpi, s.fonts[font_index]))
        .unwrap();
    unsafe {
        let c = wide(class);
        let t = wide(text);
        let module = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let control = CreateWindowExW(
            if class == "EDIT" {
                WS_EX_CLIENTEDGE
            } else {
                WINDOW_EX_STYLE(0)
            },
            PCWSTR(c.as_ptr()),
            PCWSTR(t.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            scale(x, dpi),
            scale(y, dpi),
            scale(w, dpi),
            scale(if kind == 3 { 180 } else { h }, dpi),
            hwnd,
            HMENU(id as usize as *mut _),
            HINSTANCE(module.0),
            None,
        )
        .unwrap_or_default();
        SendMessageW(control, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
        if let Some(s) = lock().as_mut() {
            s.controls.push(Control {
                hwnd: control,
                x,
                y,
                w,
                h,
                page,
                kind,
            });
        }
        control
    }
}
fn label(h: HWND, t: &str, x: i32, y: i32, w: i32, page: i32, bold: bool) {
    child(
        h,
        "STATIC",
        t,
        0,
        WINDOW_STYLE(0),
        x,
        y,
        w,
        24,
        page,
        0,
        if bold { 1 } else { 0 },
    );
}
fn check(h: HWND, t: &str, id: u16, x: i32, y: i32, page: i32, on: bool) {
    let c = child(
        h,
        "BUTTON",
        t,
        id,
        WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        x,
        y,
        440,
        26,
        page,
        0,
        0,
    );
    unsafe {
        SendMessageW(c, BM_SETCHECK, WPARAM(usize::from(on)), LPARAM(0));
    }
}
fn combo(h: HWND, id: u16, values: &[&str], index: usize, x: i32, y: i32, w: i32, page: i32) {
    let c = child(
        h,
        "COMBOBOX",
        "",
        id,
        WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
        x,
        y,
        w,
        26,
        page,
        3,
        0,
    );
    unsafe {
        for v in values {
            let text = wide(v);
            SendMessageW(c, CB_ADDSTRING, WPARAM(0), LPARAM(text.as_ptr() as isize));
        }
        SendMessageW(c, CB_SETCURSEL, WPARAM(index), LPARAM(0));
    }
}
fn edit(h: HWND, id: u16, x: i32, y: i32, w: i32, password: bool) {
    let c = child(
        h,
        "EDIT",
        "",
        id,
        WS_TABSTOP
            | WINDOW_STYLE(ES_AUTOHSCROLL as u32 | if password { ES_PASSWORD as u32 } else { 0 }),
        x,
        y,
        w,
        26,
        2,
        0,
        0,
    );
    unsafe {
        SendMessageW(
            c,
            EM_SETLIMITTEXT,
            WPARAM(if password { 2560 } else { 2048 }),
            LPARAM(0),
        );
    }
}
fn build(h: HWND) {
    let (s, startup) = lock()
        .as_ref()
        .map(|s| (s.settings.clone(), s.startup))
        .unwrap();
    child(
        h,
        "STATIC",
        "Seu widget, do seu jeito",
        0,
        WINDOW_STYLE(0),
        22,
        16,
        600,
        35,
        -1,
        1,
        2,
    );
    child(
        h,
        "STATIC",
        "Escolha o que aparece e de onde vêm os limites.",
        0,
        WINDOW_STYLE(0),
        24,
        53,
        620,
        22,
        -1,
        1,
        0,
    );
    for (i, t) in ["Geral", "Informações", "Contas e APIs"].iter().enumerate() {
        child(
            h,
            "BUTTON",
            t,
            NAV + i as u16,
            WS_TABSTOP,
            20,
            98 + i as i32 * 44,
            155,
            34,
            -1,
            1,
            0,
        );
    }
    label(h, "Serviços", 202, 96, 440, 0, true);
    check(h, "Mostrar CODEX", 110, 202, 128, 0, s.show_codex);
    check(h, "Mostrar CLAUDE", 111, 202, 158, 0, s.show_claude_code);
    label(h, "Aparência e início", 202, 204, 440, 0, true);
    check(h, "Widget visível", 112, 202, 236, 0, s.widget_visible);
    check(
        h,
        "Modo compacto (somente anel)",
        113,
        202,
        266,
        0,
        s.compact_mode,
    );
    check(
        h,
        "Aparecer somente sobre o ChatGPT",
        114,
        202,
        296,
        0,
        s.only_over_chatgpt,
    );
    check(h, "Iniciar com o Windows", 115, 202, 326, 0, startup);
    label(h, "Tamanho", 202, 374, 155, 0, false);
    combo(
        h,
        160,
        &[
            "Pequeno · 80",
            "110",
            "Padrão · 130",
            "160",
            "200",
            "Grande · 240",
        ],
        SIZES
            .iter()
            .position(|n| *n == s.bubble_size_logical)
            .unwrap_or(2),
        368,
        370,
        260,
        0,
    );
    label(h, "Consultar consumo", 202, 419, 155, 0, false);
    combo(
        h,
        161,
        &[
            "A cada minuto",
            "A cada 5 minutos",
            "A cada 15 minutos",
            "A cada hora",
        ],
        POLLS
            .iter()
            .position(|n| *n == s.poll_interval_ms)
            .unwrap_or(0),
        368,
        415,
        260,
        0,
    );
    label(h, "Atualizar aplicativo", 202, 464, 160, 0, false);
    combo(
        h,
        162,
        &[
            "Automaticamente · diário",
            "Automaticamente · a cada hora",
            "Automaticamente · semanal",
            "Somente manual",
        ],
        UPDATES
            .iter()
            .position(|n| *n == s.update_check_interval_secs)
            .unwrap_or(0),
        368,
        460,
        260,
        0,
    );
    child(
        h,
        "BUTTON",
        "Buscar atualização agora",
        UPDATE,
        WS_TABSTOP,
        202,
        510,
        235,
        32,
        0,
        0,
        0,
    );
    label(h, "Informações no cartão", 202, 96, 440, 1, true);
    for (i, t) in [
        "Quando o limite renova",
        "Segundo limite (se o serviço informar)",
        "Uso de hoje / consumo da fonte",
        "Média de 7 dias (assinatura Codex)",
        "Resets disponíveis (assinatura Codex)",
    ]
    .iter()
    .enumerate()
    {
        check(
            h,
            t,
            120 + i as u16,
            202,
            140 + i as i32 * 42,
            1,
            s.visible_sections & (1 << i) != 0,
        );
    }
    label(
        h,
        "As escolhas não interrompem a coleta do histórico.",
        202,
        380,
        455,
        1,
        false,
    );
    label(
        h,
        "Dados não fornecidos pelo serviço não são inventados.",
        202,
        410,
        455,
        1,
        false,
    );
    label(h, "Conta e fonte dos limites", 202, 96, 440, 2, true);
    combo(h, PROVIDER, &["CODEX", "CLAUDE"], 0, 202, 128, 160, 2);
    combo(
        h,
        SOURCE,
        &["Automático", "Assinatura do serviço", "API externa"],
        0,
        373,
        128,
        255,
        2,
    );
    label(
        h,
        "URL HTTPS de consulta de consumo (somente GET)",
        202,
        172,
        460,
        2,
        false,
    );
    edit(h, URL, 202, 198, 426, false);
    label(
        h,
        "Chave da API · vazia mantém a chave desta URL",
        202,
        238,
        460,
        2,
        false,
    );
    edit(h, KEY, 202, 264, 426, true);
    label(h, "Autenticação", 202, 306, 160, 2, false);
    combo(
        h,
        HEADER,
        &["Bearer · Authorization", "x-api-key", "Sem autenticação"],
        0,
        373,
        301,
        255,
        2,
    );
    label(h, "Caminho JSON: consumo usado", 202, 346, 455, 2, false);
    edit(h, USED, 202, 372, 426, false);
    label(h, "Caminho JSON: limite total", 202, 412, 455, 2, false);
    edit(h, LIMIT, 202, 438, 426, false);
    label(
        h,
        "Caminho JSON: data de renovação (opcional)",
        202,
        478,
        455,
        2,
        false,
    );
    edit(h, RESET, 202, 504, 426, false);
    label(h, "Período informado", 202, 548, 160, 2, false);
    combo(
        h,
        PERIOD,
        &["Não informado", "Diário", "Semanal", "30 dias"],
        0,
        373,
        544,
        255,
        2,
    );
    label(h, "Unidade da cota", 202, 590, 160, 2, false);
    combo(
        h,
        UNITS,
        &["Tokens", "Créditos", "Requisições"],
        0,
        373,
        586,
        255,
        2,
    );
    child(
        h,
        "BUTTON",
        "Como configurar uma API / Clipoos",
        API_HELP,
        WS_TABSTOP,
        202,
        634,
        340,
        32,
        2,
        0,
        0,
    );
    label(
        h,
        "Chave protegida pelo Windows e vinculada a esta URL.",
        202,
        680,
        455,
        2,
        false,
    );
    label(
        h,
        "Sem integração configurada, a API não usa a cota Free.",
        202,
        710,
        455,
        2,
        false,
    );
    child(
        h,
        "STATIC",
        "As alterações só valem ao salvar.",
        0,
        WINDOW_STYLE(0),
        22,
        18,
        300,
        24,
        -1,
        2,
        0,
    );
    child(
        h,
        "BUTTON",
        "Salvar alterações",
        SAVE,
        WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        365,
        12,
        165,
        34,
        -1,
        2,
        0,
    );
    child(
        h, "BUTTON", "Cancelar", CANCEL, WS_TABSTOP, 542, 12, 100, 34, -1, 2, 0,
    );
}
fn get(h: HWND, id: u16) -> HWND {
    unsafe { GetDlgItem(h, id as i32).unwrap_or_default() }
}
fn text(h: HWND, id: u16) -> String {
    unsafe {
        let c = get(h, id);
        let mut buf = vec![0u16; (GetWindowTextLengthW(c).max(0) as usize + 1).min(4096)];
        let n = GetWindowTextW(c, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}
fn set_text(h: HWND, id: u16, value: &str) {
    let value = wide(value);
    unsafe {
        let _ = SetWindowTextW(get(h, id), PCWSTR(value.as_ptr()));
    }
}
fn selected(h: HWND, id: u16) -> usize {
    unsafe {
        SendMessageW(get(h, id), CB_GETCURSEL, WPARAM(0), LPARAM(0))
            .0
            .max(0) as usize
    }
}
fn select(h: HWND, id: u16, index: usize) {
    unsafe {
        SendMessageW(get(h, id), CB_SETCURSEL, WPARAM(index), LPARAM(0));
    }
}
fn checked(h: HWND, id: u16) -> bool {
    unsafe { SendMessageW(get(h, id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1 }
}
fn capture_api(h: HWND) {
    let index = lock().as_ref().map(|s| s.api_index).unwrap_or(0);
    let c = ApiConfig {
        source: match selected(h, SOURCE) {
            1 => Source::Subscription,
            2 => Source::ExternalApi,
            _ => Source::Auto,
        },
        url: text(h, URL).trim().to_string(),
        used_pointer: text(h, USED).trim().to_string(),
        limit_pointer: text(h, LIMIT).trim().to_string(),
        reset_pointer: text(h, RESET).trim().to_string(),
        period_secs: PERIODS[selected(h, PERIOD).min(3)],
        units: UNITS_VALUES[selected(h, UNITS).min(2)].into(),
        header: HEADERS[selected(h, HEADER).min(2)].into(),
    };
    let key = text(h, KEY);
    if let Some(s) = lock().as_mut() {
        s.api[index] = c;
        s.keys[index] = key;
    }
}
fn load_api(h: HWND, index: usize) {
    let data = {
        let mut guard = lock();
        let Some(s) = guard.as_mut() else { return };
        s.api_index = index;
        (s.api[index].clone(), s.keys[index].clone())
    };
    let (c, key) = data;
    select(
        h,
        SOURCE,
        match c.source {
            Source::Auto => 0,
            Source::Subscription => 1,
            Source::ExternalApi => 2,
        },
    );
    for (id, value) in [
        (URL, c.url),
        (USED, c.used_pointer),
        (LIMIT, c.limit_pointer),
        (RESET, c.reset_pointer),
        (KEY, key),
    ] {
        set_text(h, id, &value);
    }
    select(
        h,
        PERIOD,
        PERIODS
            .iter()
            .position(|n| *n == c.period_secs)
            .unwrap_or(0),
    );
    select(
        h,
        UNITS,
        UNITS_VALUES.iter().position(|n| *n == c.units).unwrap_or(0),
    );
    select(
        h,
        HEADER,
        HEADERS.iter().position(|n| *n == c.header).unwrap_or(0),
    );
}
fn layout(h: HWND) {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(h, &mut client);
    }
    let (controls, dpi, page, offset, max) = {
        let mut guard = lock();
        let Some(s) = guard.as_mut() else { return };
        let bottom = scale(
            if s.page == 2 {
                752
            } else if s.page == 0 {
                558
            } else {
                450
            },
            s.dpi,
        );
        let max = (bottom - (client.bottom - scale(76, s.dpi))).max(0);
        s.offset = s.offset.clamp(0, max);
        (s.controls.clone(), s.dpi, s.page, s.offset, max)
    };
    let info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: max + client.bottom - 1,
        nPage: client.bottom.max(1) as u32,
        nPos: offset,
        ..Default::default()
    };
    unsafe {
        SetScrollInfo(h, SB_VERT, &info, true);
    }
    for c in controls {
        let y = if c.kind == 2 {
            client.bottom - scale(62 - c.y, dpi)
        } else {
            scale(c.y, dpi) - if c.kind == 1 { 0 } else { offset }
        };
        let visible = (c.page < 0 || c.page == page)
            && (c.kind == 1
                || c.kind == 2
                || (y >= scale(84, dpi) && y + scale(c.h, dpi) <= client.bottom - scale(74, dpi)));
        unsafe {
            let _ = MoveWindow(
                c.hwnd,
                scale(c.x, dpi),
                y,
                scale(c.w, dpi),
                scale(if c.kind == 3 { 180 } else { c.h }, dpi),
                true,
            );
            let _ = ShowWindow(c.hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
    }
    unsafe {
        let _ = InvalidateRect(h, None, true);
    }
}
fn error(h: HWND, message: &str) {
    let m = wide(message);
    let title = wide("Confira a configuração");
    unsafe {
        MessageBoxW(
            h,
            PCWSTR(m.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}
fn save(h: HWND) {
    capture_api(h);
    let (mut s, startup, api, keys, apply) = {
        let guard = lock();
        let Some(d) = guard.as_ref() else { return };
        (
            d.settings.clone(),
            checked(h, 115),
            d.api.clone(),
            d.keys.clone(),
            d.apply,
        )
    };
    for c in &api {
        if let Err(e) = crate::external_usage::validate(c) {
            error(h, &e);
            return;
        }
    }
    for (i,c) in api.iter().enumerate() {
        let id=if i==0 {ProviderId::ChatGpt}else{ProviderId::Claude};
        if c.source==Source::ExternalApi && c.header!="none" && keys[i].is_empty() && !crate::external_usage::has_key(id,c) {
            error(h,"Informe a chave de autenticação para esta URL ou selecione Sem autenticação."); return;
        }
    }
    for (i, c) in api.iter().enumerate() {
        if c.source == Source::ExternalApi && !keys[i].is_empty() {
            if let Err(e) = crate::external_usage::save_key(
                if i == 0 {
                    ProviderId::ChatGpt
                } else {
                    ProviderId::Claude
                },
                c,
                &keys[i],
            ) {
                error(h, &e);
                return;
            }
        }
    }
    s.show_codex = checked(h, 110);
    s.show_claude_code = checked(h, 111);
    s.widget_visible = checked(h, 112);
    s.compact_mode = checked(h, 113);
    s.only_over_chatgpt = checked(h, 114);
    s.visible_sections = (0..5).fold(0, |mask, i| {
        mask | if checked(h, 120 + i) { 1 << i } else { 0 }
    });
    s.bubble_size_logical = SIZES[selected(h, 160).min(5)];
    s.poll_interval_ms = POLLS[selected(h, 161).min(3)];
    s.update_check_interval_secs = UPDATES[selected(h, 162).min(3)];
    s.codex_api = api[0].clone();
    s.claude_api = api[1].clone();
    apply(s, startup);
    unsafe {
        let _ = DestroyWindow(h);
    }
}
unsafe extern "system" fn proc(h: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let id = (w.0 & 0xffff) as u16;
            let notification = ((w.0 >> 16) & 0xffff) as u16;
            match id {
                SAVE => save(h),
                CANCEL => {
                    let _ = DestroyWindow(h);
                }
                UPDATE => {
                    let action = lock().as_ref().map(|s| s.action);
                    if let Some(action) = action {
                        action(42, h);
                    }
                }
                API_HELP => {
                    let body=wide("A URL deve ser uma consulta GET de consumo, não uma URL de geração de respostas.\n\nClipoos documenta https://clipoos.online/usage. Os campos JSON de consumo e limite precisam ser confirmados pelo relatório do fornecedor; não são adivinhados. Exemplo de caminho JSON: /quota/used.\n\nA chave é exclusiva desta URL e fica protegida pelo Windows. Nunca cole tokens ChatGPT/Claude para autenticar um gateway.\n\nAutomático identifica configurações externas, mas não calcula limites de uma API sem integração. Escolha Assinatura se deseja consultar a conta oficial separadamente.");
                    let title = wide("Contas, APIs e gateways");
                    MessageBoxW(
                        h,
                        PCWSTR(body.as_ptr()),
                        PCWSTR(title.as_ptr()),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                NAV..=1002 => {
                    if let Some(s) = lock().as_mut() {
                        s.page = (id - NAV) as i32;
                        s.offset = 0;
                    }
                    layout(h);
                }
                PROVIDER if notification == 1 => {
                    capture_api(h);
                    load_api(h, selected(h, PROVIDER).min(1));
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_SIZE => {
            layout(h);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((w.0 >> 16) & 0xffff) as u16 as i16;
            if let Some(s) = lock().as_mut() {
                s.offset += scale(if delta < 0 { 48 } else { -48 }, s.dpi);
            }
            layout(h);
            LRESULT(0)
        }
        WM_VSCROLL => {
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            let _ = GetScrollInfo(h, SB_VERT, &mut info);
            if let Some(s) = lock().as_mut() {
                s.offset = match w.0 & 0xffff {
                    0 => s.offset - scale(32, s.dpi),
                    1 => s.offset + scale(32, s.dpi),
                    2 => s.offset - 200,
                    3 => s.offset + 200,
                    4 | 5 => info.nTrackPos,
                    6 => 0,
                    7 => info.nMax,
                    _ => s.offset,
                };
            }
            layout(h);
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let dc = HDC(w.0 as *mut _);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x003e3020));
            LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            if let Some(s) = lock().take() {
                for f in s.fonts {
                    let _ = DeleteObject(f);
                }
            }
            DefWindowProcW(h, message, w, l)
        }
        _ => DefWindowProcW(h, message, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static APPLIED: AtomicUsize = AtomicUsize::new(0);
    fn apply(s: Settings, _: bool) {
        APPLIED.store(usize::from(s.visible_sections) + 100, Ordering::SeqCst);
    }
    fn action(_: u32, _: HWND) {}
    #[test]
    fn native_drafts_save_and_cancel_are_distinct() {
        let h = create(Settings::default(), false, apply, action).unwrap();
        unsafe {
            SendMessageW(get(h, 120), BM_SETCHECK, WPARAM(0), LPARAM(0));
            SendMessageW(h, WM_COMMAND, WPARAM(CANCEL as usize), LPARAM(0));
        }
        assert_eq!(APPLIED.load(Ordering::SeqCst), 0);
        let h = create(Settings::default(), false, apply, action).unwrap();
        unsafe {
            SendMessageW(get(h, 120), BM_SETCHECK, WPARAM(0), LPARAM(0));
            SendMessageW(h, WM_COMMAND, WPARAM(SAVE as usize), LPARAM(0));
        }
        assert_eq!(APPLIED.load(Ordering::SeqCst), 130);
        assert!(lock().is_none());
    }
}
