//! What the sheet holds between frames: the typed order, the stage it is in, the message under the
//! button, and how a send, a review or a cancel moves it on.

use std::time::Instant;

use degenview_brokers::{Decimal, MarketRef, Side};
#[cfg(test)]
use eframe::egui;

use crate::model::broker::venue::TradingVenue;
use crate::trading::backend::TradingBackend;
use crate::trading::draft::{CancelResult, SubmitResult, TradeDraft};
use crate::trading::model::{Binding, OrderClass};
use crate::trading::preview::PreviewSlot;
use crate::trading::task::Task;
use degenview_brokers::model::preview::PreviewGate;

pub(super) enum Stage {
    Editing,
    /// The review dialog is open, for the account and connection the figures in it were worked
    /// out against. The order goes only there.
    Confirming(Binding),
    Sending(Task<SubmitResult>),
}

pub struct SheetState {
    pub draft: TradeDraft,
    /// The market the typed prices were for; a different chart starts the prices over.
    pub(super) market: Option<MarketRef>,
    pub(super) venue: Option<TradingVenue>,
    pub(super) prefilled: bool,
    /// The prices in the draft came from a click and have not been put on the tick yet.
    pub(super) snap_prices: bool,
    pub(super) preview: PreviewSlot,
    pub(super) stage: Stage,
    pub(super) notice: Option<(String, bool, Instant)>,
    pub(super) cancels: Vec<Task<CancelResult>>,
    /// Whether the open-orders list is unfolded; none until the user folds it either way.
    pub(super) orders_open: Option<bool>,
    /// Where the Buy/Sell button was drawn last frame, for the tests that press it.
    #[cfg(test)]
    pub(crate) button: Option<egui::Rect>,
    #[cfg(test)]
    pub(super) button_enabled: bool,
}

impl Default for SheetState {
    fn default() -> Self {
        Self {
            draft: TradeDraft::new(Side::Buy),
            market: None,
            venue: None,
            prefilled: false,
            snap_prices: false,
            preview: PreviewSlot::default(),
            stage: Stage::Editing,
            notice: None,
            cancels: Vec::new(),
            orders_open: None,
            #[cfg(test)]
            button: None,
            #[cfg(test)]
            button_enabled: false,
        }
    }
}

impl SheetState {
    /// Starts an order from somewhere else (a header pill, a click on the chart): the side, and a
    /// price where there is one. What was typed for another order is replaced.
    pub fn start(&mut self, side: Side, class: Option<OrderClass>, price: Option<Decimal>) {
        let class = class.unwrap_or(self.draft.class);
        self.draft = match price {
            Some(price) => TradeDraft::at_price(side, class, price),
            None => {
                let mut draft = TradeDraft::new(side);
                draft.class = class;
                draft
            }
        };
        self.prefilled = price.is_some();
        // A clicked price is a raw float: it goes onto the market's tick once the market is known.
        self.snap_prices = price.is_some();
        self.stage = Stage::Editing;
    }

    /// Whether the button was pressable when it was last drawn.
    #[cfg(test)]
    pub(crate) fn button_enabled_for_test(&self) -> bool {
        self.button_enabled
    }

    /// The prices are typed, so the sheet need not fill one in from the market.
    #[cfg(test)]
    pub(crate) fn mark_prefilled_for_test(&mut self) {
        self.prefilled = true;
    }

    /// Whether the venue's preview failed (it could not be asked, or it errored).
    #[cfg(test)]
    pub(crate) fn preview_failed(&self) -> bool {
        matches!(self.preview.gate(Instant::now()), PreviewGate::Failed(_))
    }

    /// Asks the venue again (the "Try again" button).
    #[cfg(test)]
    pub(crate) fn retry_preview(&mut self) {
        self.preview.retry(Instant::now());
    }

    /// Whether the review dialog is open.
    #[cfg(test)]
    pub(crate) fn is_reviewing(&self) -> bool {
        matches!(self.stage, Stage::Confirming(_))
    }

    /// Whether an order is on its way to the venue.
    #[cfg(test)]
    pub(crate) fn is_sending(&self) -> bool {
        matches!(self.stage, Stage::Sending(_))
    }

    #[cfg(test)]
    pub(crate) fn notice_text(&self) -> Option<&str> {
        self.notice.as_ref().map(|(text, _, _)| text.as_str())
    }

    /// Closes the review dialog without sending (a replay began). True when one was open.
    pub fn close_review(&mut self) -> bool {
        let open = matches!(self.stage, Stage::Confirming(_));
        if open {
            self.stage = Stage::Editing;
        }
        open
    }

    /// Closes the review dialog because what it describes is no longer there (not connected).
    pub(super) fn drop_review(&mut self) {
        if matches!(self.stage, Stage::Confirming(_)) {
            self.stage = Stage::Editing;
        }
    }

    /// Points the sheet at `market` on `venue`. Prices typed for another market or venue mean
    /// nothing here, so they are dropped; the same market and venue leave it as it is.
    pub(super) fn follow(&mut self, market: &MarketRef, venue: TradingVenue) {
        if self.market.as_ref() == Some(market) && self.venue == Some(venue) {
            return;
        }
        if self.market.is_some() {
            self.draft.price_text.clear();
            self.draft.stop_text.clear();
            self.draft.size_text.clear();
            self.draft.take_profit_text.clear();
            self.draft.stop_loss_text.clear();
        }
        self.market = Some(market.clone());
        self.venue = Some(venue);
        self.prefilled = false;
        self.stage = Stage::Editing;
    }

    pub(super) fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    /// Ends a review that no longer describes the order. It stands only while the order would still
    /// go where it was reviewed (if the account or the connection changed, the figures are for
    /// somewhere else), and on an answer that is still good (one that went stale while the dialog
    /// was open is not).
    pub(super) fn expire_review(&mut self, backend: &dyn TradingBackend, gate: &PreviewGate) {
        let Stage::Confirming(reviewed) = &self.stage else {
            return;
        };
        let text = if backend.binding() != *reviewed {
            "The account changed since you set this order up. Nothing was sent. Check the order again."
        } else if !gate.allows_review() {
            "The check of this order is out of date. Nothing was sent. Check the order again."
        } else {
            return;
        };
        self.stage = Stage::Editing;
        self.say(text, true);
    }

    /// Takes the venue's answer to a send, when it has one.
    pub(super) fn take_send_result(&mut self) {
        let Stage::Sending(task) = &mut self.stage else {
            return;
        };
        let Some(result) = task.poll().cloned() else {
            return;
        };
        self.stage = Stage::Editing;
        match result {
            Ok(_) => {
                // The order is on its way: what was typed for it is done with, the side and prices stay.
                self.draft.size_text.clear();
                self.draft.take_profit_text.clear();
                self.draft.stop_loss_text.clear();
                self.say("Order placed.", false);
            }
            Err(text) => self.say(text, true),
        }
    }

    /// Takes the answers to cancels that have one, and says the ones that failed.
    pub(super) fn poll_cancels(&mut self) {
        let mut said = Vec::new();
        self.cancels.retain_mut(|task| match task.poll() {
            Some(Err(text)) => {
                said.push(text.clone());
                false
            }
            Some(Ok(())) => false,
            None => true,
        });
        for text in said {
            self.say(text, true);
        }
    }
}
