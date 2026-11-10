//! Where a tab's trading happens: the simulated paper account, or a connected broker with real
//! money. A tab starts on paper and never changes venue by itself.

use degenview_brokers::BrokerId;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TradingVenue {
    #[default]
    Paper,
    Broker(BrokerId),
}

impl TradingVenue {
    pub fn broker(self) -> Option<BrokerId> {
        match self {
            Self::Paper => None,
            Self::Broker(id) => Some(id),
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Broker(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_starts_on_paper() {
        assert_eq!(TradingVenue::default(), TradingVenue::Paper);
        assert!(!TradingVenue::Paper.is_live());
        assert_eq!(TradingVenue::Paper.broker(), None);
        let live = TradingVenue::Broker(BrokerId::new("acme"));
        assert!(live.is_live());
        assert_eq!(live.broker(), Some(BrokerId::new("acme")));
    }
}
