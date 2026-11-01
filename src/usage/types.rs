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
    pub duration_secs: Option<u64>,
    pub reported: bool,
}

impl Window {
    pub fn label(&self) -> String {
        match self.duration_secs {
            Some(604800) => "7d".into(),
            Some(18000) => "5h".into(),
            Some(n) if n > 0 && n % 86400 == 0 => format!("{}d",n/86400),
            Some(n) if n > 0 && n % 3600 == 0 => format!("{}h",n/3600),
            Some(n) if n > 0 && n % 60 == 0 => format!("{}min",n/60),
            _ => "Cota".into(),
        }
    }
}

/// The pair of windows a provider reports per poll.
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageWindows {
    pub primary: Window,
    pub secondary: Window,
}

impl UsageWindows {
    pub fn displayed(&self) -> Vec<Window> {
        let mut windows:Vec<_> = [self.primary,self.secondary].into_iter().filter(|w|w.reported).collect();
        windows.sort_by_key(|w|w.duration_secs.unwrap_or(u64::MAX));
        windows
    }
    pub fn weekly(&self) -> Option<Window> {
        [self.primary,self.secondary].into_iter().find(|w|w.reported && w.duration_secs==Some(604800))
    }
    pub fn pressure(&self) -> Option<f64> {
        self.displayed().into_iter().map(|w|w.utilization).reduce(f64::max)
    }
}

/// One provider's most recent poll result, keyed by `id`.
#[derive(Clone, Debug)]
pub struct ProviderSnapshot {
    pub id: ProviderId,
    pub windows: UsageWindows,
}
