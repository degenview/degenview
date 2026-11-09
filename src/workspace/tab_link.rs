//! A tab's side of Tab Linking (see [`crate::model::tab_link`]).
//!
//! What the user does in a linked tab is queued in an outbox at the places the user acts (the
//! symbol picker, the watchlist, the timeframe buttons). What the group tells the tab is applied
//! with the commands those same places call (`switch_chart`, `reload_for_timeframe`) and never
//! touches the outbox, so a delivery cannot be announced again: the loop guard is structural, not
//! a comparison of values.

use super::ChartWorkspace;
use crate::host::tab_link_host::TabLinkHost;
use crate::model::InstrumentId;
use crate::model::chart_sync::{ChartSyncSettings, SyncHub};
use crate::model::tab_link::{LinkKind, LinkOutbox, LinkSignal, Member};
use crate::model::watchlist::WatchlistInstrument;
use eframe::egui;

/// A symbol a group asked this tab to show, until it has proved it can load. If the first fetch
/// fails with nothing on the chart, the tab goes back to what it showed and says so; the rest of
/// the group is unaffected.
pub(super) struct Probation {
    chart_id: u64,
    /// The whole market (provider and symbol): `BTCUSDT` on Coinbase is not `BTCUSDT` on Binance.
    market: InstrumentId,
    previous: WatchlistInstrument,
}

/// What a tab remembers about its link beyond its colour.
#[derive(Default)]
pub(super) struct LinkState {
    pub(super) outbox: LinkOutbox,
    /// The newest delivery applied per kind: an older one is stale and dropped.
    applied: [u64; 3],
    pub(super) probation: Option<Probation>,
    /// The switches shared by every tab of a colour; handed in by the app.
    host: Option<TabLinkHost>,
    /// Carries the group's pan and zoom to the focused chart, the way a layout's own hub carries
    /// one chart's to the others. The group is its only origin ([`LINK_ORIGIN`]).
    pub(super) hub: SyncHub,
    /// What the hub was last told to carry; a change resets it so nothing stale is replayed.
    pub(super) view_settings: ChartSyncSettings,
}

/// The "chart" that navigated, as far as the hub is concerned: no real chart has this id.
const LINK_ORIGIN: u64 = u64::MAX;

fn slot(kind: LinkKind) -> usize {
    match kind {
        LinkKind::Symbol => 0,
        LinkKind::Interval => 1,
        LinkKind::View => 2,
    }
}

impl ChartWorkspace {
    /// The user put `item` on the focused chart. A tab with no colour says nothing.
    pub(super) fn announce_market(&mut self, item: &WatchlistInstrument) {
        if self.link.is_some() {
            self.link_state
                .outbox
                .push(LinkSignal::Market(Box::new(item.clone())));
        }
    }

    /// The user moved the tab to its current timeframe.
    pub(super) fn announce_timeframe(&mut self) {
        if self.link.is_some() {
            self.link_state
                .outbox
                .push(LinkSignal::Timeframe(self.timeframe));
        }
    }

    /// Gives the tab the shared switches. Cheap to call every frame.
    pub fn set_link_host(&mut self, host: &TabLinkHost) {
        if self.link_state.host.is_none() {
            self.link_state.host = Some(host.clone());
        }
    }

    /// Whether the user did something the app has not collected yet.
    pub fn has_link_signals(&self) -> bool {
        !self.link_state.outbox.is_empty()
    }

    /// What a pan or zoom carries across tabs for this tab: Time is the position, Date range the
    /// zoom. Nothing while the tab has no colour or its group syncs neither.
    pub(super) fn link_view_settings(&self) -> ChartSyncSettings {
        match (self.link, &self.link_state.host) {
            (Some(color), Some(host)) => host.toggles(color).viewport(),
            _ => ChartSyncSettings {
                horizontal_position: false,
                horizontal_zoom: false,
            },
        }
    }

    /// What the user did since the app last asked.
    pub fn take_link_signals(&mut self) -> Vec<LinkSignal> {
        self.link_state.outbox.take()
    }

    /// What the hub needs to know about this tab. The market row is only built for a tab that has
    /// a colour.
    pub fn link_member(&self) -> Member {
        Member {
            tab: self.id,
            color: self.link,
            market: self.link.and_then(|_| {
                self.focused_market_chart()
                    .map(|chart| Box::new(self.favorite_item(chart)))
            }),
            timeframe: self.timeframe,
        }
    }

    /// Applies what the group says. Returns whether anything on the tab changed.
    ///
    /// * The symbol goes to the focused market chart only, keeping its settings, indicators,
    ///   drawings and timeframe; a tab with no market chart (blank, or a portfolio card in
    ///   focus) has nothing to change.
    /// * The timeframe is the whole tab's.
    /// * A tab in replay takes neither: a symbol or timeframe change would move the replay's
    ///   timeline from under it. It catches up with the group's next change.
    /// * A delivery older than one already applied is dropped.
    pub fn apply_link(&mut self, sequence: u64, signal: &LinkSignal, ctx: &egui::Context) -> bool {
        let kind = signal.kind();
        if self.link.is_none() || sequence <= self.link_state.applied[slot(kind)] {
            return false;
        }
        let changed = match signal {
            LinkSignal::Market(item) => {
                if self.replay.is_bar_visible() {
                    return false;
                }
                let Some(chart) = self.focused_market_chart() else {
                    return false;
                };
                let chart_id = chart.id;
                let previous = self.favorite_item(chart);
                let shows_it = chart.source == item.instrument.source
                    && crate::model::InstrumentId::new(chart.source, &chart.symbol)
                        .is_same_market(&item.instrument);
                if shows_it {
                    false
                } else {
                    self.switch_chart(chart_id, item, ctx);
                    self.link_state.probation = Some(Probation {
                        chart_id,
                        market: item.instrument.clone(),
                        previous,
                    });
                    true
                }
            }
            LinkSignal::Timeframe(timeframe) => {
                if self.replay.is_bar_visible() || self.timeframe == *timeframe {
                    false
                } else {
                    self.reload_for_timeframe(*timeframe, ctx);
                    true
                }
            }
            // The focused chart follows it before it next draws (`follow_sync`), once it has bars:
            // a chart still loading keeps the event until there is something to move.
            LinkSignal::View(change) => {
                // An event under other switches than the last one starts the hub afresh, so a
                // half-finished zoom of the old kind is never finished under the new.
                let settings = self.link_view_settings();
                if settings != self.link_state.view_settings {
                    self.link_state.view_settings = settings;
                    self.link_state.hub.reset();
                }
                self.link_state.hub.publish(LINK_ORIGIN, *change);
                true
            }
        };
        self.link_state.applied[slot(kind)] = sequence;
        if changed {
            ctx.request_repaint();
        }
        changed
    }

    /// Watches a symbol the group asked for: once it has candles it is accepted; if its first
    /// fetch fails with none, the chart goes back to what it showed. Called every frame, after
    /// the fetches were received.
    pub(super) fn tick_link_probation(&mut self, ctx: &egui::Context) {
        let Some(probation) = &self.link_state.probation else {
            return;
        };
        let Some(chart) = self
            .charts
            .iter()
            .find(|chart| chart.id == probation.chart_id)
        else {
            self.link_state.probation = None;
            return;
        };
        if !InstrumentId::new(chart.source, &chart.symbol).is_same_market(&probation.market) {
            // The user moved on before it settled.
            self.link_state.probation = None;
        } else if !chart.candles.is_empty() {
            self.link_state.probation = None;
        } else if chart.error.is_some() && !chart.loading {
            let Some(Probation {
                chart_id,
                market,
                previous,
            }) = self.link_state.probation.take()
            else {
                return;
            };
            // Not announced: this is putting back what the tab had, not a choice of the user's.
            self.switch_chart(chart_id, &previous, ctx);
            self.say(
                format!(
                    "Couldn\u{2019}t load {}. This tab kept {}.",
                    market.symbol, previous.name
                ),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests;
