// Token refresh orchestrator.
//
// Each provider's credential source advertises a `RefreshHint` describing
// which CLI to spawn. We invoke that CLI and watch the credential file's
// signature; if it changes within the timeout we declare success.



use std::time::Duration;

use crate::creds::CredentialSource;



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Refreshed,
    StillExpired,
    CliMissing,
    Timeout,
}

pub struct Orchestrator {
    timeout: Duration,
    poll_interval: Duration,
}

impl Orchestrator {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            poll_interval: Duration::from_millis(500),
        }
    }

    pub fn refresh(&self, source: &dyn CredentialSource) -> Outcome {
        // Login renewal is an explicit user action; never generate a model prompt.
        let _ = source;
        Outcome::StillExpired
    }
}
