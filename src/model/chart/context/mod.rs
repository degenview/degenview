//! The chart's right-click menus as data: what was clicked, what each menu offers, and whether
//! it is still true when an entry is chosen.
//!
//! Nothing here touches egui, a store or an engine. A menu is a `Vec<Entry>` built from a
//! [`ChartContext`] (the snapshot taken at the click) and [`Facts`] (the chart's current state);
//! the UI draws it and hands back the chosen [`Command`], and the workspace, which owns every
//! mutation, [`check`]s the snapshot against what is live before it acts. The price and time of
//! the click are captured once, so moving the pointer to an entry never changes them.

use crate::i18n::{tr, tr_n};
use crate::model::alert::{AlertId, AlertState};
use crate::model::broker::chart::Unavailable as LiveUnavailable;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::paper::{PaperOrderSide, PaperOrderType};
use crate::model::pine::instance::VisualOrder;
use crate::model::{DataSource, PriceFormat};
use degenview_brokers::model::ticket::TicketKind;

/// What the right-click landed on. Drawings keep their own menu (`drawing_layer`), which opens
/// first when a drawing is under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The plot with nothing interactive under it.
    Background,
    PriceAxis,
    TimeAxis,
    /// A script's own pane (`instance` is the Pine instance id).
    Pane {
        instance: u64,
    },
    Alert {
        id: AlertId,
    },
    Order {
        id: u64,
    },
    Position {
        key: String,
    },
}

/// Everything a menu needs to remember about the click. Taken once; never recomputed from the
/// pointer's later position.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartContext {
    pub chart_id: u64,
    pub symbol: String,
    pub source: DataSource,
    /// The time under the click, when it was over the time scale or the plot.
    pub time_ms: Option<i64>,
    /// The price under the click, when it was over the price scale or the main plot.
    pub price: Option<f64>,
    pub target: Target,
}

impl ChartContext {
    /// The captured price, only when it is a number worth acting on.
    pub fn price(&self) -> Option<f64> {
        self.price.filter(|price| price.is_finite())
    }
}

/// Whether a chart can trade on paper. Having no account yet is not a reason to refuse: choosing
/// an order entry creates the default one, as opening the paper panel does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperAccess {
    Available,
    /// This chart has no market to trade (a portfolio card, CoinMarketCap).
    Unsupported,
}

/// What the menu needs to offer live orders on this chart (a broker is the active venue).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveFacts {
    /// The broker's name, for the menu's labels.
    pub broker_name: String,
    /// Why orders cannot be placed from this chart now; `None` when they can.
    pub unavailable: Option<LiveUnavailable>,
    /// The market's price step, to put a clicked price on it.
    pub price_increment: Option<degenview_brokers::Decimal>,
    pub market: bool,
    pub limit: bool,
    pub stop_limit: bool,
    /// The chart's prices are constructed (Renko, Heikin Ashi...): a click on one is not a price the
    /// market trades at.
    pub constructed: bool,
}

/// A live order the menu asks the ticket to open. The ticket, not the menu, places it, after the
/// user has seen exactly what will be sent.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveOrderRequest {
    pub side: degenview_brokers::Side,
    pub kind: TicketKind,
    pub price: Option<degenview_brokers::Decimal>,
}

/// One quick order the clicked price suggests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuickOrder {
    pub side: PaperOrderSide,
    pub kind: PaperOrderType,
}

/// A saved or built-in appearance template, as the submenu lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateRow {
    pub id: u64,
    pub name: String,
    /// One of the presets that ship with the app, rather than one the user saved.
    pub built_in: bool,
    /// Applying it would change nothing: it is the chart's look as it stands.
    pub current: bool,
}

/// What a script pane's menu needs to know about its instance.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneFacts {
    pub name: String,
    /// The script names chart points that "Reset points" can ask for again.
    pub has_points: bool,
    pub visible: bool,
    pub order: VisualOrder,
}

/// What an order pill's menu needs to know about its order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderFacts {
    /// Still working: a filled or cancelled order cannot be cancelled.
    pub cancelable: bool,
}

/// The chart's current state, for labels and eligibility.
#[derive(Clone, Debug)]
pub struct Facts {
    pub price_format: PriceFormat,
    /// The chart's last price (replay-cut), what a pending order is placed relative to.
    pub last_price: Option<f64>,
    /// The instrument's price step; the market is "at" a price within half of it.
    pub tick: f64,
    pub replay: bool,
    pub alerts_supported: bool,
    pub paper: PaperAccess,
    /// Live trading at a broker, when one is the active venue (then paper entries are not offered).
    pub live: Option<LiveFacts>,
    /// The ticket's default quantity text, empty when it has none (never assumed to be 1).
    pub quantity: String,
    /// Drawings a Remove would delete (the user's, not the ruler).
    pub drawings: usize,
    /// Applied built-in indicators plus script instances.
    pub indicators: usize,
    /// Whether a valid drawing copy is waiting to be pasted.
    pub clipboard: bool,
    pub time_locked: bool,
    pub table_open: bool,
    pub tree_open: bool,
    pub stats_open: bool,
    /// The chart's price-scale mode, which the price-axis menu marks.
    pub scale_mode: PriceScaleMode,
    /// Whether the chart's last-price tag carries the countdown to the bar's close.
    pub bar_countdown: bool,
    /// Whether the chart has bars that close on a clock (see `countdown::eligible`); the menu
    /// entry is offered greyed out where it has none.
    pub bar_countdown_eligible: bool,
    pub templates: Vec<TemplateRow>,
    pub pane: Option<PaneFacts>,
    pub alert: Option<AlertState>,
    pub order: Option<OrderFacts>,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            price_format: PriceFormat::AUTO,
            last_price: None,
            tick: 0.0,
            replay: false,
            alerts_supported: true,
            paper: PaperAccess::Available,
            live: None,
            quantity: String::new(),
            drawings: 0,
            indicators: 0,
            clipboard: false,
            time_locked: false,
            table_open: false,
            tree_open: false,
            stats_open: false,
            scale_mode: PriceScaleMode::Regular,
            bar_countdown: true,
            bar_countdown_eligible: true,
            templates: Vec::new(),
            pane: None,
            alert: None,
            order: None,
        }
    }
}

/// Which keyboard shortcut an entry shows; the UI turns it into the platform's label from the
/// shortcut table, so a hint is only ever printed for a binding that exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hint {
    Paste,
}

/// What choosing an entry asks the workspace to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    CopyPrice,
    Paste,
    AddAlert,
    QuickOrder(QuickOrder),
    AddOrder,
    /// Open the live order ticket at a broker, filled in from the click.
    LiveOrder(LiveOrderRequest),
    ToggleTimeLock,
    ToggleTable,
    ToggleObjectTree,
    ToggleSessionStats,
    ApplyTemplate(u64),
    SaveTemplate,
    ManageTemplates,
    ResetTemplate,
    RemoveDrawings,
    RemoveIndicators,
    OpenSettings,
    ResetPriceScale,
    SetScaleMode(PriceScaleMode),
    ToggleBarCountdown,
    ResetChartView,
    PaneSettings(u64),
    /// Ask for every point of a pane's script again.
    PaneResetPoints(u64),
    PaneToggle(u64),
    PaneRemove(u64),
    PaneOrder(u64, VisualOrder),
    OpenPineStudio,
    /// Open the Pine logs pane on this script.
    PineLogs(u64),
    EditAlert(AlertId),
    ToggleAlert(AlertId),
    DeleteAlert(AlertId),
    CancelOrder(u64),
    ClosePosition(String),
    ShowPaperPanel,
}

/// The glyph of a row that cannot be told from its command (a submenu has none).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Glyph {
    Template,
    Layers,
    Pause,
    Play,
}

/// One row of a menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What it does; `None` on a row that only opens its submenu.
    pub command: Option<Command>,
    pub label: String,
    pub hint: Option<Hint>,
    /// Greyed out; the reason is the tooltip and the screen reader's description.
    pub disabled: Option<&'static str>,
    pub checked: bool,
    /// The current choice of its group (the applied template): marked at the trailing edge.
    pub current: bool,
    pub glyph: Option<Glyph>,
    pub destructive: bool,
    pub children: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item(Item),
    Separator,
    /// A small label over the rows that follow; not a row.
    Heading(String),
}

impl Item {
    fn new(command: Command, label: impl Into<String>) -> Self {
        Self {
            command: Some(command),
            label: label.into(),
            hint: None,
            disabled: None,
            checked: false,
            current: false,
            glyph: None,
            destructive: false,
            children: Vec::new(),
        }
    }

    fn submenu(label: impl Into<String>, children: Vec<Entry>) -> Self {
        Self {
            command: None,
            label: label.into(),
            hint: None,
            disabled: None,
            checked: false,
            current: false,
            glyph: None,
            destructive: false,
            children,
        }
    }

    fn hint(mut self, hint: Hint) -> Self {
        self.hint = Some(hint);
        self
    }

    fn disabled_unless(mut self, enabled: bool, reason: &'static str) -> Self {
        if !enabled {
            self.disabled = Some(reason);
        }
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    fn glyph(mut self, glyph: Glyph) -> Self {
        self.glyph = Some(glyph);
        self
    }

    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.disabled.is_none()
    }
}

impl Entry {
    fn item(item: Item) -> Self {
        Self::Item(item)
    }
}

// The reasons a row is greyed out. Functions, not constants, because they are put in the language
// in use when the menu is built.
pub fn in_replay() -> &'static str {
    tr!("Not available during replay")
}

fn no_bar_countdown() -> &'static str {
    tr!("This chart has no bars that close on a clock")
}

fn no_price() -> &'static str {
    tr!("No price at this point")
}

fn no_market() -> &'static str {
    tr!("Price is at the market")
}

fn nothing_to_paste() -> &'static str {
    tr!("No copied drawings to paste")
}

fn nothing_to_remove() -> &'static str {
    tr!("Nothing to remove")
}

fn order_not_working() -> &'static str {
    tr!("The order is no longer working")
}

// MARK: Labels

pub fn remove_drawings_label(count: usize) -> String {
    if count == 0 {
        tr!("Remove drawings").to_owned()
    } else {
        tr_n!(count, "Remove {count} drawing", "Remove {count} drawings")
    }
}

pub fn remove_indicators_label(count: usize) -> String {
    if count == 0 {
        tr!("Remove indicators").to_owned()
    } else {
        tr_n!(
            count,
            "Remove {count} indicator",
            "Remove {count} indicators"
        )
    }
}

/// A quick order's menu row: "Sell 0.5 BTC @ 70,000.00 limit". The quantity, when there is one,
/// carries its own trailing space.
fn quick_order_label(order: QuickOrder, quantity: &str, symbol: &str, price: &str) -> String {
    match (order.side, order.kind) {
        (PaperOrderSide::Sell, PaperOrderType::Limit) => tr!(
            "Sell {quantity}{symbol} @ {price} limit",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Buy, PaperOrderType::Stop) => tr!(
            "Buy {quantity}{symbol} @ {price} stop",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Buy, PaperOrderType::Limit) => tr!(
            "Buy {quantity}{symbol} @ {price} limit",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Sell, PaperOrderType::Stop) => tr!(
            "Sell {quantity}{symbol} @ {price} stop",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Buy, PaperOrderType::StopLimit) => tr!(
            "Buy {quantity}{symbol} @ {price} stop limit",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Sell, PaperOrderType::StopLimit) => tr!(
            "Sell {quantity}{symbol} @ {price} stop limit",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Buy, PaperOrderType::Market) => tr!(
            "Buy {quantity}{symbol} @ {price} market",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
        (PaperOrderSide::Sell, PaperOrderType::Market) => tr!(
            "Sell {quantity}{symbol} @ {price} market",
            quantity = quantity,
            symbol = symbol,
            price = price
        ),
    }
}

/// The greyed row shown while no order is suggested: "Sell BTC limit".
fn idle_order_label(order: QuickOrder, symbol: &str) -> String {
    match order.side {
        PaperOrderSide::Sell => tr!("Sell {symbol} limit", symbol = symbol),
        PaperOrderSide::Buy => tr!("Buy {symbol} stop", symbol = symbol),
    }
}

// MARK: Order suggestions

/// The pending orders a price suggests, relative to the market. Above the market a sell limit
/// takes profit and a buy stop breaks out; below it a buy limit buys the dip and a sell stop
/// cuts a loss. At the market (within half a tick) a pending order would fill at once, so none
/// is suggested.
pub fn suggest_orders(clicked: f64, last: f64, tick: f64) -> Option<[QuickOrder; 2]> {
    if !clicked.is_finite() || !last.is_finite() {
        return None;
    }
    let slack = if tick.is_finite() && tick > 0.0 {
        tick / 2.0
    } else {
        0.0
    };
    let gap = clicked - last;
    if gap.abs() <= slack || gap == 0.0 {
        return None;
    }
    let order = |side, kind| QuickOrder { side, kind };
    Some(if gap > 0.0 {
        [
            order(PaperOrderSide::Sell, PaperOrderType::Limit),
            order(PaperOrderSide::Buy, PaperOrderType::Stop),
        ]
    } else {
        [
            order(PaperOrderSide::Buy, PaperOrderType::Limit),
            order(PaperOrderSide::Sell, PaperOrderType::Stop),
        ]
    })
}

/// Which way the price has to travel to reach `clicked`, for an alert's default condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertSide {
    Above,
    Below,
}

/// `None` when the market is not known or sits on the price, so the editor keeps its default.
pub fn alert_side(clicked: f64, last: Option<f64>) -> Option<AlertSide> {
    let last = last.filter(|last| last.is_finite())?;
    if !clicked.is_finite() || clicked == last {
        return None;
    }
    Some(if clicked > last {
        AlertSide::Above
    } else {
        AlertSide::Below
    })
}

// MARK: Menus

/// The menu for `context`, by what it landed on.
pub fn menu(context: &ChartContext, facts: &Facts) -> Vec<Entry> {
    match &context.target {
        Target::Background => background(context, facts),
        Target::PriceAxis => price_axis(facts),
        Target::TimeAxis => time_axis(),
        Target::Pane { instance } => pane(*instance, facts),
        Target::Alert { id } => alert(*id, facts),
        Target::Order { id } => order(*id, facts),
        Target::Position { key } => position(key),
    }
}

/// Groups joined by one separator each, empty groups dropped.
fn grouped(groups: Vec<Vec<Item>>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !entries.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.extend(group.into_iter().map(Entry::item));
    }
    entries
}

fn constructed_prices() -> &'static str {
    tr!("This chart's prices are constructed, not market prices. Use a candle or line chart.")
}

/// A live order's menu row: "Live · Buy BTC limit @ 70,000.00…". `price` is `None` for a row
/// without one (a market order, or a priced order while the clicked price is unknown).
fn live_label(
    side: degenview_brokers::Side,
    kind: TicketKind,
    symbol: &str,
    price: Option<&str>,
) -> String {
    use degenview_brokers::Side;
    match (side, kind, price) {
        (Side::Buy, TicketKind::Market, _) => {
            tr!("Live · Buy {symbol} at market…", symbol = symbol)
        }
        (Side::Sell, TicketKind::Market, _) => {
            tr!("Live · Sell {symbol} at market…", symbol = symbol)
        }
        (Side::Buy, TicketKind::Limit, Some(price)) => tr!(
            "Live · Buy {symbol} limit @ {price}…",
            symbol = symbol,
            price = price
        ),
        (Side::Buy, TicketKind::Limit, None) => {
            tr!("Live · Buy {symbol} limit…", symbol = symbol)
        }
        (Side::Sell, TicketKind::Limit, Some(price)) => tr!(
            "Live · Sell {symbol} limit @ {price}…",
            symbol = symbol,
            price = price
        ),
        (Side::Sell, TicketKind::Limit, None) => {
            tr!("Live · Sell {symbol} limit…", symbol = symbol)
        }
        (Side::Buy, TicketKind::StopLimit, Some(price)) => tr!(
            "Live · Buy {symbol} stop-limit @ {price}…",
            symbol = symbol,
            price = price
        ),
        (Side::Buy, TicketKind::StopLimit, None) => {
            tr!("Live · Buy {symbol} stop-limit…", symbol = symbol)
        }
        (Side::Sell, TicketKind::StopLimit, Some(price)) => tr!(
            "Live · Sell {symbol} stop-limit @ {price}…",
            symbol = symbol,
            price = price
        ),
        (Side::Sell, TicketKind::StopLimit, None) => {
            tr!("Live · Sell {symbol} stop-limit…", symbol = symbol)
        }
    }
}

/// The live entries: a market order and a limit and stop-limit order each way. They open the
/// ticket; none places anything. Greyed, with the reason, whenever an order could not be placed
/// from here.
fn live_items(context: &ChartContext, facts: &Facts, live: &LiveFacts) -> Vec<Item> {
    use degenview_brokers::{Side, validate::snap_to_increment};
    let symbol = &context.symbol;
    let shown = |price: f64| facts.price_format.format(price);
    let clicked = context.price();
    let snapped = clicked
        .zip(live.price_increment)
        .and_then(|(price, increment)| snap_to_increment(price, increment));
    let mut items = Vec::new();
    for side in [Side::Buy, Side::Sell] {
        for (kind, supported, needs_price) in [
            (TicketKind::Market, live.market, false),
            (TicketKind::Limit, live.limit, true),
            (TicketKind::StopLimit, live.stop_limit, true),
        ] {
            let price = clicked.filter(|_| needs_price).map(shown);
            let label = live_label(side, kind, symbol, price.as_deref());
            let item = Item::new(
                Command::LiveOrder(LiveOrderRequest {
                    side,
                    kind,
                    price: if needs_price { snapped } else { None },
                }),
                label,
            );
            let item = if let Some(reason) = live.unavailable {
                item.disabled_unless(false, reason.text())
            } else if facts.replay {
                item.disabled_unless(false, in_replay())
            } else if !supported {
                item.disabled_unless(false, tr!("Not available in this market."))
            } else if needs_price && live.constructed {
                item.disabled_unless(false, constructed_prices())
            } else if needs_price && snapped.is_none() {
                item.disabled_unless(false, no_price())
            } else {
                item
            };
            items.push(item);
        }
    }
    items
}

fn background(context: &ChartContext, facts: &Facts) -> Vec<Entry> {
    let price = context.price();
    let format = facts.price_format;
    let shown = |price: f64| format.format(price);
    let symbol = &context.symbol;

    let copy = Item::new(
        Command::CopyPrice,
        price.map_or_else(
            || tr!("Copy price").to_owned(),
            |p| tr!("Copy price {price}", price = shown(p)),
        ),
    )
    .disabled_unless(price.is_some(), no_price());
    let paste = Item::new(Command::Paste, tr!("Paste"))
        .hint(Hint::Paste)
        .disabled_unless(facts.clipboard, nothing_to_paste());

    let mut trading = Vec::new();
    if facts.alerts_supported {
        let label = price.map_or_else(
            || tr!("Add alert on {symbol}…", symbol = symbol),
            |p| {
                tr!(
                    "Add alert on {symbol} at {price}…",
                    symbol = symbol,
                    price = shown(p)
                )
            },
        );
        trading.push(
            Item::new(Command::AddAlert, label)
                .disabled_unless(price.is_some(), no_price())
                .disabled_unless(!facts.replay, in_replay()),
        );
    }
    if let Some(live) = &facts.live {
        trading.extend(live_items(context, facts, live));
    } else if facts.paper != PaperAccess::Unsupported {
        let reason = facts.replay.then_some(in_replay());
        let quantity = if facts.quantity.is_empty() {
            String::new()
        } else {
            format!("{} ", facts.quantity)
        };
        let suggestions = price
            .zip(facts.last_price)
            .and_then(|(clicked, last)| suggest_orders(clicked, last, facts.tick));
        for index in 0..2 {
            let item = match (suggestions, price) {
                (Some(orders), Some(clicked)) => {
                    let order = orders[index];
                    Item::new(
                        Command::QuickOrder(order),
                        quick_order_label(order, &quantity, symbol, &shown(clicked)),
                    )
                }
                // At the market, or no market known: the entries stay so the menu does not
                // reshuffle, greyed out with the reason.
                _ => {
                    let order = QuickOrder {
                        side: if index == 0 {
                            PaperOrderSide::Sell
                        } else {
                            PaperOrderSide::Buy
                        },
                        kind: if index == 0 {
                            PaperOrderType::Limit
                        } else {
                            PaperOrderType::Stop
                        },
                    };
                    Item::new(Command::QuickOrder(order), idle_order_label(order, symbol))
                        .disabled_unless(
                            false,
                            if price.is_some() {
                                no_market()
                            } else {
                                no_price()
                            },
                        )
                }
            };
            trading.push(match reason {
                Some(reason) => item.disabled_unless(false, reason),
                None => item,
            });
        }
        let label = price.map_or_else(
            || tr!("Add order on {symbol}…", symbol = symbol),
            |p| {
                tr!(
                    "Add order on {symbol} at {price}…",
                    symbol = symbol,
                    price = shown(p)
                )
            },
        );
        let add = Item::new(Command::AddOrder, label).disabled_unless(price.is_some(), no_price());
        trading.push(match reason {
            Some(reason) => add.disabled_unless(false, reason),
            None => add,
        });
    }

    let cursor = vec![
        Item::new(
            Command::ToggleTimeLock,
            if facts.time_locked {
                tr!("Unlock vertical cursor line")
            } else {
                tr!("Lock vertical cursor line by time")
            },
        )
        .disabled_unless(facts.time_locked || context.time_ms.is_some(), no_price()),
    ];

    let utilities = vec![
        Item::new(Command::ToggleTable, tr!("Table view")).checked(facts.table_open),
        Item::new(Command::ToggleObjectTree, tr!("Object tree")).checked(facts.tree_open),
        Item::new(Command::ToggleSessionStats, tr!("Session statistics")).checked(facts.stats_open),
        template_submenu(facts),
    ];

    let cleanup = vec![
        Item::new(
            Command::RemoveDrawings,
            remove_drawings_label(facts.drawings),
        )
        .disabled_unless(facts.drawings > 0, nothing_to_remove())
        .destructive(),
        Item::new(
            Command::RemoveIndicators,
            remove_indicators_label(facts.indicators),
        )
        .disabled_unless(facts.indicators > 0, nothing_to_remove())
        .destructive(),
    ];

    let settings = vec![Item::new(Command::OpenSettings, tr!("Settings…"))];
    grouped(vec![
        vec![copy, paste],
        trading,
        cursor,
        utilities,
        cleanup,
        settings,
    ])
}

fn template_submenu(facts: &Facts) -> Item {
    let row = |row: &TemplateRow| {
        Entry::Item(
            Item::new(Command::ApplyTemplate(row.id), row.name.clone()).current(row.current),
        )
    };
    let mut children = vec![Entry::Heading(tr!("My templates").to_owned())];
    let mut saved = facts
        .templates
        .iter()
        .filter(|row| !row.built_in)
        .peekable();
    if saved.peek().is_none() {
        let mut none = Item::new(Command::SaveTemplate, tr!("None saved yet"))
            .disabled_unless(false, tr!("Save the current look to add one"));
        none.command = None;
        children.push(Entry::Item(none));
    }
    children.extend(saved.map(row));
    let mut presets = facts.templates.iter().filter(|row| row.built_in).peekable();
    if presets.peek().is_some() {
        children.push(Entry::Separator);
        children.push(Entry::Heading(tr!("Presets").to_owned()));
        children.extend(presets.map(row));
    }
    children.push(Entry::Separator);
    children.push(Entry::Item(Item::new(
        Command::SaveTemplate,
        tr!("Save current as template…"),
    )));
    children.push(Entry::Item(Item::new(
        Command::ManageTemplates,
        tr!("Manage templates…"),
    )));
    children.push(Entry::Separator);
    children.push(Entry::Item(Item::new(
        Command::ResetTemplate,
        tr!("Reset to default"),
    )));
    Item::submenu(tr!("Chart template"), children).glyph(Glyph::Template)
}

fn price_axis(facts: &Facts) -> Vec<Entry> {
    grouped(vec![
        vec![Item::new(
            Command::ResetPriceScale,
            tr!("Reset price scale"),
        )],
        PriceScaleMode::ALL
            .into_iter()
            .map(|mode| {
                Item::new(Command::SetScaleMode(mode), mode.label())
                    .checked(facts.scale_mode == mode)
            })
            .collect(),
        vec![
            Item::new(Command::ToggleBarCountdown, tr!("Countdown to bar close"))
                .checked(facts.bar_countdown)
                .disabled_unless(facts.bar_countdown_eligible, no_bar_countdown()),
        ],
    ])
}

fn time_axis() -> Vec<Entry> {
    grouped(vec![vec![Item::new(
        Command::ResetChartView,
        tr!("Reset chart view"),
    )]])
}

fn pane(instance: u64, facts: &Facts) -> Vec<Entry> {
    let Some(pane) = &facts.pane else {
        return Vec::new();
    };
    let order = |label: &str, order: VisualOrder| {
        Entry::Item(
            Item::new(Command::PaneOrder(instance, order), label).checked(pane.order == order),
        )
    };
    let reset = pane
        .has_points
        .then(|| Item::new(Command::PaneResetPoints(instance), tr!("Reset points…")));
    let mut script = vec![
        Item::new(
            Command::PaneSettings(instance),
            tr!("{name} settings…", name = pane.name),
        ),
        Item::new(
            Command::PaneToggle(instance),
            if pane.visible {
                tr!("Hide")
            } else {
                tr!("Show")
            },
        ),
        Item::submenu(
            tr!("Visual order"),
            vec![
                order(tr!("As the script declares"), VisualOrder::Script),
                order(tr!("Bring to front"), VisualOrder::Front),
                order(tr!("Send behind chart"), VisualOrder::Behind),
            ],
        )
        .glyph(Glyph::Layers),
        Item::new(Command::PineLogs(instance), tr!("Pine logs")),
        Item::new(Command::OpenPineStudio, tr!("Open Pine Studio")),
    ];
    if let Some(reset) = reset {
        script.insert(1, reset);
    }
    grouped(vec![
        script,
        vec![Item::new(Command::PaneRemove(instance), tr!("Remove from chart")).destructive()],
    ])
}

fn alert(id: AlertId, facts: &Facts) -> Vec<Entry> {
    let Some(state) = facts.alert else {
        return Vec::new();
    };
    let toggle = match state {
        AlertState::Paused => Some((tr!("Resume alert"), Glyph::Play)),
        AlertState::Active => Some((tr!("Pause alert"), Glyph::Pause)),
        AlertState::Triggered | AlertState::Unsupported => None,
    };
    let mut first = vec![Item::new(Command::EditAlert(id), tr!("Edit alert…"))];
    if let Some((label, glyph)) = toggle {
        first.push(Item::new(Command::ToggleAlert(id), label).glyph(glyph));
    }
    grouped(vec![
        first,
        vec![Item::new(Command::DeleteAlert(id), tr!("Delete alert")).destructive()],
    ])
}

fn order(id: u64, facts: &Facts) -> Vec<Entry> {
    let Some(order) = facts.order else {
        return Vec::new();
    };
    grouped(vec![
        vec![
            Item::new(Command::CancelOrder(id), tr!("Cancel order"))
                .disabled_unless(order.cancelable, order_not_working())
                .destructive(),
        ],
        vec![Item::new(
            Command::ShowPaperPanel,
            tr!("Show in Paper panel"),
        )],
    ])
}

fn position(key: &str) -> Vec<Entry> {
    grouped(vec![
        vec![
            Item::new(
                Command::ClosePosition(key.to_owned()),
                tr!("Close position"),
            )
            .destructive(),
        ],
        vec![Item::new(
            Command::ShowPaperPanel,
            tr!("Show in Paper panel"),
        )],
    ])
}

// MARK: Validity

/// What is true of the workspace right now, for [`check`].
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// The clicked chart's symbol and source, `None` when it has been closed.
    pub chart: Option<(String, DataSource)>,
    pub instances: Vec<u64>,
    pub alerts: Vec<AlertId>,
    pub orders: Vec<u64>,
    pub positions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stale {
    ChartClosed,
    ChartChanged,
    TargetGone,
}

impl Stale {
    pub fn message(self) -> &'static str {
        match self {
            Self::ChartClosed => tr!("That chart was closed, so nothing was changed."),
            Self::ChartChanged => {
                tr!("The chart changed while the menu was open, so nothing was done.")
            }
            Self::TargetGone => tr!("That item is gone, so nothing was changed."),
        }
    }
}

/// Whether the snapshot still describes the chart: it exists, still shows the same market, and
/// what the menu was opened on is still there.
pub fn check(context: &ChartContext, live: &Live) -> Result<(), Stale> {
    let Some((symbol, source)) = &live.chart else {
        return Err(Stale::ChartClosed);
    };
    if *symbol != context.symbol || *source != context.source {
        return Err(Stale::ChartChanged);
    }
    let present = match &context.target {
        Target::Background | Target::PriceAxis | Target::TimeAxis => true,
        Target::Pane { instance } => live.instances.contains(instance),
        Target::Alert { id } => live.alerts.contains(id),
        Target::Order { id } => live.orders.contains(id),
        Target::Position { key } => live.positions.contains(key),
    };
    if present {
        Ok(())
    } else {
        Err(Stale::TargetGone)
    }
}

#[cfg(test)]
mod tests;
