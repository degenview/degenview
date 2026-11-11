//! Comparison overlays: markets drawn on top of a chart, on its time axis. This is the
//! configuration only (which markets, how they look, which scale); candles live in the
//! workspace's `OverlayHub`, never here, and nothing here is part of a template.
//!
//! Pure: no egui. An overlay is not a chart, an indicator or a script: it is owned by one chart,
//! has a stable id, and never changes what a script, an alert, a drawing or a paper order reads.

pub mod align;
pub mod feed;
pub mod scale;

use crate::model::InstrumentId;
use crate::model::chart::kind::{LineDash, PriceSource};
use crate::model::editor_theme::Rgba;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Identity of one overlay inside its chart. Assigned once and never changed; it is not an index.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OverlayId(pub u64);

/// Which price scale an overlay is drawn against.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayScale {
    /// The chart's own axis. The overlay is rebased onto the main series (it starts where the
    /// main series starts), so the chart's Percent and Indexed modes compare the two. This is
    /// TradingView's "Same % scale".
    #[default]
    Main,
    /// A scale of its own: the group's visible values are fitted to the plot's height. There is
    /// no second axis; the overlay's value is in its tag and the legend. Overlays that carry the
    /// same group id share one fit. TradingView's "New price scale" without the axis column.
    Own(OverlayId),
}

/// The palette new overlays take their colour from, in order.
pub const PALETTE: [u32; 8] = [
    0x2962FF, 0xFF9800, 0xBF5AF2, 0x00BCD4, 0xFF375F, 0xFFD60A, 0x30D158, 0x8E8E93,
];

pub const DEFAULT_WIDTH: f32 = 1.5;
pub const MIN_WIDTH: f32 = 0.5;
pub const MAX_WIDTH: f32 = 6.0;

/// One comparison market on a chart.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChartOverlay {
    pub id: OverlayId,
    pub instrument: InstrumentId,
    /// A name shown instead of the provider id (a DEX pair's `PEPE/WETH`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(with = "hex_color")]
    pub color: Rgba,
    #[serde(default = "default_width", deserialize_with = "clean_width")]
    pub width: f32,
    #[serde(default)]
    pub dash: LineDash,
    /// Which value of each bar the line follows.
    #[serde(default)]
    pub source: PriceSource,
    #[serde(default)]
    pub scale: OverlayScale,
    /// The last-value tag at the line's end.
    #[serde(default = "yes")]
    pub show_label: bool,
}

fn yes() -> bool {
    true
}

fn default_width() -> f32 {
    DEFAULT_WIDTH
}

/// A stored width is clamped and never NaN, which is what lets `ChartOverlay` be `Eq`.
fn clean_width<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let width = f32::deserialize(deserializer)?;
    Ok(if width.is_finite() {
        width.clamp(MIN_WIDTH, MAX_WIDTH)
    } else {
        DEFAULT_WIDTH
    })
}

mod hex_color {
    use super::*;

    pub fn serialize<S: Serializer>(color: &Rgba, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&color.to_hex())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rgba, D::Error> {
        let text = String::deserialize(deserializer)?;
        // A colour this version cannot read must not lose the overlay: fall back to the first
        // palette colour.
        Ok(Rgba::parse(&text).unwrap_or(Rgba::hex(PALETTE[0])))
    }
}

// `width` is the only float and is always finite (see `clean_width` and `set_width`).
impl Eq for ChartOverlay {}

impl ChartOverlay {
    /// The name the legend shows.
    pub fn name(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.instrument
                .source
                .display_symbol(&self.instrument.symbol)
        })
    }

    /// The group id of an overlay on its own scale.
    pub fn group(&self) -> Option<OverlayId> {
        match self.scale {
            OverlayScale::Main => None,
            OverlayScale::Own(group) => Some(group),
        }
    }
}

/// How an add was answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddOutcome {
    Added(OverlayId),
    /// The market is already an overlay of this chart; nothing was added.
    Duplicate(OverlayId),
    /// The market is the chart's own; nothing was added.
    MainMarket,
}

/// Where an overlay moves in the visual order. Later overlays are drawn later (on top).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Move {
    Forward,
    Backward,
    ToFront,
    ToBack,
}

/// What an overlay's scale is being set to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleChoice {
    Main,
    /// A scale of its own.
    Own,
    /// The scale another overlay is on.
    SharedWith(OverlayId),
}

/// One edit of a chart's overlays, by the id of the overlay it names. Every surface that changes
/// overlays (the legend, its menu, the settings page) sends one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayCommand {
    Remove(OverlayId),
    RemoveAll,
    SetVisible(OverlayId, bool),
    SetColor(OverlayId, Rgba),
    SetWidth(OverlayId, f32),
    SetDash(OverlayId, LineDash),
    SetSource(OverlayId, PriceSource),
    SetScale(OverlayId, ScaleChoice),
    SetLabel(OverlayId, bool),
    Move(OverlayId, Move),
    /// Show or hide every overlay at once: one edit, one undo step.
    SetAllVisible(bool),
}

/// The overlays of one chart, in visual order. No upper bound: it is a plain vector.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OverlayList(Vec<ChartOverlay>);

impl OverlayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ChartOverlay> {
        self.0.iter()
    }

    pub fn as_slice(&self) -> &[ChartOverlay] {
        &self.0
    }

    pub fn get(&self, id: OverlayId) -> Option<&ChartOverlay> {
        self.0.iter().find(|overlay| overlay.id == id)
    }

    pub fn get_mut(&mut self, id: OverlayId) -> Option<&mut ChartOverlay> {
        self.0.iter_mut().find(|overlay| overlay.id == id)
    }

    pub fn position(&self, id: OverlayId) -> Option<usize> {
        self.0.iter().position(|overlay| overlay.id == id)
    }

    /// An id no overlay of this chart has.
    fn next_id(&self) -> OverlayId {
        OverlayId(
            self.0
                .iter()
                .map(|overlay| overlay.id.0)
                .max()
                .map_or(1, |top| top + 1),
        )
    }

    /// The first palette colour no overlay uses; every colour taken, the palette cycles.
    pub fn next_color(&self) -> Rgba {
        let taken = |hex: u32| self.0.iter().any(|overlay| overlay.color == Rgba::hex(hex));
        let hex = PALETTE
            .iter()
            .copied()
            .find(|hex| !taken(*hex))
            .unwrap_or(PALETTE[self.0.len() % PALETTE.len()]);
        Rgba::hex(hex)
    }

    /// Adds `instrument` on top, unless it is the chart's own market (`main`) or already here.
    /// The market is compared by `InstrumentId::key()`, so `BTC` and `btcusdt` are one.
    pub fn add(
        &mut self,
        main: &InstrumentId,
        instrument: InstrumentId,
        title: Option<String>,
    ) -> AddOutcome {
        if main.key() == instrument.key() {
            return AddOutcome::MainMarket;
        }
        if let Some(existing) = self
            .0
            .iter()
            .find(|o| o.instrument.key() == instrument.key())
        {
            return AddOutcome::Duplicate(existing.id);
        }
        let id = self.next_id();
        let color = self.next_color();
        self.0.push(ChartOverlay {
            id,
            instrument,
            title,
            visible: true,
            color,
            width: DEFAULT_WIDTH,
            dash: LineDash::Solid,
            source: PriceSource::Close,
            scale: OverlayScale::Main,
            show_label: true,
        });
        AddOutcome::Added(id)
    }

    pub fn remove(&mut self, id: OverlayId) -> Option<ChartOverlay> {
        let at = self.position(id)?;
        Some(self.0.remove(at))
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn set_visible(&mut self, id: OverlayId, visible: bool) -> bool {
        self.get_mut(id).is_some_and(|overlay| {
            let changed = overlay.visible != visible;
            overlay.visible = visible;
            changed
        })
    }

    pub fn set_width(&mut self, id: OverlayId, width: f32) -> bool {
        let width = if width.is_finite() {
            width.clamp(MIN_WIDTH, MAX_WIDTH)
        } else {
            DEFAULT_WIDTH
        };
        self.get_mut(id).is_some_and(|overlay| {
            let changed = overlay.width != width;
            overlay.width = width;
            changed
        })
    }

    /// Sets the scale. `SharedWith` an overlay that is on the main scale puts this one there too.
    pub fn set_scale(&mut self, id: OverlayId, choice: ScaleChoice) -> bool {
        let scale = match choice {
            ScaleChoice::Main => OverlayScale::Main,
            ScaleChoice::Own => OverlayScale::Own(id),
            ScaleChoice::SharedWith(other) => match self.get(other) {
                Some(other) => other.scale,
                None => return false,
            },
        };
        self.get_mut(id).is_some_and(|overlay| {
            let changed = overlay.scale != scale;
            overlay.scale = scale;
            changed
        })
    }

    /// Moves an overlay in the visual order; `false` when it is already there or unknown.
    pub fn reorder(&mut self, id: OverlayId, to: Move) -> bool {
        let Some(from) = self.position(id) else {
            return false;
        };
        let last = self.0.len() - 1;
        let target = match to {
            Move::Forward => (from + 1).min(last),
            Move::Backward => from.saturating_sub(1),
            Move::ToFront => last,
            Move::ToBack => 0,
        };
        if target == from {
            return false;
        }
        let overlay = self.0.remove(from);
        self.0.insert(target, overlay);
        true
    }

    /// Applies `command`; whether it changed anything (a name that is not here, or a value the
    /// overlay already has, changes nothing).
    pub fn apply(&mut self, command: OverlayCommand) -> bool {
        match command {
            OverlayCommand::Remove(id) => self.remove(id).is_some(),
            OverlayCommand::RemoveAll => {
                let any = !self.0.is_empty();
                self.clear();
                any
            }
            OverlayCommand::SetVisible(id, visible) => self.set_visible(id, visible),
            OverlayCommand::SetWidth(id, width) => self.set_width(id, width),
            OverlayCommand::SetScale(id, choice) => self.set_scale(id, choice),
            OverlayCommand::Move(id, to) => self.reorder(id, to),
            OverlayCommand::SetColor(id, color) => self.edit(id, |o| &mut o.color, color),
            OverlayCommand::SetDash(id, dash) => self.edit(id, |o| &mut o.dash, dash),
            OverlayCommand::SetSource(id, source) => self.edit(id, |o| &mut o.source, source),
            OverlayCommand::SetLabel(id, shown) => self.edit(id, |o| &mut o.show_label, shown),
            OverlayCommand::SetAllVisible(visible) => {
                let mut changed = false;
                for overlay in &mut self.0 {
                    changed |= overlay.visible != visible;
                    overlay.visible = visible;
                }
                changed
            }
        }
    }

    fn edit<T: PartialEq>(
        &mut self,
        id: OverlayId,
        field: impl FnOnce(&mut ChartOverlay) -> &mut T,
        value: T,
    ) -> bool {
        self.get_mut(id).is_some_and(|overlay| {
            let slot = field(overlay);
            let changed = *slot != value;
            *slot = value;
            changed
        })
    }

    /// The markets that need candles: every overlay, hidden ones included, so showing one is
    /// instant; the hub only refreshes the visible ones.
    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.0.iter().map(|overlay| &overlay.instrument)
    }
}

impl<'a> IntoIterator for &'a OverlayList {
    type Item = &'a ChartOverlay;
    type IntoIter = std::slice::Iter<'a, ChartOverlay>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    fn list_of(symbols: &[&str]) -> OverlayList {
        let main = market("BTCUSDT");
        let mut list = OverlayList::new();
        for symbol in symbols {
            assert!(matches!(
                list.add(&main, market(symbol), None),
                AddOutcome::Added(_)
            ));
        }
        list
    }

    #[test]
    fn ids_are_stable_and_not_reused_while_a_higher_one_lives() {
        let mut list = list_of(&["ETHUSDT", "SOLUSDT", "BNBUSDT"]);
        let ids: Vec<_> = list.iter().map(|o| o.id).collect();
        assert_eq!(ids, [OverlayId(1), OverlayId(2), OverlayId(3)]);
        list.remove(OverlayId(2));
        let next = list.add(&market("BTCUSDT"), market("XRPUSDT"), None);
        assert_eq!(next, AddOutcome::Added(OverlayId(4)));
        assert_eq!(list.get(OverlayId(3)).unwrap().instrument.symbol, "BNBUSDT");
    }

    #[test]
    fn the_main_market_and_a_present_overlay_are_not_added_again() {
        let main = market("BTCUSDT");
        let mut list = OverlayList::new();
        assert_eq!(list.add(&main, market("btc"), None), AddOutcome::MainMarket);
        let AddOutcome::Added(id) = list.add(&main, market("ETHUSDT"), None) else {
            panic!()
        };
        assert_eq!(
            list.add(&main, market("eth/usdt"), None),
            AddOutcome::Duplicate(id)
        );
        assert_eq!(list.len(), 1);
        // The same ticker on another provider is another market.
        let other = InstrumentId::new(DataSource::Coinbase, "ETH-USD");
        assert!(matches!(list.add(&main, other, None), AddOutcome::Added(_)));
    }

    #[test]
    fn there_is_no_upper_bound() {
        let main = market("BTCUSDT");
        let mut list = OverlayList::new();
        for n in 0..1_000 {
            let added = list.add(
                &main,
                InstrumentId::new(DataSource::Alpaca, format!("S{n}")),
                None,
            );
            assert!(matches!(added, AddOutcome::Added(_)));
        }
        assert_eq!(list.len(), 1_000);
    }

    #[test]
    fn colours_are_distinct_until_the_palette_is_used_up_then_cycle() {
        let symbols: Vec<String> = (0..PALETTE.len() + 2)
            .map(|n| format!("S{n}USDT"))
            .collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let list = list_of(&refs);
        let colors: Vec<_> = list.iter().map(|o| o.color).collect();
        for (n, hex) in PALETTE.iter().enumerate() {
            assert_eq!(colors[n], Rgba::hex(*hex));
        }
        assert_eq!(colors[PALETTE.len()], Rgba::hex(PALETTE[0]));
        assert_eq!(colors[PALETTE.len() + 1], Rgba::hex(PALETTE[1]));
    }

    #[test]
    fn removing_frees_a_colour_for_the_next_overlay() {
        let mut list = list_of(&["A1USDT", "B1USDT", "C1USDT"]);
        let freed = list.get(OverlayId(2)).unwrap().color;
        list.remove(OverlayId(2));
        list.add(&market("BTCUSDT"), market("D1USDT"), None);
        assert_eq!(list.iter().last().unwrap().color, freed);
    }

    #[test]
    fn reordering_moves_one_overlay_and_reports_no_ops() {
        let mut list = list_of(&["A1USDT", "B1USDT", "C1USDT"]);
        let order = |list: &OverlayList| list.iter().map(|o| o.id.0).collect::<Vec<_>>();
        assert!(list.reorder(OverlayId(1), Move::Forward));
        assert_eq!(order(&list), [2, 1, 3]);
        assert!(list.reorder(OverlayId(3), Move::ToBack));
        assert_eq!(order(&list), [3, 2, 1]);
        assert!(!list.reorder(OverlayId(3), Move::Backward));
        assert!(!list.reorder(OverlayId(1), Move::ToFront));
        assert!(list.reorder(OverlayId(3), Move::ToFront));
        assert_eq!(order(&list), [2, 1, 3]);
        assert!(!list.reorder(OverlayId(99), Move::Forward));
    }

    #[test]
    fn visibility_and_width_report_changes_and_clamp() {
        let mut list = list_of(&["A1USDT"]);
        let id = OverlayId(1);
        assert!(list.set_visible(id, false));
        assert!(!list.set_visible(id, false));
        assert!(list.set_width(id, 100.0));
        assert_eq!(list.get(id).unwrap().width, MAX_WIDTH);
        assert!(list.set_width(id, f32::NAN));
        assert_eq!(list.get(id).unwrap().width, DEFAULT_WIDTH);
    }

    #[test]
    fn scales_group_by_the_id_they_carry() {
        let mut list = list_of(&["A1USDT", "B1USDT", "C1USDT"]);
        assert!(list.set_scale(OverlayId(1), ScaleChoice::Own));
        assert!(list.set_scale(OverlayId(2), ScaleChoice::SharedWith(OverlayId(1))));
        assert_eq!(list.get(OverlayId(2)).unwrap().group(), Some(OverlayId(1)));
        assert_eq!(list.get(OverlayId(3)).unwrap().group(), None);
        // Removing the id the group is named after leaves the others grouped.
        list.remove(OverlayId(1));
        assert_eq!(list.get(OverlayId(2)).unwrap().group(), Some(OverlayId(1)));
        // Sharing with one on the main scale puts this one on the main scale.
        assert!(list.set_scale(OverlayId(2), ScaleChoice::SharedWith(OverlayId(3))));
        assert_eq!(list.get(OverlayId(2)).unwrap().scale, OverlayScale::Main);
        assert!(!list.set_scale(OverlayId(2), ScaleChoice::SharedWith(OverlayId(77))));
    }

    #[test]
    fn a_list_round_trips_and_an_empty_one_is_an_empty_array() {
        let mut list = list_of(&["ETHUSDT", "SOLUSDT"]);
        list.set_scale(OverlayId(2), ScaleChoice::Own);
        list.set_visible(OverlayId(1), false);
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(serde_json::from_str::<OverlayList>(&json).unwrap(), list);
        assert_eq!(serde_json::to_string(&OverlayList::new()).unwrap(), "[]");
    }

    #[test]
    fn a_row_with_only_the_required_fields_decodes_with_defaults() {
        let json = r##"[{"id":5,"instrument":{"source":"binance","symbol":"ETHUSDT"},"color":"#FF9800"}]"##;
        let list: OverlayList = serde_json::from_str(json).unwrap();
        let overlay = &list.as_slice()[0];
        assert!(overlay.visible && overlay.show_label);
        assert_eq!(overlay.width, DEFAULT_WIDTH);
        assert_eq!(overlay.scale, OverlayScale::Main);
        assert_eq!(overlay.source, PriceSource::Close);
        assert_eq!(overlay.dash, LineDash::Solid);
    }

    #[test]
    fn unknown_fields_are_ignored_and_a_bad_colour_keeps_the_overlay() {
        let json = r##"[{"id":1,"instrument":{"source":"binance","symbol":"ETHUSDT"},"color":"teal","future":{"x":1}}]"##;
        let list: OverlayList = serde_json::from_str(json).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list.as_slice()[0].color, Rgba::hex(PALETTE[0]));
    }

    #[test]
    fn an_unknown_market_is_kept_not_dropped() {
        let main = market("BTCUSDT");
        let mut list = OverlayList::new();
        let ghost = InstrumentId::new(DataSource::Alpaca, "DELISTED");
        list.add(&main, ghost.clone(), None);
        let again: OverlayList =
            serde_json::from_str(&serde_json::to_string(&list).unwrap()).unwrap();
        assert_eq!(again.as_slice()[0].instrument, ghost);
    }

    #[test]
    fn commands_edit_one_overlay_and_report_whether_anything_changed() {
        let mut list = list_of(&["A1USDT", "B1USDT"]);
        let id = OverlayId(1);
        let red = Rgba::hex(0xFF0000);
        assert!(list.apply(OverlayCommand::SetColor(id, red)));
        assert!(!list.apply(OverlayCommand::SetColor(id, red)));
        assert!(list.apply(OverlayCommand::SetDash(id, LineDash::Dotted)));
        assert!(list.apply(OverlayCommand::SetSource(id, PriceSource::High)));
        assert!(list.apply(OverlayCommand::SetLabel(id, false)));
        assert!(list.apply(OverlayCommand::SetVisible(id, false)));
        let overlay = list.get(id).unwrap();
        assert_eq!(
            (
                overlay.color,
                overlay.dash,
                overlay.source,
                overlay.show_label,
                overlay.visible
            ),
            (red, LineDash::Dotted, PriceSource::High, false, false)
        );
        assert!(!list.apply(OverlayCommand::SetColor(OverlayId(9), red)));
        assert!(list.apply(OverlayCommand::Remove(id)));
        assert!(!list.apply(OverlayCommand::Remove(id)));
        assert!(list.apply(OverlayCommand::RemoveAll));
        assert!(!list.apply(OverlayCommand::RemoveAll));
        assert!(list.is_empty());
    }

    #[test]
    fn showing_or_hiding_all_is_one_edit_that_reports_whether_anything_moved() {
        let mut list = list_of(&["A1USDT", "B1USDT", "C1USDT"]);
        assert!(
            !list.apply(OverlayCommand::SetAllVisible(true)),
            "already all shown"
        );
        list.set_visible(OverlayId(2), false);
        assert!(list.apply(OverlayCommand::SetAllVisible(true)));
        assert!(list.iter().all(|o| o.visible));
        assert!(list.apply(OverlayCommand::SetAllVisible(false)));
        assert!(list.iter().all(|o| !o.visible));
        assert!(!list.apply(OverlayCommand::SetAllVisible(false)));
        assert!(!OverlayList::new().apply(OverlayCommand::SetAllVisible(false)));
    }
}
