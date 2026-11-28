//! Oanda's two environments and the login that belongs to one.

use crate::i18n::tr_c;
use serde::{Deserialize, Serialize};

/// Which of Oanda's two systems an account and its access token belong to. Each has its own API
/// host, and a token works only in the environment it was generated in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OandaEnvironment {
    /// fxTrade: funded accounts.
    #[default]
    Live,
    /// fxTrade Practice: free demo accounts.
    Practice,
}

impl OandaEnvironment {
    pub const ALL: [Self; 2] = [Self::Live, Self::Practice];

    pub fn label(self) -> &'static str {
        match self {
            Self::Live => tr_c!("Oanda environment", "Live"),
            Self::Practice => tr_c!("Oanda environment", "Practice"),
        }
    }

    /// The environment written as `live` or `practice`, in any case.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "live" => Some(Self::Live),
            "practice" => Some(Self::Practice),
            _ => None,
        }
    }
}

/// The Oanda account id, personal access token and the environment they belong to, as one item.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OandaCredentials {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "accessToken")]
    pub access_token: String,
    /// Items saved before the choice existed were all sent to the live API.
    #[serde(default)]
    pub environment: OandaEnvironment,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_is_written_live_or_practice() {
        assert_eq!(
            OandaEnvironment::parse(" Practice "),
            Some(OandaEnvironment::Practice)
        );
        assert_eq!(
            OandaEnvironment::parse("LIVE"),
            Some(OandaEnvironment::Live)
        );
        assert_eq!(OandaEnvironment::parse("demo"), None);
        assert_eq!(OandaEnvironment::parse(""), None);
    }
}
