//! Explicit, read-only quota sources. No automatic reuse of subscription tokens.
use crate::usage::{Error, ProviderId, UsageWindows, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::*;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Auto,
    Subscription,
    ExternalApi,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiConfig {
    pub source: Source,
    pub url: String,
    pub used_pointer: String,
    pub limit_pointer: String,
    pub reset_pointer: String,
    pub period_secs: u64,
    pub units: String,
    pub header: String,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            source: Source::Auto,
            url: String::new(),
            used_pointer: String::new(),
            limit_pointer: String::new(),
            reset_pointer: String::new(),
            period_secs: 0,
            units: "tokens".into(),
            header: "bearer".into(),
        }
    }
}
pub fn config(s: &crate::settings::Settings, id: ProviderId) -> &ApiConfig {
    if id == ProviderId::Claude {
        &s.claude_api
    } else {
        &s.codex_api
    }
}
pub fn validate(c: &ApiConfig) -> Result<(), String> {
    if c.source != Source::ExternalApi {
        return Ok(());
    }
    validate_url(&c.url)?;
    for p in [&c.used_pointer, &c.limit_pointer] {
        if !p.starts_with('/') || p.len() < 2 || p.len() > 256 {
            return Err("Informe os caminhos JSON de consumo e limite, começando com /.".into());
        }
    }
    if c.used_pointer == c.limit_pointer {
        return Err("Consumo e limite precisam ser campos diferentes.".into());
    }
    if !c.reset_pointer.is_empty() && !c.reset_pointer.starts_with('/') {
        return Err("O caminho da renovação deve começar com /.".into());
    }
    if !["bearer", "x-api-key", "none"].contains(&c.header.as_str()) {
        return Err("Autenticação inválida.".into());
    }
    if !["tokens", "créditos", "requisições"].contains(&c.units.as_str()) {
        return Err("Unidade inválida.".into());
    }
    if ![0, 86400, 604800, 2592000].contains(&c.period_secs) {
        return Err("Período inválido.".into());
    }
    Ok(())
}
pub fn validate_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("Use uma URL HTTPS de consulta de consumo.".into());
    };
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty()
        || host.contains('@')
        || url.contains(['?', '#', '\r', '\n'])
        || url.chars().any(char::is_whitespace)
        || url.len() > 2048
    {
        return Err("URL inválida. Não coloque chave, usuário, senha ou parâmetros na URL.".into());
    }
    Ok(())
}
fn target(id: ProviderId, c: &ApiConfig) -> String {
    let hash = Sha256::digest(c.url.as_bytes());
    format!("CodexClaudeWidget/External/{}/{:x}", id.slug(), hash)
}
pub fn save_key(id: ProviderId, c: &ApiConfig, key: &str) -> Result<(), String> {
    validate_url(&c.url)?;
    if key.is_empty() || key.len() > 2560 || key.chars().any(char::is_whitespace) {
        return Err("Chave vazia, grande demais ou inválida.".into());
    }
    let mut name = crate::os::to_utf16_nul(&target(id, c));
    let mut bytes = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    let result = unsafe { CredWriteW(&credential, 0) };
    bytes.fill(0);
    result.map_err(|_| "Não foi possível guardar a chave nas credenciais do Windows.".into())
}
pub fn has_key(id:ProviderId,c:&ApiConfig)->bool { key(id,c).is_some() }
fn key(id: ProviderId, c: &ApiConfig) -> Option<String> {
    let name = crate::os::to_utf16_nul(&target(id, c));
    let mut ptr = std::ptr::null_mut();
    unsafe {
        CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, 0, &mut ptr).ok()?;
        if ptr.is_null() {
            return None;
        }
        let v = &*ptr;
        let result = if v.CredentialBlobSize > 0
            && v.CredentialBlobSize <= 2560
            && !v.CredentialBlob.is_null()
        {
            String::from_utf8(
                std::slice::from_raw_parts(v.CredentialBlob, v.CredentialBlobSize as usize)
                    .to_vec(),
            )
            .ok()
        } else {
            None
        };
        CredFree(ptr.cast());
        result
    }
}
#[derive(Clone, Default)]
pub struct Reading {
    pub used: f64,
    pub units: String,
    pub valid: bool,
}
#[derive(Default)]
struct State {
    external: [bool; 2],
    readings: [Reading; 2],
}
fn cell() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}
fn index(id: ProviderId) -> usize {
    if id == ProviderId::Claude {
        1
    } else {
        0
    }
}
pub fn is_external(id: ProviderId) -> bool {
    cell().lock().unwrap_or_else(|e| e.into_inner()).external[index(id)]
}
pub fn latest(id: ProviderId) -> Reading {
    cell().lock().unwrap_or_else(|e| e.into_inner()).readings[index(id)].clone()
}
pub fn configure(s: &crate::settings::Settings) {
    let flags = [
        select_external(&s.codex_api, ProviderId::ChatGpt),
        select_external(&s.claude_api, ProviderId::Claude),
    ];
    let mut state = cell().lock().unwrap_or_else(|e| e.into_inner());
    state.external = flags;
}
fn select_external(c: &ApiConfig, id: ProviderId) -> bool {
    match c.source {
        Source::ExternalApi => true,
        Source::Subscription => false,
        Source::Auto => {
            if id == ProviderId::Claude {
                let settings =
                    crate::creds::local_fs::config_root().map(|p| p.join("settings.json"));
                let value = settings
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
                let base = std::env::var("ANTHROPIC_BASE_URL").ok().or_else(|| {
                    value
                        .as_ref()?
                        .pointer("/env/ANTHROPIC_BASE_URL")?
                        .as_str()
                        .map(str::to_owned)
                });
                base.is_some_and(|u| {
                    !u.trim().is_empty() && !official_origin(&u, "api.anthropic.com")
                }) || [
                    "CLAUDE_CODE_USE_BEDROCK",
                    "CLAUDE_CODE_USE_VERTEX",
                    "CLAUDE_CODE_USE_FOUNDRY",
                    "ANTHROPIC_API_KEY",
                    "ANTHROPIC_AUTH_TOKEN",
                ]
                .iter()
                .any(|k| {
                    std::env::var(k).is_ok_and(|v| !v.is_empty())
                        || value
                            .as_ref()
                            .and_then(|v| v.get("env"))
                            .and_then(|v| v.get(*k))
                            .and_then(Value::as_str)
                            .is_some_and(|v| !v.is_empty())
                })
            } else {
                let path = std::env::var_os("CODEX_HOME")
                    .map(std::path::PathBuf::from)
                    .or_else(|| dirs::home_dir().map(|p| p.join(".codex")));
                let text = path.and_then(|p| std::fs::read_to_string(p.join("config.toml")).ok());
                let malformed = text.as_ref().is_some_and(|s| toml::from_str::<toml::Value>(s).is_err());
                let value = text.and_then(|s| toml::from_str::<toml::Value>(&s).ok());
                malformed || codex_custom(value.as_ref())
                    || std::env::var("OPENAI_BASE_URL")
                        .is_ok_and(|u| !u.is_empty() && !official_origin(&u, "api.openai.com"))
            }
        }
    }
}
fn official_origin(url: &str, host: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|s| s.split('/').next())
        .is_some_and(|h| h.eq_ignore_ascii_case(host))
}
fn codex_custom(v: Option<&toml::Value>) -> bool {
    let Some(v) = v else { return false };
    let profile = v
        .get("profile")
        .and_then(|v| v.as_str())
        .and_then(|p| v.get("profiles")?.get(p));
    let provider = profile
        .and_then(|v| v.get("model_provider"))
        .or_else(|| v.get("model_provider"))
        .and_then(|v| v.as_str())
        .unwrap_or("openai");
    if provider != "openai" {
        return true;
    }
    v.get("model_providers")
        .and_then(|v| v.get(provider))
        .and_then(|v| v.get("base_url"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !official_origin(s, "api.openai.com"))
}
pub fn poll(
    http: &crate::net::Client,
    id: ProviderId,
    c: &ApiConfig,
) -> Result<UsageWindows, Error> {
    let i = index(id);
    cell().lock().unwrap_or_else(|e| e.into_inner()).readings[i] = Reading::default();
    if c.source != Source::ExternalApi {
        return Err(Error::BadResponse("API/gateway detectado. Configure a consulta de consumo em Configurações > Contas e APIs.".into()));
    }
    validate(c).map_err(Error::BadResponse)?;
    let mut request = http
        .get(&c.url)
        .no_redirects()
        .header("Accept", "application/json");
    if c.header != "none" {
        let k = key(id, c).ok_or_else(|| {
            Error::BadResponse("Cadastre a chave desta URL em Contas e APIs.".into())
        })?;
        if k.contains(['\r', '\n']) {
            return Err(Error::BadResponse("Chave inválida.".into()));
        }
        request = if c.header == "x-api-key" {
            request.header("x-api-key", &k)
        } else {
            request.header("Authorization", &format!("Bearer {k}"))
        };
    }
    let response = request
        .send()
        .map_err(|_| Error::BadResponse("Falha de conexão com a API de consumo.".into()))?;
    if !(200..300).contains(&response.status()) {
        return Err(Error::BadResponse(format!(
            "API de consumo retornou HTTP {}. Confira URL e chave.",
            response.status()
        )));
    }
    if response.body().len() > 1024 * 1024 {
        return Err(Error::BadResponse(
            "Resposta de consumo grande demais.".into(),
        ));
    }
    let v: Value = response
        .json()
        .map_err(|_| Error::BadResponse("A consulta não retornou JSON válido.".into()))?;
    let (windows, reading) = parse(&v, c).map_err(Error::BadResponse)?;
    cell().lock().unwrap_or_else(|e| e.into_inner()).readings[i] = reading;
    Ok(windows)
}
fn parse(v: &Value, c: &ApiConfig) -> Result<(UsageWindows, Reading), String> {
    let number = |p: &str| {
        v.pointer(p)
            .and_then(|v| v.as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
    };
    let used = number(&c.used_pointer).ok_or("Campo de consumo ausente ou não numérico.")?;
    let limit = number(&c.limit_pointer)
        .filter(|n| *n > 0.0)
        .ok_or("Limite ausente, ilimitado ou inválido; não é possível calcular percentual.")?;
    let reset = if c.reset_pointer.is_empty() {
        None
    } else {
        v.pointer(&c.reset_pointer).and_then(|v| {
            if let Some(seconds) = v.as_u64().filter(|n| *n <= 253402300799) {
                UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
            } else {
                v.as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .and_then(|t| u64::try_from(t.timestamp()).ok())
                    .and_then(|n| UNIX_EPOCH.checked_add(Duration::from_secs(n)))
            }
        })
    };
    Ok((
        UsageWindows {
            primary: Window {
                utilization: (used / limit * 100.0).clamp(0.0, 100.0),
                resets_at: reset,
                duration_secs: if c.period_secs == 0 {
                    None
                } else {
                    Some(c.period_secs)
                },
                reported: true,
            },
            secondary: Window::default(),
        },
        Reading {
            used,
            units: c.units.clone(),
            valid: true,
        },
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentage_requires_explicit_same_scope_fields() {
        let c = ApiConfig {
            used_pointer: "/used".into(),
            limit_pointer: "/limit".into(),
            ..Default::default()
        };
        let (w, _) = parse(&serde_json::json!({"used":25,"limit":100}), &c).unwrap();
        assert_eq!(w.primary.utilization, 25.0);
        for v in [
            serde_json::json!({"used":25}),
            serde_json::json!({"used":-1,"limit":100}),
            serde_json::json!({"used":1,"limit":0}),
        ] {
            assert!(parse(&v, &c).is_err())
        }
    }
    #[test]
    fn secrets_are_bound_to_provider_and_exact_url() {
        let c = ApiConfig {
            url: "https://example.com/usage".into(),
            ..Default::default()
        };
        let other = ApiConfig {
            url: "https://other.example/usage".into(),
            ..c.clone()
        };
        assert_ne!(
            target(ProviderId::ChatGpt, &c),
            target(ProviderId::ChatGpt, &other)
        );
        assert_ne!(
            target(ProviderId::ChatGpt, &c),
            target(ProviderId::Claude, &c)
        );
    }
    #[test]
    fn rejects_credentials_in_urls_and_non_https() {
        for u in [
            "http://localhost/usage",
            "https://user:pass@example.com/usage",
            "https://example.com/usage?key=secret",
            "https:///usage",
            "https://example.com/#secret",
        ] {
            assert!(validate_url(u).is_err())
        }
        assert!(validate_url("https://clipoos.online/usage").is_ok());
    }
    #[test]
    fn selected_profile_overrides_top_level_provider() {
        let v: toml::Value = toml::from_str(
            "profile='work'\nmodel_provider='openai'\n[profiles.work]\nmodel_provider='gateway'",
        )
        .unwrap();
        assert!(codex_custom(Some(&v)));
    }
}
