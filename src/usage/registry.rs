// Provider registry: holds every enabled provider and dispatches polls.
//
// The app owns one `Registry` and calls `poll_enabled(http, settings)` on
// every cycle. The result is a flat list of `(id, Result<UsageWindows>)`
// pairs the app can apply to its state.

use crate::creds::Locator;
use crate::net::Client;
use crate::settings::Settings;
use crate::usage::{
    anthropic::ClaudeProvider, chatgpt::ChatGptProvider, refresh, Error, ProviderId, UsageProvider,
    UsageWindows,
};

pub struct Registry {
    claude: ClaudeProvider,
    chatgpt: ChatGptProvider,
}

impl Registry {
    pub fn with_defaults() -> Self {
        Self {
            claude: ClaudeProvider::new(Locator::for_claude()),
            chatgpt: ChatGptProvider::new(Locator::for_chatgpt()),
        }
    }

    pub fn poll_enabled(
        &mut self,
        http: &Client,
        settings: &Settings,
    ) -> Vec<(ProviderId, Result<UsageWindows, Error>)> {
        let mut out = Vec::new();
        for provider in ProviderId::LIVE_USAGE {
            if !settings.is_provider_enabled(provider) {
                continue;
            }
            let result = match provider {
                ProviderId::Claude => self.claude.poll(http),
                ProviderId::ChatGpt => self.chatgpt.poll(http),
                ProviderId::OpenCodeGo => {
                    unreachable!("OpenCode Go has no live usage provider yet")
                }
            };
            out.push((provider, result));
        }
        out
    }

    /// Attempt to refresh the active source for one provider.
    pub fn try_refresh(
        &self,
        id: ProviderId,
        orchestrator: &refresh::Orchestrator,
    ) -> refresh::Outcome {
        let locator = match id {
            ProviderId::Claude => self.claude.locator(),
            ProviderId::ChatGpt => self.chatgpt.locator(),
            ProviderId::OpenCodeGo => return refresh::Outcome::CliMissing,
        };
        match locator.first_available() {
            Some(src) => orchestrator.refresh(src),
            None => refresh::Outcome::CliMissing,
        }
    }

    /// Snapshot of credential-file fingerprints across both providers —
    /// used to detect external re-authentication between poll cycles.
    pub fn credential_signatures(&self) -> Vec<String> {
        let mut sigs = self.claude.locator().signatures();
        sigs.extend(self.chatgpt.locator().signatures());
        sigs.sort();
        sigs.dedup();
        sigs
    }
}
