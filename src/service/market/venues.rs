//! Every venue, built with the ports it is given: the one place that names them all.

use super::alpaca::Alpaca;
use super::binance::Binance;
use super::bybit::Bybit;
use super::cmc::CoinMarketCap;
use super::coinbase::Coinbase;
use super::coingecko::CoinGecko;
use super::dexscreener::DexScreener;
use super::gateio::Gate;
use super::kalshi::Kalshi;
use super::kraken::Kraken;
use super::mexc::Mexc;
use super::oanda::Oanda;
use super::okx::Okx;
use super::polymarket::Polymarket;
use super::ports::context::MarketContext;
use crate::model::DataSource;
use crate::service::MarketSource;

pub struct Venues {
    pub alpaca: Alpaca,
    pub binance: Binance,
    pub bybit: Bybit,
    pub cmc: CoinMarketCap,
    pub coinbase: Coinbase,
    pub coingecko: CoinGecko,
    pub dexscreener: DexScreener,
    pub gate: Gate,
    pub kalshi: Kalshi,
    pub kraken: Kraken,
    pub mexc: Mexc,
    pub oanda: Oanda,
    pub okx: Okx,
    pub polymarket: Polymarket,
}

impl Venues {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            alpaca: Alpaca::new(context),
            binance: Binance::new(context),
            bybit: Bybit::new(context),
            cmc: CoinMarketCap::new(context),
            coinbase: Coinbase::new(context),
            coingecko: CoinGecko::new(context),
            dexscreener: DexScreener::new(context),
            gate: Gate::new(context),
            kalshi: Kalshi::new(context),
            kraken: Kraken::new(context),
            mexc: Mexc::new(context),
            oanda: Oanda::new(context),
            okx: Okx::new(context),
            polymarket: Polymarket::new(context),
        }
    }

    /// The venue that serves `source`.
    pub fn source(&self, source: DataSource) -> &dyn MarketSource {
        match source {
            DataSource::Alpaca => &self.alpaca,
            DataSource::Binance => &self.binance,
            DataSource::Bybit => &self.bybit,
            DataSource::CoinGecko => &self.coingecko,
            DataSource::CoinMarketCap => &self.cmc,
            DataSource::Coinbase => &self.coinbase,
            DataSource::DexScreener => &self.dexscreener,
            DataSource::Gate => &self.gate,
            DataSource::Kalshi => &self.kalshi,
            DataSource::Kraken => &self.kraken,
            DataSource::Mexc => &self.mexc,
            DataSource::Oanda => &self.oanda,
            DataSource::Okx => &self.okx,
            DataSource::Polymarket => &self.polymarket,
        }
    }
}
