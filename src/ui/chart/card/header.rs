//! The header row drawn over the top of the plot: the market and its prices, the change in view,
//! the indicator legend and the header's buttons.

use super::*;
use crate::i18n::tr;

impl ChartCard {
    /// Header row. Reordering has its own handle so clicking the market title
    /// never moves its icon and text into egui's drag preview layer.
    #[allow(clippy::too_many_arguments)]
    pub fn header(
        &self,
        ui: &mut egui::Ui,
        symbol: &str,
        source: DataSource,
        indicators: &IndicatorConfig,
        candles: &[Kline],
        replaying: bool,
        is_favorite: bool,
        watchlists: &[WatchlistMembership],
        action: &mut ChartCardAction,
        drag_id: u64,
    ) {
        let header_width = ui.available_width();
        let price_change = self.visible_move(candles);
        // The change sits beside the price now, so the right end holds the star and little else.
        let paper_price = if self.paper.trading_enabled {
            candles
                .last()
                .map(|last| self.appearance.price_format().format(last.close))
        } else {
            None
        };
        let live_pills = self.live.pills_visible();
        let reserved = 108.0
            + paper_price.as_deref().map_or(0.0, |price_text| {
                chart_trading::quick_trade_width(ui, price_text) + 8.0
            })
            + if live_pills {
                crate::ui::broker::chart_trading::quick_trade_width(
                    ui,
                    self.live.bid.as_deref(),
                    self.live.ask.as_deref(),
                ) + 8.0
            } else {
                0.0
            }
            // The star and the gap after it.
            + STAR_END
            // And what the pills need beyond the star to stay clear of the price axis.
            + if paper_price.is_some() || live_pills {
                (self.axis_clearance() - STAR_END).max(0.0)
            } else {
                0.0
            };
        let title_block_width = (header_width - reserved).max(96.0);
        let title_content_width = title_block_width - 18.0;
        let indicator_space = if indicator_strip::has_items(indicators, self) {
            35.0
        } else {
            0.0
        };
        let show_replay_badge = replaying && header_width >= 430.0;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.dnd_drag_source(egui::Id::new(("chart-drag", drag_id)), drag_id, |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 20.0), Sense::hover());
                    let color = theme::text_secondary().gamma_multiply(0.7);
                    for x in [-2.0, 2.0] {
                        for y in [-4.0, 0.0, 4.0] {
                            ui.painter()
                                .circle_filled(rect.center() + Vec2::new(x, y), 1.0, color);
                        }
                    }
                })
                .response
                .on_hover_text(tr!("Drag to reorder charts"));
                ui.vertical(|ui| {
                    ui.set_width(title_content_width);
                    ui.spacing_mut().item_spacing.y = -3.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon_rect, icon_response) =
                            ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                        crate::ui::kit::chart_icon_with_label(
                            ui,
                            icon_rect,
                            symbol,
                            self.title.as_deref().unwrap_or(symbol),
                            source,
                            true,
                        );
                        if icon_response.settings_tip(source).clicked() {
                            action.settings_requested = source != DataSource::CoinMarketCap;
                        }
                        let title_max_width = (title_content_width
                            - 26.0
                            - indicator_space
                            - if show_replay_badge { 72.0 } else { 0.0 })
                        .max(20.0);
                        if ui
                            .scope(|ui| {
                                ui.set_max_width(title_max_width);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(self.display_title(symbol, source))
                                            .size(13.0)
                                            .strong(),
                                    )
                                    .truncate()
                                    .sense(Sense::click()),
                                )
                            })
                            .inner
                            .settings_tip(source)
                            .clicked()
                        {
                            action.settings_requested = source != DataSource::CoinMarketCap;
                        }
                        if show_replay_badge {
                            // The size and fill of an indicator pill (chart_indicator_strip).
                            let label = crate::ui::kit::galley(
                                ui,
                                tr!("Replay"),
                                10.0,
                                crate::ui::kit::MEDIUM,
                                crate::ui::kit::primary(0.7),
                                f32::INFINITY,
                            );
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(8.0 + 9.0 + 4.0 + label.size().x + 8.0, 20.0),
                                Sense::hover(),
                            );
                            ui.painter()
                                .rect_filled(rect, 10.0, crate::ui::kit::primary(0.06));
                            crate::ui::icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    Pos2::new(rect.left() + 8.0 + 4.5, rect.center().y),
                                    Vec2::splat(9.0),
                                ),
                                crate::ui::icons::Icon::ClockArrow,
                                theme::REPLAY,
                            );
                            ui.painter().galley(
                                Pos2::new(
                                    rect.left() + 21.0,
                                    rect.center().y - label.size().y / 2.0,
                                ),
                                label,
                                crate::ui::kit::primary(0.7),
                            );
                            response.on_hover_text(tr!("Historical replay"));
                        }
                        // CoinMarketCap's indices aren't price series: nothing to overlay.
                        if source != DataSource::CoinMarketCap {
                            // Salted per chart: the strip keys its hover state by ui id, and
                            // identical ids across charts made one chart's chips light up another's.
                            action.indicator_action = ui
                                .push_id(("indicator-strip", drag_id), |ui| {
                                    indicator_strip::show(ui, self, indicators, drag_id)
                                })
                                .inner;
                        }
                    });
                    if let Some(last) = candles.last() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let price = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(
                                            if source == DataSource::CoinMarketCap {
                                                // An index on a 0–100 scale, not a price.
                                                format!("{:.0} / 100", last.close)
                                            } else if let Some((label, price)) =
                                                self.leading_choice()
                                            {
                                                tr!(
                                                    "{label} · {price}",
                                                    label = label,
                                                    price = format::headline(price, true)
                                                )
                                            } else {
                                                format::headline(
                                                    last.close,
                                                    source.is_prediction_market(),
                                                )
                                            },
                                        )
                                        .size(11.0)
                                        .color(theme::text_secondary()),
                                    )
                                    .frame(false),
                                )
                                .settings_tip(source);
                            if price.clicked() {
                                action.settings_requested = source != DataSource::CoinMarketCap;
                            }
                            // The change over the bars on screen, beside the price it moved:
                            // in price and percent where a price difference means something,
                            // and the percent alone where it does not or the row is too tight.
                            if let Some((amount, percent)) = price_change {
                                let color = if percent >= 0.0 {
                                    theme::UP
                                } else {
                                    theme::DOWN
                                };
                                let room = title_content_width - price.rect.width() - 6.0;
                                let wide = !source.is_prediction_market()
                                    && source != DataSource::CoinMarketCap;
                                let measure = |text: &str| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            text.to_owned(),
                                            egui::FontId::proportional(11.0),
                                            color,
                                        )
                                        .size()
                                        .x
                                };
                                let full = change_text(wide.then_some(amount), percent);
                                let short = change_text(None, percent);
                                let text = if measure(&full) <= room {
                                    Some(full)
                                } else if measure(&short) <= room {
                                    Some(short)
                                } else {
                                    None
                                };
                                if let Some(text) = text {
                                    ui.label(egui::RichText::new(text).color(color).size(11.0));
                                }
                            }
                        });
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if source == DataSource::CoinMarketCap
                        && icons::button(
                            ui,
                            Icon::Xmark,
                            27.0,
                            theme::text_secondary(),
                            tr!("Remove Chart"),
                        )
                        .clicked()
                    {
                        action.remove_requested = true;
                    }
                    let favorite_icon = if is_favorite {
                        Icon::StarFill
                    } else {
                        Icon::Star
                    };
                    let favorite_color = if is_favorite {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    };
                    let favorite_tip = if is_favorite {
                        tr!("Remove from watchlist")
                    } else {
                        tr!("Add to watchlist")
                    };
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // At the header's top, not centred in it: the star then sits the same distance
                    // from the card's top edge as from its right edge.
                    let favorite = ui
                        .allocate_ui_with_layout(
                            Vec2::new(27.0, HEADER_HEIGHT),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                // The star's points sit low in its square and its sides inset
                                // from it: lift it until the two margins read the same.
                                ui.add_space(-3.5);
                                icons::button(ui, favorite_icon, 27.0, favorite_color, favorite_tip)
                            },
                        )
                        .inner;
                    favorite.context_menu(|ui| {
                        ui.label(egui::RichText::new(tr!("Add to Watchlist")).small().weak());
                        for list in watchlists {
                            let mut included = list.contains_market;
                            if ui.checkbox(&mut included, &list.name).changed() {
                                action.watchlist_change = Some((list.id, included));
                                ui.close();
                            }
                        }
                    });
                    // The header is drawn over the plot, whose crosshair cursor would show here.
                    favorite
                        .clone()
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if favorite.clicked() {
                        action.toggle_favorite = true;
                    }
                    if paper_price.is_some() || self.live.pills_visible() {
                        // The header is drawn over the plot, so the pills would cover the price
                        // axis: move them left until they end a gap before it.
                        let used = ui.max_rect().right() - ui.cursor().max.x;
                        let missing = self.axis_clearance() - used;
                        if missing > 0.0 {
                            ui.add_space(missing);
                        }
                    }
                    if let Some(price_text) = &paper_price
                        && let Some(side) = pills_beside_star(
                            ui,
                            chart_trading::quick_trade_width(ui, price_text),
                            |ui| chart_trading::quick_trade_buttons(ui, price_text),
                        )
                    {
                        action.quick_trade = Some(side);
                    }
                    if let Some(entry) = self.live.order_entry
                        // Live pills: the broker's own bid and ask, never the chart's last price.
                        && let Some(side) = pills_beside_star(
                            ui,
                            crate::ui::broker::chart_trading::quick_trade_width(
                                ui,
                                self.live.bid.as_deref(),
                                self.live.ask.as_deref(),
                            ),
                            |ui| {
                                crate::ui::broker::chart_trading::quick_trade_buttons(
                                    ui,
                                    &self.live.broker_name,
                                    self.live.bid.as_deref(),
                                    self.live.ask.as_deref(),
                                    entry,
                                )
                            },
                        )
                    {
                        action.live_trade = Some(side);
                    }
                });
            },
        );
    }
}

/// How far from the header's right edge the star's own room reaches: its width and the gap after.
const STAR_END: f32 = 27.0 + 4.0;

/// The gap between the trade pills and the price axis they must stay left of.
const AXIS_GAP: f32 = 8.0;

impl ChartCard {
    /// How far from the header's right edge the trade pills must end: the price axis the header is
    /// drawn over, and a gap. Zero before a plot has been drawn, which leaves the pills at the star.
    fn axis_clearance(&self) -> f32 {
        if self.axis_width > 0.0 {
            self.axis_width + AXIS_GAP
        } else {
            0.0
        }
    }
}

/// Draws a group of quick-trade pills in the header's right-to-left end, level with the favourite
/// star: the star is pinned to the header's top, so the pills sit at the top too, lifted until
/// their centre meets the star's (star 27 tall at -3.5, pill 24 tall at -2.0: both centre at 10).
/// `width` is the pills' own, so the group takes exactly its room and nothing after it is pushed.
fn pills_beside_star<R>(
    ui: &mut egui::Ui,
    width: f32,
    pills: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(width, HEADER_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.add_space(-2.0);
            pills(ui)
        },
    )
    .inner
}

/// `+$67.13 (+0.10%)`, or `+0.10%` when the amount is left out (prediction markets quote
/// probabilities, so a price difference means nothing there).
pub(super) fn change_text(amount: Option<f64>, percent: f64) -> String {
    let sign = if percent >= 0.0 { "+" } else { "-" };
    match amount {
        Some(amount) => format!(
            "{sign}{} ({sign}{:.2}%)",
            format::headline(amount.abs(), false),
            percent.abs()
        ),
        None => format!("{sign}{:.2}%", percent.abs()),
    }
}

/// The "Chart settings" hover hint, except where a chart has no settings to open.
trait SettingsTip {
    fn settings_tip(self, source: DataSource) -> Self;
}

impl SettingsTip for egui::Response {
    fn settings_tip(self, source: DataSource) -> Self {
        if source == DataSource::CoinMarketCap {
            self
        } else {
            // The plot's crosshair cursor is set before the header is drawn over it.
            self.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tr!("Chart settings"))
        }
    }
}
