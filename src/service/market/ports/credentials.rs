//! The keys a venue signs its requests with: the ones the user saved in Settings, else the
//! environment, as `service::keychain::api_keys` decides. A venue asks once per answer and passes
//! what it was given down, so the pages of one history are all signed by one login.

use crate::model::market::oanda::OandaCredentials;

pub trait Credentials: Send + Sync {
    /// Alpaca's key id and secret.
    fn alpaca(&self) -> Option<(String, String)>;

    /// Oanda's account, access token and the environment they belong to.
    fn oanda(&self) -> Option<OandaCredentials>;

    /// CoinMarketCap's API key. Its public endpoints answer without one, at lower limits.
    fn cmc(&self) -> Option<String>;
}
