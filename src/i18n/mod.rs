// Portuguese UI strings and formatting helpers.

use std::time::{Duration, SystemTime};

/// The strings every UI module needs.
#[derive(Clone, Debug)]
pub struct LocaleStrings {
    pub window_title: String,
    pub help_body: String,
    pub history_refresh_note: String,
    pub local_refresh_note: String,
    pub install_update: String,
    pub updates_menu: String,
    pub appearance: String,
    pub refresh: String,
    pub update_frequency: String,
    pub one_minute: String,
    pub five_minutes: String,
    pub fifteen_minutes: String,
    pub one_hour: String,
    pub providers: String,
    pub claude_label: String,
    pub chatgpt_label: String,
    pub opencode_go_label: String,
    pub settings: String,
    pub start_with_windows: String,
    pub reset_position: String,
    pub size_smaller: String,
    pub size_larger: String,
    pub reset_size: String,
    pub controls: String,
    pub control_left_click: String,
    pub control_right_click: String,
    pub control_drag: String,
    pub control_ctrl_wheel: String,
    pub control_tray_click: String,
    pub tray_left_click: String,
    pub check_for_updates: String,
    pub checking_for_updates: String,
    pub up_to_date: String,
    pub update_failed: String,
    pub applying_update: String,
    pub update_available: String,
    pub update_via_winget: String,
    pub auto_update_check: String,
    pub auto_check_disabled: String,
    pub auto_check_hourly: String,
    pub auto_check_daily: String,
    pub auto_check_weekly: String,
    pub exit: String,
    pub restart: String,
    pub show_widget: String,
    pub only_over_chatgpt: String,
    pub used_word: String,
    pub remaining_word: String,
    pub reset_prefix: String,
    pub tokens_prefix: String,
    pub budget_prefix: String,
    pub pace_prefix: String,
    pub tokens_today_prefix: String,
    pub session_window: String,
    pub weekly_window: String,
    pub now: String,
    pub day_suffix: String,
    pub hour_suffix: String,
    pub minute_suffix: String,
    pub second_suffix: String,
    pub token_expired_title: String,
    pub token_expired_body: String,
    pub chatgpt_token_expired_title: String,
    pub chatgpt_token_expired_body: String,
    /// Body text for "your usage just crossed 80% of the 5h limit". The
    /// title is composed from the provider label + percent.
    pub threshold_80_body: String,
    /// Body text for the 95% threshold balloon.
    pub threshold_95_body: String,
    /// Title for the tray balloon shown on first launch after an auto-update.
    pub update_applied_title: String,
    /// Prefix for the tray balloon body. Call site appends the version.
    pub update_applied_body: String,
    /// Prefix for the rollback-failed MessageBox body. Call site appends
    /// the backup path and a separator with the expected target filename.
    pub update_rollback_failed_body: String,
}

pub struct I18n {
    strings: LocaleStrings,
}

impl I18n {
    pub fn load() -> Self {
        Self {
            strings: portuguese_strings(),
        }
    }

    pub fn strings(&self) -> &LocaleStrings {
        &self.strings
    }
}

fn portuguese_strings() -> LocaleStrings {
    LocaleStrings {
        window_title: "Consumo de IA".into(),
        help_body: "COMO USAR\nClique esquerdo no widget: ver detalhes.\nClique direito: abrir o menu.\nArraste o widget para mudar de lugar.\nCtrl + roda do mouse: aumentar ou diminuir.\nClique no ícone perto do relógio para mostrar ou ocultar.\n\nO QUE CADA NÚMERO SIGNIFICA\nLimites: cada janela da conta tem sua própria porcentagem disponível e prazo de renovação. Se houver cotas de 5 horas e 7 dias, ambas aparecem. Se vier só uma, só ela será exibida. O ícone perto do relógio destaca a cota com maior porcentagem usada.\nHoje neste PC (Codex): tokens registrados neste computador desde a meia-noite local; leitura a cada 5 segundos. Não inclui outros computadores nem sessões remotas sem registros neste PC.\nMédia de 7 dias da conta: média dos sete dias completos anteriores, sem incluir hoje. Se faltar algum dia, aparece como indisponível.\n\nAJUSTES\nO intervalo da consulta da cota não altera a leitura dos tokens locais.\nMostrar só sobre a janela do Codex: o widget aparece quando essa janela está em primeiro plano.\nBuscar novas versões automaticamente apenas procura atualizações; a instalação é uma ação separada.".into(),
        history_refresh_note: "Média da conta: consulta a cada 1 minuto".into(),
        local_refresh_note: "Tokens deste PC: leitura a cada 5 segundos".into(),
        install_update: "Instalar a nova versão".into(),
        updates_menu: "Atualizações do programa".into(),
        appearance: "Exibição e tamanho".into(),
        refresh: "Consultar cota agora".into(),
        update_frequency: "Intervalo da consulta da cota".into(),
        one_minute: "1 minuto".into(),
        five_minutes: "5 minutos".into(),
        fifteen_minutes: "15 minutos".into(),
        one_hour: "1 hora".into(),
        providers: "Serviços exibidos".into(),
        claude_label: "Claude Code".into(),
        chatgpt_label: "Codex".into(),
        opencode_go_label: "OpenCode Go".into(),
        settings: "Configurações".into(),
        start_with_windows: "Iniciar com o Windows".into(),
        reset_position: "Voltar à posição padrão".into(),
        size_smaller: "Diminuir widget".into(),
        size_larger: "Aumentar widget".into(),
        reset_size: "Voltar ao tamanho padrão".into(),
        controls: "Como usar".into(),
        control_left_click: "Clique esquerdo: ver detalhes do consumo".into(),
        control_right_click: "Clique direito: abrir este menu".into(),
        control_drag: "Arrastar: mover o widget pela tela".into(),
        control_ctrl_wheel: "Ctrl + roda do mouse: ajustar o tamanho".into(),
        control_tray_click: "Ícone perto do relógio: mostrar ou ocultar".into(),
        tray_left_click: "Clique: mostrar ou ocultar".into(),
        check_for_updates: "Buscar nova atualização".into(),
        checking_for_updates: "Buscando nova atualização…".into(),
        up_to_date: "Você já está na versão mais recente".into(),
        update_failed: "Falha na atualização; tente novamente".into(),
        applying_update: "Instalando atualização…".into(),
        update_available: "Nova versão disponível para instalar".into(),
        update_via_winget: "pelo WinGet".into(),
        auto_update_check: "Buscar novas versões automaticamente".into(),
        auto_check_disabled: "Não buscar automaticamente".into(),
        auto_check_hourly: "A cada hora".into(),
        auto_check_daily: "Uma vez por dia".into(),
        auto_check_weekly: "Uma vez por semana".into(),
        exit: "Sair do widget".into(),
        restart: "Reiniciar widget".into(),
        show_widget: "Mostrar widget".into(),
        only_over_chatgpt: "Mostrar só sobre a janela do Codex".into(),
        used_word: "usados".into(),
        remaining_word: "disponíveis".into(),
        reset_prefix: "Cota renova em".into(),
        tokens_prefix: "Tokens registrados neste PC".into(),
        budget_prefix: "Para durar até renovar a cota semanal".into(),
        pace_prefix: "Projeção semanal".into(),
        tokens_today_prefix: "Hoje neste PC".into(),
        session_window: "5h".into(),
        weekly_window: "7d".into(),
        now: "agora".into(),
        day_suffix: "d".into(),
        hour_suffix: "h".into(),
        minute_suffix: "min".into(),
        second_suffix: "s".into(),
        token_expired_title: "Entre novamente no Claude Code".into(),
        token_expired_body: "A sessão expirou. Entre no Claude Code para voltar a consultar a cota.".into(),
        chatgpt_token_expired_title: "Entre novamente no Codex".into(),
        chatgpt_token_expired_body: "A sessão expirou. Entre no Codex para atualizar os dados da conta.".into(),
        threshold_80_body: "Você já usou pelo menos 80% da cota de uso. Confira o tempo até a renovação.".into(),
        threshold_95_body: "Você já usou pelo menos 95% da cota de uso. O limite está próximo.".into(),
        update_applied_title: "Atualização concluída".into(),
        update_applied_body: "Versão instalada: v".into(),
        update_rollback_failed_body: "Não foi possível concluir a atualização. O arquivo da versão anterior está salvo em: ".into(),
    }
}

// ---------- Free-function helpers ----------

/// Format a `usage::Window` percentage + countdown as `"73% · 2h"`-style text.
/// Returns just the percentage when no reset time is available.
pub fn format_window(window: &crate::usage::Window, strings: &LocaleStrings) -> String {
    let pct = format!("{:.0}%", window.utilization);
    let cd = format_countdown(window.resets_at, strings);
    if cd.is_empty() {
        pct
    } else {
        format!("{pct} \u{00b7} {cd}")
    }
}

/// Fork (Rafael): explicit "used x remaining" window text, e.g.
/// "3% usada · 97% resta · 6d". Used by the expanded panel and tray
/// tooltip so it is always clear what was consumed vs. what is left.
pub fn format_window_remaining(window: &crate::usage::Window, strings: &LocaleStrings) -> String {
    let used = window.utilization.clamp(0.0, 100.0);
    let left = (100.0 - used).clamp(0.0, 100.0);
    let base = format!(
        "{:.0}% {} · {:.0}% {}",
        used, strings.used_word, left, strings.remaining_word
    );
    let cd = format_countdown(window.resets_at, strings);
    if cd.is_empty() {
        base
    } else {
        format!("{base} · {cd}")
    }
}

/// Fork (Rafael): precise two-unit countdown in full PT words —
/// "6 dias e 19 horas", "5 horas e 12 minutos", "3 minutos e 20 segundos".
/// Singular/plural handled; zero parts omitted ("6 dias", not "6 dias e 0 horas").
pub fn format_precise_countdown(resets_at: Option<SystemTime>, strings: &LocaleStrings) -> String {
    let Some(reset) = resets_at else {
        return String::new();
    };
    let secs = match reset.duration_since(SystemTime::now()) {
        Ok(d) => d.as_secs(),
        Err(_) => return strings.now.clone(),
    };
    let d = secs / 86_400;
    let h = secs % 86_400 / 3_600;
    let m = secs % 3_600 / 60;
    let s = secs % 60;
    fn qty(n: u64, one: &str, many: &str) -> String {
        format!("{n} {}", if n == 1 { one } else { many })
    }
    if d >= 1 {
        if h >= 1 {
            format!("{} e {}", qty(d, "dia", "dias"), qty(h, "hora", "horas"))
        } else {
            qty(d, "dia", "dias")
        }
    } else if h >= 1 {
        if m >= 1 {
            format!("{} e {}", qty(h, "hora", "horas"), qty(m, "minuto", "minutos"))
        } else {
            qty(h, "hora", "horas")
        }
    } else if m >= 1 {
        if s >= 1 {
            format!(
                "{} e {}",
                qty(m, "minuto", "minutos"),
                qty(s, "segundo", "segundos")
            )
        } else {
            qty(m, "minuto", "minutos")
        }
    } else {
        qty(s, "segundo", "segundos")
    }
}

/// Countdown only — used by the bubble, which renders the percent inside the
/// bar fill and only needs the time-to-reset on the right.
pub fn format_countdown(resets_at: Option<SystemTime>, strings: &LocaleStrings) -> String {
    let Some(reset) = resets_at else {
        return String::new();
    };
    let remaining = match reset.duration_since(SystemTime::now()) {
        Ok(d) => d,
        Err(_) => return strings.now.clone(),
    };
    format_countdown_secs(remaining.as_secs(), strings)
}

fn format_countdown_secs(total_secs: u64, strings: &LocaleStrings) -> String {
    let days = total_secs / 86_400;
    let hours = total_secs / 3_600;
    let mins = total_secs / 60;
    if days >= 1 {
        format!("{days}{}", strings.day_suffix)
    } else if hours >= 1 {
        format!("{hours}{}", strings.hour_suffix)
    } else if mins >= 1 {
        format!("{mins}{}", strings.minute_suffix)
    } else {
        format!("{total_secs}{}", strings.second_suffix)
    }
}

/// How long before `format_window`'s string would change.
/// Used by the countdown timer to refresh exactly when needed.
pub fn time_until_display_change(resets_at: Option<SystemTime>) -> Option<Duration> {
    let reset = resets_at?;
    let remaining = reset.duration_since(SystemTime::now()).ok()?;
    let secs = remaining.as_secs();
    let bucket_start = if secs / 86_400 >= 1 {
        (secs / 86_400) * 86_400
    } else if secs / 3_600 >= 1 {
        (secs / 3_600) * 3_600
    } else if secs / 60 >= 1 {
        (secs / 60) * 60
    } else {
        secs
    };
    Some(Duration::from_secs(secs.saturating_sub(bucket_start) + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portuguese_strings_include_required_menu_labels() {
        let strings = portuguese_strings();
        for (name, value) in [
            ("refresh", strings.refresh.as_str()),
            ("providers", strings.providers.as_str()),
            ("settings", strings.settings.as_str()),
            ("size_smaller", strings.size_smaller.as_str()),
            ("size_larger", strings.size_larger.as_str()),
            ("reset_size", strings.reset_size.as_str()),
            ("controls", strings.controls.as_str()),
            ("control_left_click", strings.control_left_click.as_str()),
            ("control_right_click", strings.control_right_click.as_str()),
            ("control_drag", strings.control_drag.as_str()),
            ("control_ctrl_wheel", strings.control_ctrl_wheel.as_str()),
            ("control_tray_click", strings.control_tray_click.as_str()),
            ("tray_left_click", strings.tray_left_click.as_str()),
        ] {
            assert!(!value.trim().is_empty(), "empty string: {name}");
        }
    }
}
