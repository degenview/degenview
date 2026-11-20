//! What the order-book mode remembers between frames: the viewer's choices, apart from the data.

use crate::model::depth::{Price, View};

/// From which size a trade stands out on the tape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BigTrade {
    TenThousand,
    #[default]
    HundredThousand,
    Million,
}

impl BigTrade {
    pub const ALL: [Self; 3] = [Self::TenThousand, Self::HundredThousand, Self::Million];

    /// The notional, in the quote currency, from which a trade counts as big.
    pub const fn threshold(self) -> f64 {
        match self {
            Self::TenThousand => 10_000.0,
            Self::HundredThousand => 100_000.0,
            Self::Million => 1_000_000.0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::TenThousand => "≥ $10K",
            Self::HundredThousand => "≥ $100K",
            Self::Million => "≥ $1M",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BookViewState {
    /// The price span of a row; `None` lets the market's price pick it.
    pub group: Option<Price>,
    /// The price the ladder was scrolled to; `None` follows the market.
    pub centre: Option<Price>,
    pub big: BigTrade,
    /// Scrolling that has not yet added up to a row, in points.
    pub(super) scrolled: f32,
}

impl BookViewState {
    /// What the ladder is to be computed for.
    pub fn view(&self) -> View {
        View {
            group: self.group,
            centre: self.centre,
        }
    }

    /// Starts over on another market: its prices have nothing to do with the last one's.
    pub fn forget_market(&mut self) {
        *self = Self {
            big: self.big,
            ..Self::default()
        };
    }
}
