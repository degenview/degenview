//! Which exchange and which of its markets a feed is about. Liquidations, taker flow and the order
//! book all name their sources this way, so adding an exchange starts here.

#[cfg(test)]
mod tests;

/// An exchange whose public sockets the app follows.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Exchange {
    Binance,
    Bybit,
}

impl Exchange {
    pub fn name(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Bybit => "Bybit",
        }
    }
}

/// Spot or the perpetual contract of the same asset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Market {
    Spot,
    Perpetual,
}

/// One market of an asset on one exchange.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Venue {
    pub exchange: Exchange,
    pub market: Market,
}

impl Venue {
    /// Every venue, in the order a panel lists them.
    pub const ALL: [Self; 4] = [
        Self::new(Exchange::Binance, Market::Spot),
        Self::new(Exchange::Binance, Market::Perpetual),
        Self::new(Exchange::Bybit, Market::Spot),
        Self::new(Exchange::Bybit, Market::Perpetual),
    ];

    pub const fn new(exchange: Exchange, market: Market) -> Self {
        Self { exchange, market }
    }

    pub fn label(self) -> String {
        let market = match self.market {
            Market::Spot => "spot",
            Market::Perpetual => "perp",
        };
        format!("{} {market}", self.exchange.name())
    }
}
