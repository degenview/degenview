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

/// The Oanda account id, personal access token and the environment they belong to, as one login.
/// How it is stored is the keychain's business (`service::keychain::api_keys`); its `Debug` leaves
/// the token out, so a login in a log or a failed assertion does not leak it.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct OandaCredentials {
    pub account_id: String,
    pub access_token: String,
    pub environment: OandaEnvironment,
}

impl OandaCredentials {
    /// The login without the whitespace a pasted id or token may carry.
    pub fn trimmed(&self) -> Self {
        Self {
            account_id: self.account_id.trim().to_owned(),
            access_token: self.access_token.trim().to_owned(),
            environment: self.environment,
        }
    }
}

impl std::fmt::Debug for OandaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OandaCredentials")
            .field("account_id", &self.account_id)
            .field("access_token", &"<redacted>")
            .field("environment", &self.environment)
            .finish()
    }
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

    #[test]
    fn a_login_is_trimmed_in_its_id_and_token_and_keeps_its_environment() {
        let pasted = OandaCredentials {
            account_id: " 101-001-1 \n".to_owned(),
            access_token: "\ttoken ".to_owned(),
            environment: OandaEnvironment::Practice,
        };
        assert_eq!(
            pasted.trimmed(),
            OandaCredentials {
                account_id: "101-001-1".to_owned(),
                access_token: "token".to_owned(),
                environment: OandaEnvironment::Practice,
            }
        );
    }

    #[test]
    fn a_login_printed_for_debugging_does_not_hold_its_token() {
        let login = OandaCredentials {
            account_id: "101-001-1".to_owned(),
            access_token: "s3cr3t-token".to_owned(),
            environment: OandaEnvironment::Live,
        };
        let printed = format!("{login:?} {login:#?}");
        assert!(!printed.contains("s3cr3t-token"), "{printed}");
        assert!(printed.contains("101-001-1"), "the account is no secret");
    }
}
