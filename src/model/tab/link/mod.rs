//! Tab Linking: tabs tagged with the same colour keep chosen parts of their chart in step, in
//! any window. Pure: it decides who hears about a change and what they are told, so every rule
//! can be tested without a window. The workspace applies what it is told with the commands it
//! already has (`ChartWorkspace::apply_link`).
//!
//! Not to be confused with Link charts ([`crate::model::chart::sync`]), which carries a pan or
//! zoom between the charts of one layout. Tab Linking is between tabs and never looks inside a
//! layout beyond the chart the user is working in.
//!
//! # Rules
//!
//! * A tab is in one group (a [`LinkColor`]) or none. Membership lives on the tab; the hub reads
//!   it fresh every tick from the tabs that exist, so a closed, moved or recoloured tab cannot
//!   leave a stale subscriber behind and a group with no tabs keeps nothing.
//! * The four switches ([`LinkToggles`]) belong to the group: one set per colour.
//! * Only a user's action is a [`LinkSignal`]. What the hub delivers is applied by the target
//!   without queueing a signal of its own, so nothing echoes (there is no value comparison in
//!   the loop guard; see `ChartWorkspace::apply_link`).
//! * Every signal gets a growing sequence number; a delivery older than one the target has
//!   already applied is dropped, so a slow path cannot overwrite a newer state.
//! * A tab that joins a group adopts the group's latest known values for the enabled switches;
//!   the tabs already in it are never touched. A group's first tab seeds its values.
//!
//! What a switch means is inferred where TradingView does not say (it names Time and Date range
//! without defining them): Time is the position (the time at the right edge of the chart) and
//! Date range is the zoom (the span of time shown), which is what [`ChartSyncSettings`] already
//! calls them; both on gives the same time range.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::Timeframe;
use crate::model::chart::sync::{ChartSyncSettings, HorizontalChange};
use crate::model::watchlist::WatchlistInstrument;

/// Tabs are named by the same id the tab strip uses.
pub type TabId = u64;

/// The nine colour tags, in the order the menu shows them.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LinkColor {
    Red,
    Green,
    Blue,
    Yellow,
    Orange,
    Cyan,
    Purple,
    Pink,
    Violet,
}

impl LinkColor {
    pub const ALL: [Self; 9] = [
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Yellow,
        Self::Orange,
        Self::Cyan,
        Self::Purple,
        Self::Pink,
        Self::Violet,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Yellow => "Yellow",
            Self::Orange => "Orange",
            Self::Cyan => "Cyan",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
            Self::Violet => "Violet",
        }
    }

    /// The spelling stored on disk.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Cyan => "cyan",
            Self::Purple => "purple",
            Self::Pink => "pink",
            Self::Violet => "violet",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.key() == key)
    }

    /// For `#[serde(deserialize_with)]` on an `Option<LinkColor>` of a stored tab: a value this
    /// build does not know (a colour added later, a hand edit, the wrong type) reads as "not
    /// linked" instead of making the whole row undecodable, because a tab table with an
    /// undecodable row is never overwritten.
    pub fn lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Self>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(Self::from_key))
    }
}

impl Serialize for LinkColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for LinkColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::from_key(&key).ok_or_else(|| serde::de::Error::custom("unknown link colour"))
    }
}

/// Which parts of a chart a group keeps in step. One set per colour.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LinkToggles {
    /// The market (provider and symbol).
    pub symbol: bool,
    /// The timeframe.
    pub interval: bool,
    /// The point in time the chart is at (its right edge).
    pub time: bool,
    /// The span of time the chart shows.
    pub date_range: bool,
}

impl Default for LinkToggles {
    /// Symbol on and the rest off, as TradingView's menu shows a fresh tab.
    fn default() -> Self {
        Self {
            symbol: true,
            interval: false,
            time: false,
            date_range: false,
        }
    }
}

impl LinkToggles {
    #[cfg(test)]
    pub const NONE: Self = Self {
        symbol: false,
        interval: false,
        time: false,
        date_range: false,
    };

    pub const fn enables(self, kind: LinkKind) -> bool {
        match kind {
            LinkKind::Symbol => self.symbol,
            LinkKind::Interval => self.interval,
            LinkKind::View => self.time || self.date_range,
        }
    }

    /// What a pan or zoom carries to the other tab, in the terms the chart-to-chart follower
    /// already has: Time is the position, Date range is the zoom.
    pub const fn viewport(self) -> ChartSyncSettings {
        ChartSyncSettings {
            horizontal_position: self.time,
            horizontal_zoom: self.date_range,
        }
    }
}

/// The switches of every group. Colours nobody has set read as the defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LinkTable {
    by_color: BTreeMap<LinkColor, LinkToggles>,
}

impl LinkTable {
    /// The `setting` row the table is stored under. Local to the Mac (see `sync::policy`).
    pub const KEY: &'static str = "tab.link_toggles";

    pub fn toggles(&self, color: LinkColor) -> LinkToggles {
        self.by_color.get(&color).copied().unwrap_or_default()
    }

    /// Returns whether anything changed.
    pub fn set(&mut self, color: LinkColor, toggles: LinkToggles) -> bool {
        if self.toggles(color) == toggles {
            return false;
        }
        self.by_color.insert(color, toggles);
        true
    }
}

impl Serialize for LinkTable {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let map: BTreeMap<&str, &LinkToggles> = self
            .by_color
            .iter()
            .map(|(color, toggles)| (color.key(), toggles))
            .collect();
        map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LinkTable {
    /// Entry by entry: a colour this build does not know, or switches of the wrong shape, are
    /// dropped (the group then has the defaults) and never fail the whole table.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut table = Self::default();
        if let serde_json::Value::Object(entries) = value {
            for (key, toggles) in entries {
                if let (Some(color), Ok(toggles)) =
                    (LinkColor::from_key(&key), serde_json::from_value(toggles))
                {
                    table.by_color.insert(color, toggles);
                }
            }
        }
        Ok(table)
    }
}

/// The three things a tab can tell its group.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LinkKind {
    Symbol,
    Interval,
    View,
}

/// What a user did that the group may care about.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkSignal {
    /// The focused chart now shows this market. Carries the whole row (provider, symbol, title,
    /// artwork): a bare ticker is ambiguous, and a DEX pair or a prediction market has an opaque
    /// symbol that is useless without its title.
    Market(Box<WatchlistInstrument>),
    Timeframe(Timeframe),
    /// The focused chart's horizontal navigation, as a time range (never pixels or bar indices).
    View(HorizontalChange),
}

impl LinkSignal {
    pub const fn kind(&self) -> LinkKind {
        match self {
            Self::Market(_) => LinkKind::Symbol,
            Self::Timeframe(_) => LinkKind::Interval,
            Self::View(_) => LinkKind::View,
        }
    }
}

/// A tab's mailbox of what the user did since the app last looked. One slot per kind, newest
/// wins: a symbol picked twice between two looks is one delivery, a drag is its latest position.
/// That is what bounds the work to three entries per tab however fast the user acts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkOutbox {
    signals: Vec<LinkSignal>,
}

impl LinkOutbox {
    pub fn push(&mut self, signal: LinkSignal) {
        match self
            .signals
            .iter_mut()
            .find(|held| held.kind() == signal.kind())
        {
            Some(held) => *held = signal,
            None => self.signals.push(signal),
        }
    }

    pub fn take(&mut self) -> Vec<LinkSignal> {
        std::mem::take(&mut self.signals)
    }

    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }
}

/// What the hub needs to know about one tab, read from the tabs themselves each tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub tab: TabId,
    pub color: Option<LinkColor>,
    /// The market of the tab's focused chart, when it has one, as a whole row (so a group's
    /// first tab can seed it). Only filled for a tab that has a colour. Lets the hub skip telling
    /// a tab what it already shows.
    pub market: Option<Box<WatchlistInstrument>>,
    pub timeframe: Timeframe,
}

/// Something one tab is told to apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    pub target: TabId,
    /// Grows across the whole app. A target keeps the newest it has applied per kind and drops
    /// anything older.
    pub sequence: u64,
    pub signal: LinkSignal,
}

#[derive(Debug, Default)]
struct Group {
    members: BTreeSet<TabId>,
    market: Option<Box<WatchlistInstrument>>,
    timeframe: Option<Timeframe>,
    view: Option<HorizontalChange>,
}

/// The coordinator. It keeps, per colour, who is in it and the latest value of each kind that was
/// shared, and turns signals into deliveries. It holds no tab and does no I/O.
#[derive(Debug, Default)]
pub struct LinkHub {
    sequence: u64,
    groups: HashMap<LinkColor, Group>,
}

impl LinkHub {
    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// Brings the hub's idea of who is in which group up to date with the tabs that exist, and
    /// returns what each tab that just joined a group must adopt.
    ///
    /// Call it before [`ingest`](Self::ingest) every tick. A tab that is new to a group (a first
    /// tick, a recolour, a restore) is a joiner; at a restore every tab is new at once and the
    /// hub is empty, so nothing is delivered and no restored tab is overwritten.
    pub fn reconcile(&mut self, members: &[Member], table: &LinkTable) -> Vec<Delivery> {
        let mut now: HashMap<LinkColor, Vec<&Member>> = HashMap::new();
        for member in members {
            if let Some(color) = member.color {
                now.entry(color).or_default().push(member);
            }
        }
        // A group with no tabs keeps nothing.
        self.groups.retain(|color, _| now.contains_key(color));

        let mut deliveries = Vec::new();
        let mut colors: Vec<_> = now.keys().copied().collect();
        colors.sort();
        for color in colors {
            let present = &now[&color];
            let toggles = table.toggles(color);
            let group = self.groups.entry(color).or_default();
            let ids: BTreeSet<TabId> = present.iter().map(|member| member.tab).collect();
            // Members seen before this call are not joiners; closed ones simply vanish.
            let joiners: Vec<&Member> = present
                .iter()
                .copied()
                .filter(|member| !group.members.contains(&member.tab))
                .collect();
            // The tab that has been in the group longest (the lowest id, so it does not depend on
            // the order tabs are listed in) stands for what the group holds until someone changes
            // it. Only used when a tab joins a group that already had tabs.
            let veteran = present
                .iter()
                .copied()
                .filter(|member| group.members.contains(&member.tab))
                .min_by_key(|member| member.tab);
            group.members = ids;

            // The first tab of a group seeds what the group holds, so the next one to join has
            // something to adopt; so does a veteran when the group has no value yet (it was
            // restored with several tabs at once, and nobody has changed anything). A restore
            // itself delivers nothing: every tab is new and none is a veteran.
            let seed = match (present.as_slice(), veteran) {
                ([only], _) => Some(*only),
                (_, Some(veteran)) if !joiners.is_empty() => Some(veteran),
                _ => None,
            };
            if let Some(seed) = seed {
                if toggles.symbol && group.market.is_none() {
                    group.market = seed.market.clone();
                }
                if toggles.interval && group.timeframe.is_none() {
                    group.timeframe = Some(seed.timeframe);
                }
            }

            for joiner in joiners {
                if group.members.len() == 1 {
                    continue;
                }
                if toggles.symbol
                    && let Some(market) = &group.market
                    && !joiner
                        .market
                        .as_ref()
                        .is_some_and(|held| held.instrument.is_same_market(&market.instrument))
                {
                    let sequence = self.sequence + 1;
                    self.sequence = sequence;
                    deliveries.push(Delivery {
                        target: joiner.tab,
                        sequence,
                        signal: LinkSignal::Market(market.clone()),
                    });
                }
                if toggles.interval
                    && let Some(timeframe) = group.timeframe
                    && joiner.timeframe != timeframe
                {
                    let sequence = self.sequence + 1;
                    self.sequence = sequence;
                    deliveries.push(Delivery {
                        target: joiner.tab,
                        sequence,
                        signal: LinkSignal::Timeframe(timeframe),
                    });
                }
                if toggles.enables(LinkKind::View)
                    && let Some(view) = group.view
                {
                    let sequence = self.sequence + 1;
                    self.sequence = sequence;
                    deliveries.push(Delivery {
                        target: joiner.tab,
                        sequence,
                        signal: LinkSignal::View(view),
                    });
                }
            }
        }
        deliveries
    }

    /// A user's action in `origin`. Returns what the other members of its group must apply.
    ///
    /// Nothing is delivered, and nothing recorded, for a tab with no colour, for a kind the
    /// group has switched off, or to the origin itself. A symbol or interval the target already
    /// shows is not sent again.
    pub fn ingest(
        &mut self,
        origin: TabId,
        signal: LinkSignal,
        members: &[Member],
        table: &LinkTable,
    ) -> Vec<Delivery> {
        let Some(color) = members
            .iter()
            .find(|member| member.tab == origin)
            .and_then(|member| member.color)
        else {
            return Vec::new();
        };
        let toggles = table.toggles(color);
        if !toggles.enables(signal.kind()) {
            return Vec::new();
        }
        let sequence = self.next_sequence();
        let group = self.groups.entry(color).or_default();
        group.members.insert(origin);
        match &signal {
            LinkSignal::Market(market) => group.market = Some(market.clone()),
            LinkSignal::Timeframe(timeframe) => group.timeframe = Some(*timeframe),
            LinkSignal::View(change) => group.view = Some(*change),
        }
        members
            .iter()
            .filter(|member| member.tab != origin && member.color == Some(color))
            .filter(|member| match &signal {
                LinkSignal::Market(market) => !member
                    .market
                    .as_ref()
                    .is_some_and(|held| held.instrument.is_same_market(&market.instrument)),
                LinkSignal::Timeframe(timeframe) => member.timeframe != *timeframe,
                LinkSignal::View(_) => true,
            })
            .map(|member| Delivery {
                target: member.tab,
                sequence,
                signal: signal.clone(),
            })
            .collect()
    }

    /// A whole tick: [`reconcile`](Self::reconcile), then every drained signal in the order the
    /// tabs spoke. Deliveries to one tab are merged to the newest per kind, so a target is told
    /// once however many signals arrived; the last of BTC → ETH → SOL → AVAX is all it hears.
    /// A tab is never told something older than what it said itself in the same tick, so every
    /// tab of the group ends on the newest value.
    pub fn tick(
        &mut self,
        members: &[Member],
        table: &LinkTable,
        signals: Vec<(TabId, Vec<LinkSignal>)>,
    ) -> Vec<Delivery> {
        let mut all = self.reconcile(members, table);
        // What each tab said itself this tick, and when. `ingest` leaves a signal's own origin
        // out of its deliveries, so without this a tab that spoke last would still be handed
        // an older signal of another tab, and end up unlike the tabs that took its value.
        let mut said: HashMap<(TabId, LinkKind), u64> = HashMap::new();
        for (origin, signals) in signals {
            for signal in signals {
                let kind = signal.kind();
                let before = self.sequence;
                all.extend(self.ingest(origin, signal, members, table));
                // An ignored signal (unlinked tab, switch off) takes no sequence and has no say.
                if self.sequence != before {
                    said.insert((origin, kind), self.sequence);
                }
            }
        }
        all.retain(|delivery| {
            said.get(&(delivery.target, delivery.signal.kind()))
                .is_none_or(|own| delivery.sequence > *own)
        });
        merge(all)
    }

    /// The most recent value the group shares, for tests.
    #[cfg(test)]
    pub fn group_timeframe(&self, color: LinkColor) -> Option<Timeframe> {
        self.groups.get(&color).and_then(|group| group.timeframe)
    }

    #[cfg(test)]
    pub fn group_market(&self, color: LinkColor) -> Option<&WatchlistInstrument> {
        self.groups
            .get(&color)
            .and_then(|group| group.market.as_deref())
    }

    #[cfg(test)]
    pub fn has_group(&self, color: LinkColor) -> bool {
        self.groups.contains_key(&color)
    }
}

/// Keeps the highest sequence per target and kind, in the order the targets first appear.
fn merge(deliveries: Vec<Delivery>) -> Vec<Delivery> {
    let mut merged: Vec<Delivery> = Vec::with_capacity(deliveries.len());
    for delivery in deliveries {
        match merged.iter_mut().find(|held| {
            held.target == delivery.target && held.signal.kind() == delivery.signal.kind()
        }) {
            Some(held) if held.sequence < delivery.sequence => *held = delivery,
            Some(_) => {}
            None => merged.push(delivery),
        }
    }
    merged
}

#[cfg(test)]
mod tests;
