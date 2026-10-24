//! The chart's right-click menus as data: what was clicked, what each menu offers, and whether
//! it is still true when an entry is chosen.
//!
//! Nothing here touches egui, a store or an engine. A menu is a `Vec<Entry>` built from a
//! [`ChartContext`] (the snapshot taken at the click) and [`Facts`] (the chart's current state);
//! the UI draws it and hands back the chosen [`Command`], and the workspace, which owns every
//! mutation, [`check`]s the snapshot against what is live before it acts. The price and time of
//! the click are captured once, so moving the pointer to an entry never changes them.

use super::alert::{AlertId, AlertState};
use super::paper::{PaperOrderSide, PaperOrderType};
use super::pine_instance::VisualOrder;
use super::price_scale::PriceScaleMode;
use super::{DataSource, PriceFormat};

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
    /// The chart's price-scale mode, which the price-axis menu marks.
    pub scale_mode: PriceScaleMode,
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
            quantity: String::new(),
            drawings: 0,
            indicators: 0,
            clipboard: false,
            time_locked: false,
            table_open: false,
            tree_open: false,
            scale_mode: PriceScaleMode::Regular,
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
    ToggleTimeLock,
    ToggleTable,
    ToggleObjectTree,
    ApplyTemplate(u64),
    SaveTemplate,
    ManageTemplates,
    ResetTemplate,
    RemoveDrawings,
    RemoveIndicators,
    OpenSettings,
    ResetPriceScale,
    SetScaleMode(PriceScaleMode),
    ResetChartView,
    PaneSettings(u64),
    PaneToggle(u64),
    PaneRemove(u64),
    PaneOrder(u64, VisualOrder),
    OpenScriptManager,
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

pub const IN_REPLAY: &str = "Not available during replay";
const NO_PRICE: &str = "No price at this point";
const NO_MARKET: &str = "Price is at the market";
const NOTHING_TO_PASTE: &str = "No copied drawings to paste";
const NOTHING_TO_REMOVE: &str = "Nothing to remove";
const FILLED: &str = "The order is no longer working";

// MARK: Labels

/// "1 drawing", "17 drawings".
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub fn remove_drawings_label(count: usize) -> String {
    if count == 0 {
        "Remove drawings".to_owned()
    } else {
        format!("Remove {}", counted(count, "drawing"))
    }
}

pub fn remove_indicators_label(count: usize) -> String {
    if count == 0 {
        "Remove indicators".to_owned()
    } else {
        format!("Remove {}", counted(count, "indicator"))
    }
}

fn side_word(side: PaperOrderSide) -> &'static str {
    match side {
        PaperOrderSide::Buy => "Buy",
        PaperOrderSide::Sell => "Sell",
    }
}

fn kind_word(kind: PaperOrderType) -> &'static str {
    match kind {
        PaperOrderType::Limit => "limit",
        PaperOrderType::Stop => "stop",
        PaperOrderType::StopLimit => "stop limit",
        PaperOrderType::Market => "market",
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

fn background(context: &ChartContext, facts: &Facts) -> Vec<Entry> {
    let price = context.price();
    let format = facts.price_format;
    let shown = |price: f64| format.format(price);
    let symbol = &context.symbol;

    let copy = Item::new(
        Command::CopyPrice,
        price.map_or_else(
            || "Copy price".to_owned(),
            |p| format!("Copy price {}", shown(p)),
        ),
    )
    .disabled_unless(price.is_some(), NO_PRICE);
    let paste = Item::new(Command::Paste, "Paste")
        .hint(Hint::Paste)
        .disabled_unless(facts.clipboard, NOTHING_TO_PASTE);

    let mut trading = Vec::new();
    if facts.alerts_supported {
        let label = price.map_or_else(
            || format!("Add alert on {symbol}…"),
            |p| format!("Add alert on {symbol} at {}…", shown(p)),
        );
        trading.push(
            Item::new(Command::AddAlert, label)
                .disabled_unless(price.is_some(), NO_PRICE)
                .disabled_unless(!facts.replay, IN_REPLAY),
        );
    }
    if facts.paper != PaperAccess::Unsupported {
        let reason = facts.replay.then_some(IN_REPLAY);
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
                        format!(
                            "{} {quantity}{symbol} @ {} {}",
                            side_word(order.side),
                            shown(clicked),
                            kind_word(order.kind)
                        ),
                    )
                }
                // At the market, or no market known: the entries stay so the menu does not
                // reshuffle, greyed out with the reason.
                _ => {
                    let (side, kind) = if index == 0 {
                        ("Sell", "limit")
                    } else {
                        ("Buy", "stop")
                    };
                    Item::new(
                        Command::QuickOrder(QuickOrder {
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
                        }),
                        format!("{side} {symbol} {kind}"),
                    )
                    .disabled_unless(false, if price.is_some() { NO_MARKET } else { NO_PRICE })
                }
            };
            trading.push(match reason {
                Some(reason) => item.disabled_unless(false, reason),
                None => item,
            });
        }
        let label = price.map_or_else(
            || format!("Add order on {symbol}…"),
            |p| format!("Add order on {symbol} at {}…", shown(p)),
        );
        let add = Item::new(Command::AddOrder, label).disabled_unless(price.is_some(), NO_PRICE);
        trading.push(match reason {
            Some(reason) => add.disabled_unless(false, reason),
            None => add,
        });
    }

    let cursor = vec![
        Item::new(
            Command::ToggleTimeLock,
            if facts.time_locked {
                "Unlock vertical cursor line"
            } else {
                "Lock vertical cursor line by time"
            },
        )
        .disabled_unless(facts.time_locked || context.time_ms.is_some(), NO_PRICE),
    ];

    let utilities = vec![
        Item::new(Command::ToggleTable, "Table view").checked(facts.table_open),
        Item::new(Command::ToggleObjectTree, "Object tree").checked(facts.tree_open),
        template_submenu(facts),
    ];

    let cleanup = vec![
        Item::new(
            Command::RemoveDrawings,
            remove_drawings_label(facts.drawings),
        )
        .disabled_unless(facts.drawings > 0, NOTHING_TO_REMOVE)
        .destructive(),
        Item::new(
            Command::RemoveIndicators,
            remove_indicators_label(facts.indicators),
        )
        .disabled_unless(facts.indicators > 0, NOTHING_TO_REMOVE)
        .destructive(),
    ];

    let settings = vec![Item::new(Command::OpenSettings, "Settings…")];
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
    let mut children = vec![Entry::Heading("My templates".to_owned())];
    let mut saved = facts
        .templates
        .iter()
        .filter(|row| !row.built_in)
        .peekable();
    if saved.peek().is_none() {
        let mut none = Item::new(Command::SaveTemplate, "None saved yet")
            .disabled_unless(false, "Save the current look to add one");
        none.command = None;
        children.push(Entry::Item(none));
    }
    children.extend(saved.map(row));
    let mut presets = facts.templates.iter().filter(|row| row.built_in).peekable();
    if presets.peek().is_some() {
        children.push(Entry::Separator);
        children.push(Entry::Heading("Presets".to_owned()));
        children.extend(presets.map(row));
    }
    children.push(Entry::Separator);
    children.push(Entry::Item(Item::new(
        Command::SaveTemplate,
        "Save current as template…",
    )));
    children.push(Entry::Item(Item::new(
        Command::ManageTemplates,
        "Manage templates…",
    )));
    children.push(Entry::Separator);
    children.push(Entry::Item(Item::new(
        Command::ResetTemplate,
        "Reset to default",
    )));
    Item::submenu("Chart template", children).glyph(Glyph::Template)
}

fn price_axis(facts: &Facts) -> Vec<Entry> {
    grouped(vec![
        vec![Item::new(Command::ResetPriceScale, "Reset price scale")],
        PriceScaleMode::ALL
            .into_iter()
            .map(|mode| {
                Item::new(Command::SetScaleMode(mode), mode.label())
                    .checked(facts.scale_mode == mode)
            })
            .collect(),
    ])
}

fn time_axis() -> Vec<Entry> {
    grouped(vec![vec![Item::new(
        Command::ResetChartView,
        "Reset chart view",
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
    grouped(vec![
        vec![
            Item::new(
                Command::PaneSettings(instance),
                format!("{} settings…", pane.name),
            ),
            Item::new(
                Command::PaneToggle(instance),
                if pane.visible { "Hide" } else { "Show" },
            ),
            Item::submenu(
                "Visual order",
                vec![
                    order("As the script declares", VisualOrder::Script),
                    order("Bring to front", VisualOrder::Front),
                    order("Send behind chart", VisualOrder::Behind),
                ],
            )
            .glyph(Glyph::Layers),
            Item::new(Command::OpenScriptManager, "Open Script Manager"),
        ],
        vec![Item::new(Command::PaneRemove(instance), "Remove from chart").destructive()],
    ])
}

fn alert(id: AlertId, facts: &Facts) -> Vec<Entry> {
    let Some(state) = facts.alert else {
        return Vec::new();
    };
    let toggle = match state {
        AlertState::Paused => Some(("Resume alert", Glyph::Play)),
        AlertState::Active => Some(("Pause alert", Glyph::Pause)),
        AlertState::Triggered | AlertState::Unsupported => None,
    };
    let mut first = vec![Item::new(Command::EditAlert(id), "Edit alert…")];
    if let Some((label, glyph)) = toggle {
        first.push(Item::new(Command::ToggleAlert(id), label).glyph(glyph));
    }
    grouped(vec![
        first,
        vec![Item::new(Command::DeleteAlert(id), "Delete alert").destructive()],
    ])
}

fn order(id: u64, facts: &Facts) -> Vec<Entry> {
    let Some(order) = facts.order else {
        return Vec::new();
    };
    grouped(vec![
        vec![
            Item::new(Command::CancelOrder(id), "Cancel order")
                .disabled_unless(order.cancelable, FILLED)
                .destructive(),
        ],
        vec![Item::new(Command::ShowPaperPanel, "Show in Paper panel")],
    ])
}

fn position(key: &str) -> Vec<Entry> {
    grouped(vec![
        vec![Item::new(Command::ClosePosition(key.to_owned()), "Close position").destructive()],
        vec![Item::new(Command::ShowPaperPanel, "Show in Paper panel")],
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
            Self::ChartClosed => "That chart was closed, so nothing was changed.",
            Self::ChartChanged => "The chart changed while the menu was open, so nothing was done.",
            Self::TargetGone => "That item is gone, so nothing was changed.",
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
