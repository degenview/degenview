//! The one interface every venue implements for the trading screens.
//!
//! The order sheet, the orders panel and the chart pills are written against this trait, so adding
//! a broker adds an adapter and nothing in the screens. Reading is synchronous and cheap (an
//! `Arc` snapshot or a locked read); a screen calls these every frame. Anything that waits on a
//! network is a command with a handle that is polled, and arrives with the order entry.

use degenview_brokers::{Decimal, MarketRef, Side};

use super::draft::{CancelResult, DraftCheck, PreviewResult, SubmitResult, TradeDraft};
use super::model::{
    AccountStats, AccountStrip, Amount, BackendState, BalanceRow, FillRow, MarketAccess, OrderRow,
    PositionRow, QuoteView, TabKind, VenueInfo,
};
use super::task::Task;

pub trait TradingBackend: Send + Sync {
    fn info(&self) -> VenueInfo;

    fn state(&self) -> BackendState;

    /// The account the orders go to; `None` while there is none to show.
    fn account(&self) -> Option<AccountStrip>;

    /// The account's numbers for the strip at the top of the orders panel. Calling it says the
    /// prices behind them are wanted, so call it every frame the strip is drawn.
    fn stats(&self) -> Option<AccountStats>;

    /// What can be spent on an order of `side` in `market`: the quote asset to buy, the base asset
    /// to sell (a venue that nets positions counts its free funds either way).
    fn available(&self, market: &MarketRef, side: Side) -> Option<Amount>;

    /// Whether the chart's market is this venue's and what may be asked of it.
    fn market(&self, market: &MarketRef) -> MarketAccess;

    /// The market's best prices. Calling it says something on screen wants them, so a venue that
    /// reads them on demand keeps doing so; call it every frame they are drawn.
    fn quote(&self, market: &MarketRef) -> Option<QuoteView>;

    /// The tabs of the orders panel this venue has data for, in order.
    fn tabs(&self) -> Vec<TabKind>;

    /// Every order the venue knows of, working and finished, newest first.
    fn orders(&self) -> Vec<OrderRow>;

    fn positions(&self) -> Vec<PositionRow>;

    fn fills(&self) -> Vec<FillRow>;

    fn balances(&self) -> Vec<BalanceRow>;

    /// What is wrong with the order as typed, and what it would cost as far as can be known without
    /// asking the venue. Cheap: called every frame.
    fn check(&self, market: &MarketRef, draft: &TradeDraft) -> DraftCheck;

    /// The largest size of `side` in `market` the account can afford at `price` (the reference price
    /// when `None`), a whole number of the market's size increments.
    fn max_size(&self, market: &MarketRef, side: Side, price: Option<Decimal>) -> Option<Decimal>;

    /// Asks the venue what the order would do, without placing it. The draft must be complete.
    fn preview(
        &self,
        market: &MarketRef,
        draft: &TradeDraft,
    ) -> Task<Result<PreviewResult, String>>;

    /// Sends the order. The draft must be complete.
    fn submit(&self, market: &MarketRef, draft: &TradeDraft) -> Task<SubmitResult>;

    /// Cancels the working order with this id (an [`OrderRow::id`](super::model::OrderRow::id)).
    fn cancel(&self, order_id: &str) -> Task<CancelResult>;

    /// Rises whenever anything above may have changed, so a screen can skip rebuilding rows.
    fn revision(&self) -> u64;
}
