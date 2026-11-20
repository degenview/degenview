//! What the order-book mode remembers between frames: the viewer's choices, apart from the data.

use crate::model::depth::{DepthMarket, Price, View};

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

/// The rows the ladder is asked for before it has been drawn once: about what a panel of the
/// default size has room for.
const FIRST_ROWS: usize = 14;

#[derive(Clone, Debug, PartialEq)]
pub struct BookViewState {
    /// The price span of a row; `None` lets the market's price pick it.
    pub group: Option<Price>,
    /// The price the ladder was scrolled to; `None` follows the market.
    pub centre: Option<Price>,
    pub big: BigTrade,
    /// How many price rows the ladder had room for when it was last drawn. The ladder is computed
    /// before it is drawn, so a resize shows with a frame's delay.
    pub(super) rows: usize,
    /// The market the choices above are in the prices of.
    market: Option<DepthMarket>,
    /// Scrolling that has not yet added up to a row, in points.
    pub(super) scrolled: f32,
}

impl Default for BookViewState {
    fn default() -> Self {
        Self {
            group: None,
            centre: None,
            big: BigTrade::default(),
            rows: FIRST_ROWS,
            market: None,
            scrolled: 0.0,
        }
    }
}

impl BookViewState {
    /// What the ladder is to be computed for.
    pub fn view(&self) -> View {
        View {
            group: self.group,
            centre: self.centre,
            rows: self.rows,
        }
    }

    /// Notes the market the book is drawn for. On another one the row size and scroll start over,
    /// because they are in the last market's prices; the big-trade choice and the room for rows
    /// stay.
    pub fn follow(&mut self, market: Option<&DepthMarket>) {
        if self.market.as_ref() != market {
            *self = Self {
                big: self.big,
                rows: self.rows,
                market: market.cloned(),
                ..Self::default()
            };
        }
    }
}
