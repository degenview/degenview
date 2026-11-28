use crate::model::market::oanda::OandaCredentials;
use crate::service::keychain::api_keys::{
    self, AlpacaCredentialsStore, CoinMarketCapKeyStore, OandaCredentialsStore,
};
use crate::service::market::ports::credentials::Credentials;

/// The keys saved in Settings (the Keychain, or memory when it is turned off), else the
/// environment: `service::keychain::api_keys` holds the rules and the order. Each venue is asked
/// of its own store.
pub struct KeychainCredentials<'a> {
    alpaca: &'a AlpacaCredentialsStore,
    oanda: &'a OandaCredentialsStore,
    cmc: &'a CoinMarketCapKeyStore,
    env: fn(&str) -> Option<String>,
}

impl KeychainCredentials<'static> {
    /// The app's stores and its process environment.
    pub fn app() -> Self {
        Self::over(
            AlpacaCredentialsStore::shared(),
            OandaCredentialsStore::shared(),
            CoinMarketCapKeyStore::shared(),
            |name| std::env::var(name).ok(),
        )
    }
}

impl<'a> KeychainCredentials<'a> {
    pub fn over(
        alpaca: &'a AlpacaCredentialsStore,
        oanda: &'a OandaCredentialsStore,
        cmc: &'a CoinMarketCapKeyStore,
        env: fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            alpaca,
            oanda,
            cmc,
            env,
        }
    }
}

impl Credentials for KeychainCredentials<'_> {
    fn alpaca(&self) -> Option<(String, String)> {
        api_keys::alpaca_credentials_with(self.alpaca, &self.env)
    }

    fn oanda(&self) -> Option<OandaCredentials> {
        api_keys::oanda_credentials_with(self.oanda, &self.env)
    }

    fn cmc(&self) -> Option<String> {
        api_keys::cmc_api_key_with(self.cmc, &self.env)
    }
}
