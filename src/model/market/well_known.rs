//! The markets the app knows without asking anyone: what the new tab and the pickers offer before
//! anything is searched, and the companies of stocks no provider has named yet. Company and coin
//! names are market data, so they are never translated.

/// A coin offered before anything is searched.
#[derive(Clone, Copy, Debug)]
pub struct Coin {
    pub ticker: &'static str,
    pub name: &'static str,
}

/// A US stock or ETF offered before anything is searched.
#[derive(Clone, Copy, Debug)]
pub struct Stock {
    pub ticker: &'static str,
    /// The short name a tile has room for.
    pub name: &'static str,
    /// What the listing is called.
    pub company: &'static str,
}

const fn coin(ticker: &'static str, name: &'static str) -> Coin {
    Coin { ticker, name }
}

const fn stock(ticker: &'static str, name: &'static str, company: &'static str) -> Stock {
    Stock {
        ticker,
        name,
        company,
    }
}

pub const COINS: [Coin; 10] = [
    coin("BTC", "Bitcoin"),
    coin("ETH", "Ethereum"),
    coin("SOL", "Solana"),
    coin("BNB", "BNB"),
    coin("XRP", "XRP"),
    coin("ADA", "Cardano"),
    coin("DOGE", "Dogecoin"),
    coin("AVAX", "Avalanche"),
    coin("DOT", "Polkadot"),
    coin("LINK", "Chainlink"),
];

pub const STOCKS: [Stock; 12] = [
    stock("AAPL", "Apple", "Apple Inc."),
    stock("MSFT", "Microsoft", "Microsoft Corporation"),
    stock("NVDA", "NVIDIA", "NVIDIA Corporation"),
    stock("AMZN", "Amazon", "Amazon.com, Inc."),
    stock("GOOGL", "Alphabet", "Alphabet Inc. Class A"),
    stock("META", "Meta", "Meta Platforms, Inc."),
    stock("TSLA", "Tesla", "Tesla, Inc."),
    stock("SPY", "S&P 500", "SPDR S&P 500 ETF Trust"),
    stock("QQQ", "Nasdaq 100", "Invesco QQQ Trust"),
    stock("AMD", "AMD", "Advanced Micro Devices, Inc."),
    stock("MSTR", "Strategy", "Strategy Inc"),
    stock("SPCX", "SpaceX", "Space Exploration Technologies Corp."),
];

/// The company listed under `ticker`, in any case, if it is a well-known one.
pub fn company(ticker: &str) -> Option<&'static str> {
    STOCKS
        .iter()
        .find(|stock| stock.ticker.eq_ignore_ascii_case(ticker))
        .map(|stock| stock.company)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_known_stock_names_its_company_in_any_case() {
        assert_eq!(company("aapl"), Some("Apple Inc."));
        assert_eq!(company("MSTR"), Some("Strategy Inc"));
        assert_eq!(
            company("SPCX"),
            Some("Space Exploration Technologies Corp.")
        );
        assert_eq!(company("ZZZZ"), None);
    }
}
