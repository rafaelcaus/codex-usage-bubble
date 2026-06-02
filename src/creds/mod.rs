// Pluggable credential discovery.
//
// Each `CredentialSource` knows how to read a single OAuth token from
// somewhere, such as a local JSON file. The `Locator`
// holds a priority-ordered list and serves the first source that
// actually has a token. New sources drop in without touching the locator.

pub mod codex_auth;
pub mod local_fs;

#[derive(Clone, Debug)]
pub struct Token {
    pub access_token: String,
    /// Expiry timestamp in *milliseconds* since Unix epoch, matching the
    /// format the Claude CLI writes. `None` means "the source didn't say".
    pub expires_at_unix_ms: Option<i64>,
    pub account_id: Option<String>,
}

/// Tells `RefreshOrchestrator` which CLI to spawn when the token rotates.
#[derive(Clone, Debug)]
pub enum RefreshHint {
    /// `claude.cmd` / `claude.exe` on PATH.
    LocalClaudeCli,
    /// `codex` / `codex.cmd` / `codex.ps1` on PATH.
    LocalCodexCli,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("required field missing from credential JSON: {0}")]
    MissingField(&'static str),
    #[error("credential source unavailable")]
    Unavailable,
}

pub trait CredentialSource: Send + Sync {
    /// Stable identifier used in logs and the locator's change-detection
    /// signatures (e.g. `"local:C:\\Users\\me\\.claude\\.credentials.json"`).
    fn id(&self) -> &str;

    /// Read the current token.
    fn read(&self) -> Result<Token, Error>;

    /// Cheap change-detection fingerprint. `None` means "source is missing".
    fn signature(&self) -> Option<String>;

    fn refresh_hint(&self) -> RefreshHint;
}

/// Ordered set of credential sources. The first source with a valid
/// `signature()` is treated as the "active" one.
pub struct Locator {
    sources: Vec<Box<dyn CredentialSource>>,
}

impl Locator {
    pub fn new(sources: Vec<Box<dyn CredentialSource>>) -> Self {
        Self { sources }
    }

    /// Build a Claude locator with the standard Windows credential path.
    pub fn for_claude() -> Self {
        let mut sources: Vec<Box<dyn CredentialSource>> = Vec::new();
        if let Some(s) = local_fs::LocalClaudeCreds::detect() {
            sources.push(Box::new(s));
        }
        Self { sources }
    }

    /// Build a ChatGPT/Codex locator with the standard search order.
    pub fn for_chatgpt() -> Self {
        let mut sources: Vec<Box<dyn CredentialSource>> = Vec::new();
        if let Some(s) = codex_auth::LocalCodexCreds::detect() {
            sources.push(Box::new(s));
        }
        Self { sources }
    }

    /// First source whose signature is currently non-None.
    pub fn first_available(&self) -> Option<&dyn CredentialSource> {
        self.sources
            .iter()
            .find(|s| s.signature().is_some())
            .map(Box::as_ref)
    }

    /// Snapshot of fingerprints for every reachable source — used by the
    /// app to detect credential changes (re-login) between poll cycles.
    pub fn signatures(&self) -> Vec<String> {
        let mut sigs: Vec<String> = self.sources.iter().filter_map(|s| s.signature()).collect();
        sigs.sort();
        sigs.dedup();
        sigs
    }
}
