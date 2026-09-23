// Query the GitHub Releases API and pick the relevant asset.

use serde::Deserialize;

use crate::net::Client;

const ASSET_NAME: &str = "claude-code-usage-bubble.exe";
// Fork (Rafael): self-update feed points at our own GitHub repo so every PC
// running this fork receives fork releases (never the upstream build).
const REPO_OWNER: &str = "rafaelcaus";
const REPO_NAME: &str = "codex-usage-bubble";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    pub asset_url: String,
    /// SHA-256 of the asset bytes, parsed from the GitHub Releases
    /// API `digest` field. `None` if GitHub omitted it (older
    /// releases predate the digest field).
    pub asset_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version {
            major: 0,
            minor: 0,
            patch: 0,
        })
    }

    pub fn parse(s: &str) -> Option<Self> {
        let core = s.trim().trim_start_matches('v').split('-').next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        Some(Version {
            major: parts.next().flatten().unwrap_or(0),
            minor: parts.next().flatten().unwrap_or(0),
            patch: parts.next().flatten().unwrap_or(0),
        })
    }
}

pub fn fetch_latest(http: &Client) -> Result<super::CheckOutcome, super::Error> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let resp = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", user_agent())
        .send()?;
    if !(200..300).contains(&resp.status()) {
        return Err(super::Error::Network(crate::net::Error::Status(resp.status())));
    }
    let body: GhRelease = resp.json()?;
    let candidate = Version::parse(&body.tag_name)
        .ok_or_else(|| super::Error::BadVersion(body.tag_name.clone()))?;
    if candidate <= Version::current() {
        return Ok(super::CheckOutcome::UpToDate);
    }
    let asset = body
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET_NAME))
        .or_else(|| {
            body.assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
        })
        .ok_or(super::Error::NoAsset)?;
    Ok(super::CheckOutcome::Available(Release {
        version: candidate,
        asset_url: asset.browser_download_url.clone(),
        asset_sha256: asset.digest.as_deref().and_then(parse_sha256_digest),
    }))
}

/// Parse a GitHub `digest` field of the form `"sha256:<64 hex chars>"`
/// into a 32-byte array. Returns `None` for any other algorithm or
/// malformed input — callers should treat a missing digest as "no
/// integrity check available" rather than as a parse failure.
fn parse_sha256_digest(raw: &str) -> Option<[u8; 32]> {
    let hex = raw.strip_prefix("sha256:")?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte_chars) in hex.as_bytes().chunks(2).enumerate() {
        let high = (byte_chars[0] as char).to_digit(16)?;
        let low = (byte_chars[1] as char).to_digit(16)?;
        out[i] = ((high << 4) | low) as u8;
    }
    Some(out)
}

pub fn user_agent() -> &'static str {
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    /// GitHub started returning `digest: "sha256:..."` on the asset
    /// object in 2024. Older releases omit it; we treat that as
    /// "verification unavailable" rather than a hard error.
    #[serde(default)]
    digest: Option<String>,
}
