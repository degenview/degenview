//! What the trading interface knows about a venue, in words that belong to none of them.
//!
//! A *venue* is somewhere orders go: the simulated paper account, or a connected broker. The
//! interface (the order sheet, the orders panel, the chart pills) is drawn from these types only;
//! each venue's adapter turns its own account state into them. Nothing here names a venue.
//!
//! Money and sizes are [`Decimal`]: the paper engine works in `f64` and converts at its edge,
//! rounded to the market's increments, so a number shown here is a number that can be sent.

use degenview_brokers::{Decimal, Side};

use crate::model::broker::venue::TradingVenue;

/// How a venue treats the orders sent to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VenueKind {
    /// Nothing leaves this computer: a practice account with pretend money.
    Simulated,
    /// A broker's test environment: real software, canned or fake money.
    Test,
    /// Real money.
    Live,
}

impl VenueKind {
    /// Whether an order placed here costs real money, so it is confirmed and marked as such.
    pub fn is_real_money(self) -> bool {
        self == Self::Live
    }

    /// The word on the venue's pill.
    pub fn label(self) -> &'static str {
        match self {
            Self::Simulated => "Simulated",
            Self::Test => "Test",
            Self::Live => "Live",
        }
    }
}

/// A venue, as the list the user chooses from shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VenueInfo {
    pub venue: TradingVenue,
    pub name: String,
    pub kind: VenueKind,
    /// One line on what this is, for the list of venues to connect.
    pub summary: String,
    /// Connecting needs no credential: it is enabled with one click and kept as a choice.
    pub needs_no_credentials: bool,
}

/// Whether a venue can be used right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendState {
    /// Chosen by the user but not connected (no connection, or none yet).
    NotConnected,
    Connecting,
    Ready,
    /// Working, but orders are held back until it recovers; the sentence says why.
    Paused(String),
    /// Needs the user (a rejected key, an unreadable record); the sentence says what.
    NeedsAttention(String),
}

impl BackendState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Which kind of order the sheet offers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OrderClass {
    Market,
    Limit,
    /// Triggers at a price, then trades at the market.
    Stop,
    /// Triggers at a price, then rests at a limit.
    StopLimit,
}

impl OrderClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Market => "Market",
            Self::Limit => "Limit",
            Self::Stop => "Stop",
            Self::StopLimit => "Stop limit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimeInForce {
    GoodTilCancelled,
    Day,
    ImmediateOrCancel,
    FillOrKill,
}

impl TimeInForce {
    pub fn label(self) -> &'static str {
        match self {
            Self::GoodTilCancelled => "Good til cancelled",
            Self::Day => "Day",
            Self::ImmediateOrCancel => "Immediate or cancel",
            Self::FillOrKill => "Fill or kill",
        }
    }
}

/// What can be asked for in one market at one venue. The order sheet shows exactly this: a control
/// the venue cannot honour is not drawn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketCaps {
    pub order_classes: Vec<OrderClass>,
    pub time_in_force: Vec<TimeInForce>,
    /// An order that must rest and never take liquidity.
    pub post_only: bool,
    /// An order that may only shrink a position.
    pub reduce_only: bool,
    /// Take-profit and stop-loss orders attached to an entry.
    pub brackets: bool,
    /// A market buy may be sized as an amount of the quote asset to spend.
    pub quote_sized_market: bool,
    /// A sell may open a position the account does not hold.
    pub shorting: bool,
    /// A working order's price or size can be changed in place.
    pub edit_in_place: bool,
}

impl TicketCaps {
    pub fn allows(&self, class: OrderClass) -> bool {
        self.order_classes.contains(&class)
    }
}

/// How a market is sized and priced at a venue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketRules {
    /// What the venue calls the market, for display (`BTC-USD`).
    pub name: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub price_increment: Decimal,
    pub size_increment: Decimal,
    pub min_size: Decimal,
    /// The smallest order value in the quote asset, where the venue has one.
    pub min_notional: Decimal,
    pub caps: TicketCaps,
}

/// Why a chart's market cannot be traded at a venue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unavailable {
    pub kind: UnavailableKind,
    /// The sentence shown to the user.
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableKind {
    /// The chart is not a market of this venue at all (another data source, no market, or one the venue does not list):
    /// nothing is offered and nothing is substituted.
    NotThisVenue,
    /// Will be possible shortly (connecting, reading the market's rules).
    Waiting,
    /// Not possible now, and the user can do something about it or must wait for the venue.
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarketAccess {
    Ready(Box<MarketRules>),
    Unavailable(Unavailable),
}

impl MarketAccess {
    #[cfg(test)]
    pub fn rules(&self) -> Option<&MarketRules> {
        match self {
            Self::Ready(rules) => Some(rules),
            Self::Unavailable(_) => None,
        }
    }
}

/// The best prices of a market. A venue with no book (paper) gives only `last`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QuoteView {
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub last: Option<Decimal>,
}

impl QuoteView {
    /// The price an order of `side` would meet first, falling back to the last trade.
    pub fn reference(&self, side: Side) -> Option<Decimal> {
        match side {
            Side::Buy => self.ask.or(self.last),
            Side::Sell => self.bid.or(self.last),
        }
    }
}

/// An amount of one asset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Amount {
    pub value: Decimal,
    pub asset: String,
}

impl Amount {
    pub fn new(value: Decimal, asset: impl Into<String>) -> Self {
        Self {
            value,
            asset: asset.into(),
        }
    }
}

/// Which account, and which connection to it, an order was reviewed against. The sheet keeps the
/// one it showed the figures for and sends the order only against that same one: if the account or
/// the connection changed in between (another key was entered, another paper account chosen), the
/// review no longer describes where the order would go, and the venue refuses to send it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Binding {
    /// The venue's own id of the account. Empty when there is none (not connected).
    pub account: String,
    /// Distinguishes one connection to the same account from the next. 0 when the venue has no such
    /// thing.
    pub generation: u64,
}

/// The account line at the top of the sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountStrip {
    pub label: String,
    /// Total value including open positions, where the venue knows it.
    pub equity: Option<Amount>,
}

/// How a figure in the account strip is coloured: quiet, green or red for a gain or a loss, orange
/// or red for a buffer that is thin or nearly gone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatTone {
    Neutral,
    Profit,
    Loss,
    Thin,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trend {
    Up,
    Down,
    Flat,
}

/// The line under the strip's big figure: a trend arrow, a figure and a word ("open").
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeroNote {
    pub trend: Trend,
    pub text: String,
    pub tone: StatTone,
    pub label: String,
}

/// The big figure at the left of the account strip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hero {
    pub title: String,
    pub value: String,
    /// What the figure is not exact about (a value that leaves out unpriced assets), shown on hover.
    pub help: Option<String>,
    pub note: Option<HeroNote>,
}

/// One quieter figure of the strip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatCell {
    pub title: String,
    pub value: String,
    pub tone: StatTone,
    /// Said on hover.
    pub help: String,
}

/// The account's numbers, in words and figures the strip draws as they are: every venue builds its
/// own, the strip knows none of them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountStats {
    pub hero: Hero,
    pub cells: Vec<StatCell>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Good,
    Warning,
    Bad,
    Neutral,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowStatus {
    pub label: &'static str,
    pub tone: Tone,
}

/// One order, working or finished.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderRow {
    /// The venue's id, as a string; opaque to the interface, passed back to cancel or edit.
    pub id: String,
    pub market: String,
    pub side: Side,
    pub kind: String,
    /// The limit price, or the stop's when that is all the order has.
    pub price: Option<Decimal>,
    /// The price that sets a stop order off, when the order has both a trigger and a limit.
    pub trigger: Option<Decimal>,
    pub filled: Decimal,
    /// The size asked for, in the base asset; `None` for a market order sized by an amount to spend.
    pub size: Option<Decimal>,
    /// The note shown beside a size sized in the quote asset ("spend").
    pub spend: Option<Decimal>,
    pub status: RowStatus,
    pub created_ms: Option<i64>,
    pub working: bool,
    pub can_cancel: bool,
    pub can_edit: bool,
    /// The venue's words when it refused or ended the order.
    pub note: Option<String>,
    /// Why a working order cannot be edited, for the tooltip of the disabled pencil.
    pub edit_hint: Option<String>,
    /// The market's data source, for its logo; none when the venue does not say.
    pub source: Option<crate::model::DataSource>,
    /// A small chip beside the type: the role of a protective order ("TP", "SL").
    pub tag: Option<RowStatus>,
    /// Sent, and the answer was lost: it may or may not be live. `id` is then the key to look it up.
    pub unconfirmed: bool,
    /// Sent, and the venue has not listed it yet (being sent, or unconfirmed): not an order the
    /// venue can be asked to cancel.
    pub pending: bool,
    /// What the order has cost so far, in the quote asset.
    pub fees: Decimal,
}

/// One page of a list read from a venue, and where the next one starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

/// One execution of an order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FillRow {
    pub id: String,
    pub time_ms: i64,
    pub market: String,
    pub side: Side,
    /// In the market's price step.
    pub price: String,
    pub size: Decimal,
    pub fee: Decimal,
    /// The order it executed, as the venue names it.
    pub order: String,
}

/// What the account holds of one asset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceRow {
    pub asset: String,
    pub available: Decimal,
    /// Reserved by open orders.
    pub held: Decimal,
}

impl BalanceRow {
    pub fn total(&self) -> Decimal {
        self.available + self.held
    }
}

/// A sentence the panel shows about the connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub text: String,
    pub error: bool,
}

/// A working order as a chart's line shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChartOrder {
    pub id: String,
    pub side: Side,
    /// The short type on the line's tag ("LMT", "STP-LMT", "MKT").
    pub tag: &'static str,
    /// Where the line sits: a stop-limit rests at its stop until it triggers.
    pub price: Decimal,
    pub price_text: String,
    pub remaining_text: String,
    /// What is pending on the order ("cancel requested", "edit requested").
    pub note: Option<&'static str>,
    pub can_edit: bool,
    pub can_cancel: bool,
}

/// What a chart draws for one market of a venue: the price step and the working orders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChartLines {
    pub price_increment: Option<Decimal>,
    pub orders: Vec<ChartOrder>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_live_venue_is_real_money() {
        assert!(VenueKind::Live.is_real_money());
        assert!(!VenueKind::Test.is_real_money());
        assert!(!VenueKind::Simulated.is_real_money());
    }

    #[test]
    fn a_quote_without_a_book_falls_back_to_the_last_trade() {
        let quote = QuoteView {
            last: Some(Decimal::new(100, 0)),
            ..Default::default()
        };
        assert_eq!(quote.reference(Side::Buy), Some(Decimal::new(100, 0)));
        assert_eq!(quote.reference(Side::Sell), Some(Decimal::new(100, 0)));
        let book = QuoteView {
            bid: Some(Decimal::new(99, 0)),
            ask: Some(Decimal::new(101, 0)),
            last: Some(Decimal::new(100, 0)),
        };
        assert_eq!(book.reference(Side::Buy), Some(Decimal::new(101, 0)));
        assert_eq!(book.reference(Side::Sell), Some(Decimal::new(99, 0)));
    }
}
