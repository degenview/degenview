use crate::model::market::oanda::OandaCredentials;
use crate::service::keychain::api_keys;
use crate::service::market::ports::credentials::Credentials;

/// The keys saved in Settings (the Keychain, or memory when it is turned off), else the
/// environment: `service::keychain::api_keys` holds the rules and the order.
pub struct KeychainCredentials;

impl Credentials for KeychainCredentials {
    fn alpaca(&self) -> Option<(String, String)> {
        api_keys::alpaca_credentials()
    }

    fn oanda(&self) -> Option<OandaCredentials> {
        api_keys::oanda_credentials()
    }

    fn cmc(&self) -> Option<String> {
        api_keys::cmc_api_key()
    }
}
