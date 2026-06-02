// Usage data shapes shared across providers.

use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ProviderId {
    Claude,
    ChatGpt,
    OpenCodeGo,
}

impl ProviderId {
    pub const ALL: [Self; 3] = [Self::Claude, Self::ChatGpt, Self::OpenCodeGo];
    pub const LIVE_USAGE: [Self; 2] = [Self::Claude, Self::ChatGpt];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ChatGpt => "chatgpt",
            Self::OpenCodeGo => "opencode-go",
        }
    }

    pub fn metadata(self) -> ProviderMetadata {
        match self {
            Self::Claude => ProviderMetadata {
                id: self,
                slug: "claude",
                default_enabled: true,
                tray_icon_id: 1,
                display_mode: ProviderDisplayMode::TwoWindow,
                live_usage: true,
            },
            Self::ChatGpt => ProviderMetadata {
                id: self,
                slug: "chatgpt",
                default_enabled: false,
                tray_icon_id: 2,
                display_mode: ProviderDisplayMode::TwoWindow,
                live_usage: true,
            },
            Self::OpenCodeGo => ProviderMetadata {
                id: self,
                slug: "opencode-go",
                default_enabled: false,
                tray_icon_id: 3,
                display_mode: ProviderDisplayMode::WeeklyMonthlyFourBar,
                live_usage: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderDisplayMode {
    TwoWindow,
    WeeklyMonthlyFourBar,
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderMetadata {
    pub id: ProviderId,
    pub slug: &'static str,
    pub default_enabled: bool,
    pub tray_icon_id: u32,
    pub display_mode: ProviderDisplayMode,
    pub live_usage: bool,
}

/// One usage window: how much you've consumed (0–100), and when it resets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Window {
    pub utilization: f64,
    pub resets_at: Option<SystemTime>,
}

/// The pair of windows a provider reports per poll.
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageWindows {
    pub primary: Window,
    pub secondary: Window,
}

/// One provider's most recent poll result, keyed by `id`.
#[derive(Clone, Debug)]
pub struct ProviderSnapshot {
    pub id: ProviderId,
    pub windows: UsageWindows,
}
