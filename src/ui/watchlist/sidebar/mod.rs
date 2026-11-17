//! The watchlist panel: header (list selector, add, filter, options), column header,
//! rows with native-feeling drag reorder, and the prompts around them.
//!
//! A pure view. It reads the lists, flags and quotes it is handed and returns
//! `WatchlistAction`s; the workspace applies them to the store. Which list a window
//! shows, its filter and its highlighted row live in `WatchlistSidebarState`.

use crate::i18n::tr;
use crate::model::InstrumentId;
use crate::model::market::trend::Trend;
use crate::model::watchlist;
use crate::model::watchlist::layout::{Filter, Flags, Row};
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::watchlist::text::ImportReport;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistDisplaySettings, WatchlistError, WatchlistFlag,
    WatchlistInstrument, WatchlistNotice, WatchlistSort,
};
use crate::service::work_queue::Pending;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use crate::ui::watchlist::style;
use content::content;
use detail::{detail_height, detail_panel};
use eframe::egui::{self, Align, Id, RichText, Sense, Stroke, Vec2};
use header::{filter_field, header};
use modals::modals;
use std::collections::HashMap;

mod content;
mod detail;
mod header;
mod menus;
mod modals;

/// What the workspace should do on the sidebar's behalf.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchlistAction {
    // Charts.
    /// Open in the tab's focused chart.
    Open(WatchlistInstrument),
    AddChart(WatchlistInstrument),
    OpenInNewTab(WatchlistInstrument),
    /// The "+" button: the host shows its Add Symbol sheet for the current list.
    AddSymbol {
        list: Option<EntryId>,
    },
    CreateAlert(WatchlistInstrument),
    // Lists.
    CreateList {
        name: String,
        /// Seed the new list with this market ("New Watchlist…" from a membership menu).
        with: Option<WatchlistInstrument>,
    },
    RenameList {
        id: EntryId,
        name: String,
    },
    DuplicateList {
        id: EntryId,
        name: String,
    },
    DeleteList(EntryId),
    // Contents.
    AddToList {
        list: EntryId,
        item: WatchlistInstrument,
    },
    RemoveFromList {
        list: EntryId,
        instrument: InstrumentId,
    },
    RemoveEntry {
        list: EntryId,
        entry: EntryId,
    },
    MoveToSection {
        list: EntryId,
        entry: EntryId,
        section: Option<EntryId>,
    },
    Move {
        list: EntryId,
        mv: watchlist::layout::Move,
    },
    MoveToSectionByDrop {
        list: EntryId,
        mv: watchlist::layout::SectionMove,
    },
    AddSection {
        list: EntryId,
        title: String,
    },
    RenameSection {
        list: EntryId,
        section: EntryId,
        title: String,
    },
    DeleteSection {
        list: EntryId,
        section: EntryId,
    },
    SetSectionCollapsed {
        list: EntryId,
        section: EntryId,
        collapsed: bool,
    },
    // Display.
    SetSort {
        list: EntryId,
        sort: WatchlistSort,
    },
    SetColumn {
        list: EntryId,
        column: WatchlistColumn,
        visible: bool,
    },
    SetShowsDescription {
        list: EntryId,
        shows: bool,
    },
    SetShowsChart {
        list: EntryId,
        shows: bool,
    },
    SetFlag {
        instrument: InstrumentId,
        flag: Option<WatchlistFlag>,
    },
    // Exchange.
    Import {
        list: EntryId,
        text: String,
    },
    BeginImport,
    /// "Open File…" in the import sheet: the host owns the open dialog and hands the job back
    /// through [`WatchlistSidebarState::read_import_file`].
    ChooseImportFile,
    /// "Export to File…": the host owns the save dialog.
    Export {
        list: EntryId,
    },
    CopyText(String),
    DismissError,
}

pub struct WatchlistSidebarInput<'a> {
    pub lists: &'a [Watchlist],
    pub flags: &'a Flags,
    pub quote: &'a dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
    /// Each market's last month of closes, by `InstrumentId::key()`, for the row charts.
    pub trends: &'a HashMap<String, Trend>,
    /// The market the tab's focused chart shows; the matching row is highlighted.
    pub focused_market: Option<&'a InstrumentId>,
    /// The saved lists couldn't be read, so changes are off.
    pub load_failed: bool,
    pub error: Option<&'a WatchlistNotice>,
    /// What the last import did, shown in the import sheet.
    pub import_report: Option<&'a ImportReport>,
}

#[derive(Clone, Debug)]
enum PromptKind {
    NewList,
    RenameList(EntryId),
    CopyList(EntryId),
    NewSection(EntryId),
    RenameSection { list: EntryId, section: EntryId },
    NewListWith(Box<WatchlistInstrument>),
}

impl PromptKind {
    fn title(&self) -> &'static str {
        match self {
            Self::NewList | Self::NewListWith(_) => tr!("New Watchlist"),
            Self::RenameList(_) => tr!("Rename Watchlist"),
            Self::CopyList(_) => tr!("Make a Copy"),
            Self::NewSection(_) => tr!("New Section"),
            Self::RenameSection { .. } => tr!("Rename Section"),
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NewList => tr!("Name the new watchlist.").to_owned(),
            Self::NewListWith(item) => {
                tr!(
                    "Name the new watchlist. {name} will be added to it.",
                    name = item.name
                )
            }
            Self::RenameList(_) => tr!("Enter a new name.").to_owned(),
            Self::CopyList(_) => tr!("Name the copy.").to_owned(),
            Self::NewSection(_) => tr!("Sections group symbols under a heading.").to_owned(),
            Self::RenameSection { .. } => tr!("Enter a new name.").to_owned(),
        }
    }

    fn confirm(&self) -> &'static str {
        match self {
            Self::NewList | Self::NewListWith(_) => tr!("Create"),
            Self::RenameList(_) | Self::RenameSection { .. } => tr!("Rename"),
            Self::CopyList(_) => tr!("Copy"),
            Self::NewSection(_) => tr!("Create"),
        }
    }
}

struct Drag {
    /// Index into the rows as shown.
    index: usize,
}

pub struct WatchlistSidebarState {
    selected_list: Option<EntryId>,
    pub filter: Filter,
    pub filter_visible: bool,
    focus_filter: bool,
    pub selected_entry: Option<EntryId>,
    prompt: Option<PromptKind>,
    prompt_text: String,
    confirm_delete: bool,
    import_text: Option<String>,
    import_picker: Option<Pending<Result<Option<String>, String>>>,
    import_error: Option<String>,
    shows_detail: bool,
    drag: Option<Drag>,
    synced_focus: Option<String>,
    synced_list: Option<EntryId>,
    /// Markets of the rows drawn this frame, for the quote feed.
    demand: Vec<InstrumentId>,
    /// The filter box gave up focus this frame, so its Enter must not reach the rows.
    /// The list drawn this frame shows the month's chart, so `demand` also wants closes.
    demands_trends: bool,
    filter_blurred: bool,
}

impl Default for WatchlistSidebarState {
    fn default() -> Self {
        Self {
            selected_list: None,
            filter: Filter::default(),
            filter_visible: false,
            focus_filter: false,
            selected_entry: None,
            prompt: None,
            prompt_text: String::new(),
            confirm_delete: false,
            import_text: None,
            import_picker: None,
            import_error: None,
            // The detail card under the rows is on until the user hides it.
            shows_detail: true,
            drag: None,
            synced_focus: None,
            synced_list: None,
            demand: Vec::new(),
            demands_trends: false,
            filter_blurred: false,
        }
    }
}

impl WatchlistSidebarState {
    /// The import sheet's file being chosen and read, as the host started it for
    /// [`WatchlistAction::ChooseImportFile`]; its text fills the sheet when it arrives.
    pub fn read_import_file(&mut self, job: Pending<Result<Option<String>, String>>) {
        self.import_picker = Some(job);
    }

    pub fn minimum_width(&mut self, lists: &[Watchlist]) -> f32 {
        match self.resolved(lists) {
            Some(list) => style::minimum_panel_width(&list.display),
            None => style::minimum_panel_width(&WatchlistDisplaySettings::default()),
        }
    }

    #[cfg(test)]
    pub fn selected_list_id(&self) -> Option<EntryId> {
        self.selected_list
    }

    pub fn select_list(&mut self, id: EntryId) {
        self.selected_list = Some(id);
    }

    /// Opens the "name your watchlist" prompt seeded with a market, for a membership
    /// menu's "New Watchlist…" elsewhere in the app.
    pub fn prompt_new_list_with(&mut self, item: WatchlistInstrument) {
        self.prompt_text.clear();
        self.prompt = Some(PromptKind::NewListWith(Box::new(item)));
    }

    fn open_prompt(&mut self, kind: PromptKind, text: String) {
        self.prompt_text = text;
        self.prompt = Some(kind);
    }

    fn clear_filter(&mut self) {
        self.filter = Filter::default();
    }

    /// The list this window shows: its pick if it still exists, else Favorites, else the first.
    fn resolved<'a>(&mut self, lists: &'a [Watchlist]) -> Option<&'a Watchlist> {
        let found = self
            .selected_list
            .and_then(|id| lists.iter().find(|list| list.id == id))
            .or_else(|| lists.iter().find(|list| list.is_favorites))
            .or_else(|| lists.first());
        self.selected_list = found.map(|list| list.id);
        found
    }

    /// The markets whose quotes the rows drawn last frame need: nothing for a hidden row,
    /// a collapsed section, a filtered-out row or a portfolio card.
    pub fn quote_demand(&self) -> &[InstrumentId] {
        &self.demand
    }

    /// Markets whose month of closes the rows drawn this frame chart; none when the chart is off.
    pub fn trend_demand(&self) -> &[InstrumentId] {
        if self.demands_trends {
            &self.demand
        } else {
            &[]
        }
    }

    /// Keeps the highlight on a row that is drawn: a filter, a collapsed section or
    /// another list can hide it, and Enter/Delete and the detail card act on it.
    fn reconcile_selection(&mut self, rows: &[Row]) {
        if let Some(id) = self.selected_entry
            && !rows
                .iter()
                .any(|row| matches!(row, Row::Instrument { item, .. } if item.id == id))
        {
            self.selected_entry = None;
        }
    }

    /// The highlighted row follows the focused chart's market.
    fn sync_selection(&mut self, list: &Watchlist, focused: Option<&InstrumentId>) {
        let key = focused.map(InstrumentId::key);
        if self.synced_focus == key && self.synced_list == Some(list.id) {
            return;
        }
        self.synced_focus = key.clone();
        self.synced_list = Some(list.id);
        self.selected_entry = key.and_then(|key| {
            list.instruments()
                .find(|item| item.instrument.key() == key)
                .map(|item| item.id)
        });
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
) -> Vec<WatchlistAction> {
    let mut actions = Vec::new();
    if let Some(picker) = &mut state.import_picker {
        match picker.poll() {
            Some(Ok(Some(text))) => state.import_text = Some(text),
            Some(Ok(None)) => {}
            Some(Err(error)) => state.import_error = Some(error),
            None => {}
        }
        if !picker.is_pending() {
            state.import_picker = None;
        }
    }
    state.demand.clear();
    state.demands_trends = false;
    state.filter_blurred = false;
    let list = state.resolved(input.lists);

    header(ui, state, input, list, &mut actions);
    if state.filter_visible {
        filter_field(ui, state);
    }
    let separator = ui.painter().clone();
    let rect = ui.available_rect_before_wrap();
    separator.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, theme::border()),
    );
    ui.add_space(1.0);

    if input.load_failed {
        centered_message(
            ui,
            Icon::Triangle,
            tr!("Watchlists Unavailable"),
            &WatchlistError::PersistenceUnavailable.to_string(),
        );
    } else if let Some(list) = list {
        state.sync_selection(list, input.focused_market);
        let rows = watchlist::layout::rows(
            list,
            &state.filter,
            input.flags,
            list.display.sort,
            input.quote,
        );
        state.reconcile_selection(&rows);
        state.demands_trends = list.display.shows_chart;
        state.demand = rows
            .iter()
            .filter_map(|row| match row {
                Row::Instrument { item, .. } if item.card.is_none() => {
                    Some(item.instrument.clone())
                }
                _ => None,
            })
            .collect();
        if state.shows_detail {
            let detail_height = state
                .selected_entry
                .and_then(|id| list.instruments().find(|item| item.id == id))
                .map_or(0.0, |item| {
                    detail_height((input.quote)(&item.instrument).as_ref())
                });
            if detail_height > 0.0 {
                egui::Panel::bottom(Id::new(("watchlist-detail", list.id)))
                    .exact_size(detail_height)
                    .resizable(false)
                    .frame(
                        egui::Frame::new()
                            .fill(theme::chrome())
                            .inner_margin(egui::Margin::ZERO),
                    )
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| detail_panel(ui, state, input, list));
                    });
            }
        }
        content(ui, state, input, list, &rows, &mut actions);
    } else {
        centered_message(
            ui,
            Icon::List,
            tr!("No Watchlists"),
            tr!("Create a watchlist from the menu above."),
        );
    }

    modals(ui.ctx(), state, input, list, &mut actions);
    actions
}

fn centered_message(ui: &mut egui::Ui, icon: Icon, title: &str, detail: &str) {
    ui.add_space(((ui.available_height() - 170.0) / 2.0).max(24.0));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        icons::paint(ui.painter(), rect, icon, theme::text_tertiary());
        ui.add_space(12.0);
        ui.label(
            RichText::new(title)
                .strong()
                .size(16.0)
                .color(theme::text_primary()),
        );
        ui.add_space(5.0);
        ui.add_sized(
            [ui.available_width() - 36.0, 0.0],
            egui::Label::new(
                RichText::new(detail)
                    .size(12.0)
                    .color(ui.visuals().weak_text_color()),
            )
            .wrap()
            .halign(Align::Center),
        );
    });
}

#[cfg(test)]
mod tests;
