// Read Claude credentials from the Windows user profile.
//
// Path: `%USERPROFILE%\.claude\.credentials.json` (matches what the
// official Claude CLI writes). Schema: `{ "claudeAiOauth": { "accessToken",
// "expiresAt": <ms-since-epoch> } }`.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

pub struct LocalClaudeCreds {
    path: PathBuf,
    id: String,
}

impl LocalClaudeCreds {
    pub fn detect() -> Option<Self> {
        let home = dirs::home_dir()?;
        let path = home.join(".claude").join(".credentials.json");
        let id = format!("local:{}", path.display());
        Some(Self { path, id })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl super::CredentialSource for LocalClaudeCreds {
    fn id(&self) -> &str {
        &self.id
    }

    fn read(&self) -> Result<super::Token, super::Error> {
        let content = std::fs::read_to_string(&self.path)?;
        parse_claude_json(&content)
    }

    fn signature(&self) -> Option<String> {
        let meta = std::fs::metadata(&self.path).ok()?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(format!("{}|{}|{}", self.id, meta.len(), modified))
    }

    fn refresh_hint(&self) -> super::RefreshHint {
        super::RefreshHint::LocalClaudeCli
    }
}

/// Parse the Claude credential JSON shape written by the official CLI.
pub(crate) fn parse_claude_json(content: &str) -> Result<super::Token, super::Error> {
    let value: serde_json::Value = serde_json::from_str(content)?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or(super::Error::MissingField("claudeAiOauth"))?;
    let access_token = oauth
        .get("accessToken")
        .and_then(|v| v.as_str())
        .ok_or(super::Error::MissingField("accessToken"))?
        .to_string();
    let expires_at_unix_ms = oauth.get("expiresAt").and_then(|v| v.as_i64());
    Ok(super::Token {
        access_token,
        expires_at_unix_ms,
        account_id: None,
    })
}
