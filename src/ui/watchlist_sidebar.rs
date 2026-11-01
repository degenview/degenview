//! The watchlist panel: header (list selector, add, filter, options), column header,
//! rows with native-feeling drag reorder, and the prompts around them.
//!
//! A pure view. It reads the lists, flags and quotes it is handed and returns
//! `WatchlistAction`s; the workspace applies them to the store. Which list a window
//! shows, its filter and its highlighted row live in `WatchlistSidebarState`.

use crate::model::InstrumentId;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistError, WatchlistFlag, WatchlistInstrument,
    WatchlistSort, WatchlistSortKey,
};
use crate::model::watchlist_layout::{self, Filter, Flags, Row};
use crate::model::watchlist_quote::WatchlistQuote;
use crate::model::watchlist_quote::{self, Direction, Freshness};
use crate::model::watchlist_text::ImportReport;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::sheet;
use crate::ui::source_logo;
use crate::ui::theme;
use crate::ui::watchlist_rows::{self, RowStyle};
use crate::ui::watchlist_style::{self as style, LEADING_INSET, TRAILING_INSET};
use eframe::egui::{self, Align, Color32, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::sync::mpsc::{self, Receiver};

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
        mv: watchlist_layout::Move,
    },
    MoveToSectionByDrop {
        list: EntryId,
        mv: watchlist_layout::SectionMove,
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
    /// The market the tab's focused chart shows; the matching row is highlighted.
    pub focused_market: Option<&'a InstrumentId>,
    /// The saved lists couldn't be read, so changes are off.
    pub load_failed: bool,
    pub error: Option<&'a str>,
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
            Self::NewList | Self::NewListWith(_) => "New Watchlist",
            Self::RenameList(_) => "Rename Watchlist",
            Self::CopyList(_) => "Make a Copy",
            Self::NewSection(_) => "New Section",
            Self::RenameSection { .. } => "Rename Section",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NewList => "Name the new watchlist.".to_owned(),
            Self::NewListWith(item) => {
                format!("Name the new watchlist. {} will be added to it.", item.name)
            }
            Self::RenameList(_) => "Enter a new name.".to_owned(),
            Self::CopyList(_) => "Name the copy.".to_owned(),
            Self::NewSection(_) => "Sections group symbols under a heading.".to_owned(),
            Self::RenameSection { .. } => "Enter a new name.".to_owned(),
        }
    }

    fn confirm(&self) -> &'static str {
        match self {
            Self::NewList | Self::NewListWith(_) => "Create",
            Self::RenameList(_) | Self::RenameSection { .. } => "Rename",
            Self::CopyList(_) => "Copy",
            Self::NewSection(_) => "Create",
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
    import_picker: Option<Receiver<Result<Option<String>, String>>>,
    import_error: Option<String>,
    shows_detail: bool,
    drag: Option<Drag>,
    synced_focus: Option<String>,
    synced_list: Option<EntryId>,
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
        }
    }
}

impl WatchlistSidebarState {
    pub fn minimum_width(&mut self, lists: &[Watchlist]) -> f32 {
        let columns = self
            .resolved(lists)
            .map(|list| list.display.columns.as_slice());
        style::minimum_panel_width(
            columns.unwrap_or(&[WatchlistColumn::Last, WatchlistColumn::ChangePercent]),
        )
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

// MARK: Entry point

pub fn show(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
) -> Vec<WatchlistAction> {
    let mut actions = Vec::new();
    if let Some(receiver) = &state.import_picker {
        match receiver.try_recv() {
            Ok(Ok(Some(text))) => {
                state.import_text = Some(text);
                state.import_picker = None;
            }
            Ok(Ok(None)) => {
                state.import_picker = None;
            }
            Ok(Err(error)) => {
                state.import_error = Some(error);
                state.import_picker = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                state.import_picker = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
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
            "Watchlists Unavailable",
            &WatchlistError::PersistenceUnavailable.to_string(),
        );
    } else if let Some(list) = list {
        state.sync_selection(list, input.focused_market);
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
        content(ui, state, input, list, &mut actions);
    } else {
        centered_message(
            ui,
            Icon::List,
            "No Watchlists",
            "Create a watchlist from the menu above.",
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

// MARK: Header

fn header(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let controls_right = rect.right() - TRAILING_INSET;
    let options_rect = Rect::from_center_size(
        Pos2::new(controls_right - 12.0, rect.center().y),
        Vec2::splat(24.0),
    );
    let filter_rect = options_rect.translate(Vec2::new(-32.0, 0.0));
    let add_rect = filter_rect.translate(Vec2::new(-32.0, 0.0));
    let selector_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET, rect.center().y - 13.0),
        Pos2::new(
            (add_rect.left() - 8.0).max(rect.left() + LEADING_INSET + 64.0),
            rect.center().y + 13.0,
        ),
    );

    list_selector(ui, state, input, list, selector_rect);
    let enabled = !input.load_failed && list.is_some();
    let add = kit::hover_button(
        ui,
        add_rect,
        Id::new("watchlist-add"),
        Icon::Plus,
        "Add Symbol",
        true,
        enabled,
    );
    if add.clicked() {
        actions.push(WatchlistAction::AddSymbol {
            list: list.map(|list| list.id),
        });
    }
    let filtering = state.filter_visible || state.filter.is_active();
    let filter = ui.interact(
        filter_rect,
        Id::new("watchlist-filter-toggle"),
        Sense::click(),
    );
    let filter = crate::ui::accessibility::label(filter, "Filter this watchlist")
        .on_hover_text("Filter this watchlist");
    if filtering {
        ui.painter()
            .rect_filled(filter_rect, 6.0, theme::selected_tint(filter.hovered()));
    } else if filter.hovered() {
        ui.painter()
            .rect_filled(filter_rect, 6.0, Color32::from_white_alpha(17));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(filter_rect.center(), Vec2::splat(15.0)),
        Icon::Magnifier,
        theme::text_primary(),
    );
    if filter.clicked() {
        state.filter_visible = !state.filter_visible;
        if state.filter_visible {
            state.focus_filter = true;
        } else {
            state.clear_filter();
        }
    }
    options_menu(ui, state, list, actions, options_rect, enabled);
}

/// Narrowest the list menu opens; a long list name widens nothing, it ellipsizes.
const LIST_MENU_WIDTH: f32 = 232.0;
/// The header "…" menu and the row menus, and their submenus.
const MENU_WIDTH: f32 = 212.0;
const SUBMENU_WIDTH: f32 = 188.0;
/// What the menu frame's margins take from its width.
const MENU_FRAME: f32 = 10.0;

/// The header's list title with its chevron: a plate that hugs them, lit like the other header
/// buttons while hovered and while its menu is open.
fn list_selector(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    rect: Rect,
) {
    const PAD: f32 = 6.0;
    const CHEVRON: f32 = 10.0;
    let title = list.map_or("Watchlists", |list| list.name.as_str());
    let color = if input.load_failed {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };
    let galley = kit::galley(
        ui,
        title,
        14.0,
        kit::SEMIBOLD,
        color,
        (rect.width() - PAD * 3.0 - CHEVRON).max(10.0),
    );
    // The title keeps the panel's leading edge; the plate reaches out behind it.
    let plate = Rect::from_min_size(
        Pos2::new(rect.left() - PAD, rect.center().y - 14.0),
        Vec2::new(PAD * 3.0 + galley.size().x + CHEVRON, 28.0),
    );
    let response = ui.interact(
        plate,
        Id::new("watchlist-selector"),
        if input.load_failed {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let response = crate::ui::accessibility::label(response, format!("Watchlist: {title}"));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if !input.load_failed {
        if open || response.is_pointer_button_down_on() {
            ui.painter().rect_filled(plate, 6.0, theme::ink(26));
        } else if response.hovered() {
            ui.painter().rect_filled(plate, 6.0, theme::ink(14));
        }
    }
    ui.painter().galley(
        Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(plate.right() - PAD - CHEVRON / 2.0, rect.center().y),
            Vec2::splat(CHEVRON),
        ),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    kit::menu_popup(&response)
        .width(LIST_MENU_WIDTH)
        .show(|ui| {
            kit::menu_scope(ui, LIST_MENU_WIDTH - MENU_FRAME);
            if !input.lists.is_empty() {
                kit::menu_heading(ui, "Watchlists");
                egui::ScrollArea::vertical()
                    .max_height(kit::MENU_MAX_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for candidate in input.lists {
                            let count = candidate.instruments().count().to_string();
                            let row = kit::menu_row(
                                ui,
                                kit::MenuRow {
                                    icon: Some(if candidate.is_favorites {
                                        Icon::StarFill
                                    } else {
                                        Icon::List
                                    }),
                                    shortcut: Some(&count),
                                    current: list.is_some_and(|list| list.id == candidate.id),
                                    ..kit::MenuRow::new(&candidate.name)
                                },
                            );
                            if row.clicked() {
                                state.select_list(candidate.id);
                                state.selected_entry = None;
                                state.synced_list = None;
                                ui.close();
                            }
                        }
                    });
                kit::menu_separator(ui);
            }
            if kit::menu_item(ui, Icon::Plus, "Create New Watchlist…", true, false) {
                state.open_prompt(PromptKind::NewList, String::new());
                ui.close();
            }
            let Some(list) = list else {
                return;
            };
            if kit::menu_item(ui, Icon::Pencil, "Rename Watchlist…", true, false) {
                state.open_prompt(PromptKind::RenameList(list.id), list.name.clone());
                ui.close();
            }
            if kit::menu_item(ui, Icon::CopyDoc, "Make a Copy…", true, false) {
                state.open_prompt(PromptKind::CopyList(list.id), format!("{} copy", list.name));
                ui.close();
            }
            kit::menu_separator(ui);
            let deletable = !list.is_favorites;
            let delete = kit::menu_row(
                ui,
                kit::MenuRow {
                    icon: Some(Icon::Trash),
                    enabled: deletable,
                    destructive: true,
                    ..kit::MenuRow::new("Delete Watchlist…")
                },
            );
            if !deletable {
                delete.on_hover_text("Favorites can be renamed but not deleted");
            } else if delete.clicked() {
                state.confirm_delete = true;
                ui.close();
            }
        });
}

/// A row for one of the flags, with its swatch in the icon's place; `current` marks the chosen one.
fn flag_row(ui: &mut egui::Ui, flag: WatchlistFlag, current: bool) -> egui::Response {
    kit::menu_row(
        ui,
        kit::MenuRow {
            glyph: Some(&|painter: &egui::Painter, rect: Rect, _: Color32| {
                style::paint_swatch(painter, rect, flag);
            }),
            current,
            ..kit::MenuRow::new(flag.title())
        },
    )
}

fn options_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
    rect: Rect,
    enabled: bool,
) {
    let response = kit::hover_button(
        ui,
        rect,
        Id::new("watchlist-options"),
        Icon::Ellipsis,
        "Watchlist options",
        true,
        enabled,
    );
    kit::menu_popup(&response).width(MENU_WIDTH).show(|ui| {
        kit::menu_scope(ui, MENU_WIDTH - MENU_FRAME);
        let Some(list) = list else {
            return;
        };
        if kit::menu_item(ui, Icon::Plus, "Add Section…", true, false) {
            state.open_prompt(PromptKind::NewSection(list.id), String::new());
            ui.close();
        }
        kit::menu_separator(ui);

        let row = |icon, label| kit::MenuRow {
            icon: Some(icon),
            ..kit::MenuRow::new(label)
        };
        kit::menu_submenu(
            ui,
            row(Icon::ChevronUpDown, "Sort By"),
            SUBMENU_WIDTH,
            |ui| {
                let sort = list.display.sort;
                for key in WatchlistSortKey::ALL {
                    let chosen = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: sort.key == key,
                            ..kit::MenuRow::new(key.title())
                        },
                    );
                    if chosen.clicked() {
                        let sort = if key == WatchlistSortKey::Manual {
                            WatchlistSort::MANUAL
                        } else {
                            WatchlistSort {
                                key,
                                ascending: key == WatchlistSortKey::Symbol,
                            }
                        };
                        actions.push(WatchlistAction::SetSort {
                            list: list.id,
                            sort,
                        });
                        ui.close();
                    }
                }
                kit::menu_separator(ui);
                let manual = sort.key == WatchlistSortKey::Manual;
                let descending = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: !sort.ascending && !manual,
                        enabled: !manual,
                        ..kit::MenuRow::new("Descending")
                    },
                );
                if descending.clicked() {
                    actions.push(WatchlistAction::SetSort {
                        list: list.id,
                        sort: WatchlistSort {
                            key: sort.key,
                            ascending: !sort.ascending,
                        },
                    });
                }
            },
        );

        kit::menu_submenu(
            ui,
            row(Icon::SplitColumns, "Columns"),
            SUBMENU_WIDTH,
            |ui| {
                for column in WatchlistColumn::ALL {
                    let visible = list.display.columns.contains(&column);
                    let toggle = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: visible,
                            ..kit::MenuRow::new(column.accessibility_title())
                        },
                    );
                    if toggle.clicked() {
                        actions.push(WatchlistAction::SetColumn {
                            list: list.id,
                            column,
                            visible: !visible,
                        });
                    }
                }
                kit::menu_separator(ui);
                let shows = list.display.shows_description;
                let subtitle = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: shows,
                        ..kit::MenuRow::new("Show Symbol Subtitle")
                    },
                );
                if subtitle.clicked() {
                    actions.push(WatchlistAction::SetShowsDescription {
                        list: list.id,
                        shows: !shows,
                    });
                }
            },
        );

        kit::menu_submenu(ui, row(Icon::Tag, "Filter by Flag"), SUBMENU_WIDTH, |ui| {
            for flag in WatchlistFlag::ALL {
                if flag_row(ui, flag, state.filter.flag == Some(flag)).clicked() {
                    state.filter.flag = Some(flag);
                    ui.close();
                }
            }
            kit::menu_separator(ui);
            if kit::menu_item(
                ui,
                Icon::Xmark,
                "Show All",
                state.filter.flag.is_some(),
                false,
            ) {
                state.filter.flag = None;
                ui.close();
            }
        });

        let details = kit::menu_row(
            ui,
            kit::MenuRow {
                icon: Some(Icon::Eye),
                current: state.shows_detail,
                ..kit::MenuRow::new("Show Details")
            },
        );
        if details.clicked() {
            state.shows_detail = !state.shows_detail;
        }

        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Download, "Import Symbols…", true, false) {
            state.import_text = Some(String::new());
            state.import_error = None;
            actions.push(WatchlistAction::BeginImport);
            ui.close();
        }
        if kit::menu_item(ui, Icon::ShareUp, "Export to File…", true, false) {
            actions.push(WatchlistAction::Export { list: list.id });
            ui.close();
        }
        if kit::menu_item(ui, Icon::CopyDoc, "Copy as Text", true, false) {
            actions.push(WatchlistAction::CopyText(
                crate::model::watchlist_text::export(list),
            ));
            ui.close();
        }
    });
}

fn filter_field(ui: &mut egui::Ui, state: &mut WatchlistSidebarState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + LEADING_INSET + 7.0, rect.top() + 11.0),
            Vec2::splat(14.0),
        ),
        Icon::Magnifier,
        theme::text_secondary(),
    );
    let clear_width = if state.filter.is_active() { 24.0 } else { 0.0 };
    let flag_width = if state.filter.flag.is_some() {
        14.0
    } else {
        0.0
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET + 22.0, rect.top() + 1.0),
        Pos2::new(
            rect.right() - TRAILING_INSET - clear_width - flag_width,
            rect.top() + 25.0,
        ),
    );
    let response = ui.put(
        text_rect,
        egui::TextEdit::singleline(&mut state.filter.text)
            .hint_text("Filter symbols")
            .frame(egui::Frame::NONE),
    );
    if std::mem::take(&mut state.focus_filter) {
        response.request_focus();
    }
    if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.filter_visible = false;
        state.clear_filter();
    }
    if let Some(flag) = state.filter.flag {
        style::paint_flag(
            ui.painter(),
            rect.right() - TRAILING_INSET - clear_width - 8.0,
            rect.top() + 12.0,
            flag,
        );
    }
    if state.filter.is_active() {
        let clear_rect = Rect::from_center_size(
            Pos2::new(rect.right() - TRAILING_INSET - 10.0, rect.top() + 12.0),
            Vec2::splat(20.0),
        );
        if kit::hover_button(
            ui,
            clear_rect,
            Id::new("watchlist-filter-clear"),
            Icon::XmarkCircleFill,
            "Clear filter",
            true,
            true,
        )
        .clicked()
        {
            state.clear_filter();
            response.request_focus();
        }
    }
}

// MARK: Content

fn content(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    actions: &mut Vec<WatchlistAction>,
) {
    let rows = watchlist_layout::rows(
        list,
        &state.filter,
        input.flags,
        list.display.sort,
        input.quote,
    );
    if rows.is_empty() {
        empty_state(ui, state, list, actions);
        return;
    }

    if let Some(key) = watchlist_rows::column_header(ui, &list.display) {
        let sort = list.display.sort.after_clicking(key);
        actions.push(WatchlistAction::SetSort {
            list: list.id,
            sort,
        });
    }

    // Order on screen is the stored order only in Manual with no filter.
    let derived_order =
        list.display.sort.key != WatchlistSortKey::Manual || state.filter.is_active();
    let mut rects: Vec<Rect> = Vec::with_capacity(rows.len());
    let mut drop_requested = false;
    let mut drag_started = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let dragged = state.drag.as_ref().is_some_and(|drag| drag.index == index);
                let response = match row {
                    Row::Section { section, count } => {
                        let response =
                            watchlist_rows::section_row(ui, section, *count, index == 0, dragged);
                        if response.clicked() {
                            actions.push(WatchlistAction::SetSectionCollapsed {
                                list: list.id,
                                section: section.id,
                                collapsed: !section.is_collapsed,
                            });
                        }
                        kit::context_popup(&response)
                            .width(SUBMENU_WIDTH)
                            .show(|ui| {
                                section_menu(
                                    ui,
                                    state,
                                    list,
                                    section.id,
                                    &section.title,
                                    section.is_collapsed,
                                    actions,
                                );
                            });
                        response
                    }
                    Row::Instrument { item, .. } => {
                        let style = RowStyle {
                            display: &list.display,
                            flags: input.flags,
                            quote: input.quote,
                            selected: state.selected_entry == Some(item.id),
                            dragged,
                        };
                        let response = watchlist_rows::instrument_row(ui, item, &style);
                        let more_rect = Rect::from_center_size(
                            Pos2::new(
                                response.rect.right() - TRAILING_INSET - 11.0,
                                response.rect.center().y,
                            ),
                            Vec2::splat(22.0),
                        );
                        let more = ui.interact(
                            more_rect,
                            Id::new(("watchlist-row-more", item.id)),
                            Sense::click(),
                        );
                        let more = crate::ui::accessibility::label(
                            more,
                            format!("Actions for {}", item.name),
                        );
                        let popup = kit::menu_popup(&more).width(MENU_WIDTH);
                        let open = popup.is_open();
                        if response.hovered() || more.hovered() || open {
                            // Opaque, so the row's own trailing text does not show through.
                            ui.painter().rect_filled(more_rect, 6.0, theme::chrome());
                            if open || more.is_pointer_button_down_on() {
                                ui.painter().rect_filled(more_rect, 6.0, theme::ink(26));
                            } else if more.hovered() {
                                ui.painter().rect_filled(more_rect, 6.0, theme::ink(14));
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(more_rect.center(), Vec2::splat(13.0)),
                                Icon::Ellipsis,
                                theme::text_secondary(),
                            );
                        }
                        if response.clicked() && !more.clicked() {
                            state.selected_entry = Some(item.id);
                            actions.push(WatchlistAction::Open(item.clone()));
                        }
                        popup.show(|ui| instrument_menu(ui, state, input, list, item, actions));
                        kit::context_popup(&response)
                            .width(MENU_WIDTH)
                            .show(|ui| instrument_menu(ui, state, input, list, item, actions));
                        response
                    }
                    Row::EmptySection { .. } => watchlist_rows::empty_section_row(ui),
                };
                rects.push(response.rect);

                if row.entry_id().is_some() {
                    if response.drag_started() {
                        drag_started = Some(index);
                    }
                    if response.drag_stopped() {
                        drop_requested = true;
                    }
                }
            }
        });

    if let Some(index) = drag_started {
        state.drag = Some(Drag { index });
    }
    if let Some(drag) = &state.drag {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let destination = pointer.map(|pointer| insertion_index(&rects, pointer.y));
        if let Some(destination) = destination {
            paint_insertion_line(ui, &rects, destination);
        }
        if drop_requested {
            if let Some(destination) = destination {
                let source = drag.index;
                let dragged_is_symbol = matches!(rows.get(source), Some(Row::Instrument { .. }));
                if derived_order && dragged_is_symbol {
                    if let Some(mv) =
                        watchlist_layout::resolve_section_move(&rows, &[source], destination)
                    {
                        actions.push(WatchlistAction::MoveToSectionByDrop { list: list.id, mv });
                    }
                } else if let Some(mv) =
                    watchlist_layout::resolve_move(&rows, &[source], destination)
                {
                    actions.push(WatchlistAction::Move { list: list.id, mv });
                }
            }
            state.drag = None;
        }
    }
    if !ui.ctx().egui_wants_keyboard_input()
        && let Some(item) = state
            .selected_entry
            .and_then(|id| list.instruments().find(|item| item.id == id))
    {
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            actions.push(WatchlistAction::Open(item.clone()));
        }
        if ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) {
            actions.push(WatchlistAction::RemoveEntry {
                list: list.id,
                entry: item.id,
            });
            state.selected_entry = None;
        }
    }
}

/// The row boundary the pointer is nearest to: before the first row whose middle is
/// below it, or the end.
fn insertion_index(rects: &[Rect], pointer_y: f32) -> usize {
    rects
        .iter()
        .position(|rect| rect.center().y > pointer_y)
        .unwrap_or(rects.len())
}

fn paint_insertion_line(ui: &egui::Ui, rects: &[Rect], destination: usize) {
    let Some(first) = rects.first() else {
        return;
    };
    let y = match rects.get(destination) {
        Some(rect) => rect.top(),
        None => rects.last().map_or(first.top(), |rect| rect.bottom()),
    };
    let painter = ui.painter();
    let (left, right) = (first.left() + LEADING_INSET, first.right() - TRAILING_INSET);
    painter.line_segment(
        [Pos2::new(left, y), Pos2::new(right, y)],
        Stroke::new(2.0, theme::ACCENT),
    );
    painter.circle_filled(Pos2::new(left, y), 3.0, theme::ACCENT);
}

fn empty_state(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    actions: &mut Vec<WatchlistAction>,
) {
    if state.filter.is_active() {
        centered_message(
            ui,
            Icon::Magnifier,
            "No Matches",
            &format!("Nothing in {} matches this filter.", list.name),
        );
        ui.vertical_centered(|ui| {
            if ui.button("Clear Filter").clicked() {
                state.clear_filter();
            }
        });
    } else {
        centered_message(
            ui,
            Icon::Star,
            "No Symbols",
            "Add a stock, crypto, or prediction-market item to follow it here.",
        );
        ui.vertical_centered(|ui| {
            if ui.button("Add Symbol").clicked() {
                actions.push(WatchlistAction::AddSymbol {
                    list: Some(list.id),
                });
            }
        });
    }
}

// MARK: Detail card

const DETAIL_PAD: f32 = 12.0;
const DETAIL_HEADER: f32 = 34.0;
const DETAIL_HERO: f32 = 52.0;
const DETAIL_TILE: f32 = 40.0;
const DETAIL_TILE_GAP: f32 = 6.0;
const DETAIL_LISTS: f32 = 22.0;
const DETAIL_NOTE: f32 = 28.0;
/// Space between the card's bands.
const DETAIL_GAP: f32 = 10.0;

/// The card's height: its bands are fixed, so only a provider's note changes it.
fn detail_height(quote: Option<&WatchlistQuote>) -> f32 {
    let note = quote.is_some_and(|quote| quote.note.is_some());
    DETAIL_PAD * 2.0
        + DETAIL_HEADER
        + DETAIL_HERO
        + DETAIL_TILE * 2.0
        + DETAIL_TILE_GAP
        + DETAIL_LISTS
        + DETAIL_GAP * 3.0
        + if note { DETAIL_NOTE + DETAIL_GAP } else { 0.0 }
}

/// Paints `text` as a capsule centred on `center_y` with its right edge at `right`, and returns
/// the capsule's rect.
fn detail_pill(
    ui: &egui::Ui,
    right: f32,
    center_y: f32,
    text: &str,
    ink: Color32,
    fill: Color32,
    swatch: Option<WatchlistFlag>,
) -> Rect {
    let galley = kit::galley(ui, text, 11.0, kit::MEDIUM, ink, f32::INFINITY);
    let lead = if swatch.is_some() { 16.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + 16.0 + lead, 20.0);
    let rect = Rect::from_min_size(Pos2::new(right - size.x, center_y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    if let Some(flag) = swatch {
        style::paint_swatch(
            ui.painter(),
            Rect::from_center_size(Pos2::new(rect.left() + 14.0, center_y), Vec2::splat(12.0)),
            flag,
        );
    }
    ui.painter().galley(
        Pos2::new(rect.left() + 8.0 + lead, center_y - galley.size().y / 2.0),
        galley,
        ink,
    );
    rect
}

/// One labelled fact in a tile; `dot` puts a status light before the value. Hovering shows the
/// whole value when it was cut short.
fn detail_tile(
    ui: &mut egui::Ui,
    rect: Rect,
    id: Id,
    label: &str,
    value: &str,
    dot: Option<Color32>,
) {
    ui.painter().rect_filled(rect, 8.0, theme::ink(8));
    let caption = kit::galley(
        ui,
        &label.to_uppercase(),
        9.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
        rect.width() - 20.0,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 10.0, rect.top() + 7.0),
        caption,
        theme::text_tertiary(),
    );
    let lead = if dot.is_some() { 12.0 } else { 0.0 };
    let color = if value == watchlist_quote::MISSING {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };
    let text = kit::galley(
        ui,
        value,
        12.5,
        kit::REGULAR,
        color,
        rect.width() - 20.0 - lead,
    );
    let at = Pos2::new(rect.left() + 10.0 + lead, rect.top() + 21.0);
    if let Some(dot) = dot {
        ui.painter().circle_filled(
            Pos2::new(rect.left() + 13.5, at.y + text.size().y / 2.0),
            3.0,
            dot,
        );
    }
    ui.painter().galley(at, text, color);
    ui.interact(rect, id, Sense::hover())
        .on_hover_text(format!("{label}: {value}"));
}

fn freshness_color(freshness: Freshness) -> Color32 {
    match freshness {
        Freshness::Live => theme::UP,
        Freshness::Stale => theme::ORANGE,
        Freshness::MarketClosed => theme::text_secondary(),
        Freshness::Unavailable | Freshness::Unsupported => theme::text_tertiary(),
    }
}

/// Facts about the highlighted market, from what the quote provider reports; a field it does not
/// supply is a dash.
fn detail_panel(
    ui: &mut egui::Ui,
    state: &WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
) {
    let Some(item) = state
        .selected_entry
        .and_then(|id| list.instruments().find(|item| item.id == id))
    else {
        return;
    };
    let quote = (input.quote)(&item.instrument);
    let quote = quote.as_ref();
    let source = item.instrument.source;
    let (card, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), detail_height(quote)),
        Sense::hover(),
    );
    ui.painter().rect_filled(card, 0.0, theme::chrome());
    let (left, right) = (card.left() + DETAIL_PAD, card.right() - DETAIL_PAD);
    let width = right - left;
    let mut y = card.top() + DETAIL_PAD;

    // Who: the provider's logo on a plate, the market's name, and its flag.
    let header_mid = y + DETAIL_HEADER / 2.0;
    let plate = Rect::from_center_size(Pos2::new(left + 17.0, header_mid), Vec2::splat(34.0));
    ui.painter().rect_filled(plate, 9.0, theme::ink(10));
    source_logo::paint(
        ui,
        source,
        Rect::from_center_size(plate.center(), Vec2::splat(22.0)),
    );
    let mut text_right = right;
    if let Some(flag) = input.flags.get(&item.instrument.key()) {
        let pill = detail_pill(
            ui,
            right,
            header_mid,
            flag.title(),
            theme::text_secondary(),
            theme::ink(10),
            Some(*flag),
        );
        text_right = pill.left() - 8.0;
    }
    let text_left = plate.right() + 10.0;
    let name = kit::galley(
        ui,
        &item.name,
        14.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        text_right - text_left,
    );
    let subtitle = kit::galley(
        ui,
        source.display_name(),
        11.0,
        kit::REGULAR,
        theme::text_secondary(),
        text_right - text_left,
    );
    let block = name.size().y + subtitle.size().y;
    ui.painter().galley(
        Pos2::new(text_left, header_mid - block / 2.0),
        name.clone(),
        theme::text_primary(),
    );
    ui.painter().galley(
        Pos2::new(text_left, header_mid - block / 2.0 + name.size().y),
        subtitle,
        theme::text_secondary(),
    );
    y += DETAIL_HEADER + DETAIL_GAP;

    // What it is worth: the price large, how it moved beside it.
    let direction = watchlist_quote::direction(quote);
    let move_color = match direction {
        Direction::Up => theme::UP,
        Direction::Down => theme::DOWN,
        Direction::Flat => theme::text_secondary(),
    };
    let last = watchlist_quote::last_text(quote, Some(source));
    let percent = watchlist_quote::percent_text(quote, Some(source));
    let pill_text = if direction == Direction::Flat {
        percent
    } else {
        format!("{} {percent}", direction.glyph())
    };
    let hero_top = y;
    let pill = detail_pill(
        ui,
        right,
        hero_top + 15.0,
        &pill_text,
        move_color,
        theme::with_alpha(move_color, 0.14),
        None,
    );
    let price = kit::digits(
        ui,
        &last,
        24.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (pill.left() - 8.0 - left).max(40.0),
    );
    ui.painter().galley(
        Pos2::new(left, hero_top),
        price.clone(),
        theme::text_primary(),
    );
    let change = watchlist_quote::change_text(quote, Some(source));
    let change_galley = kit::galley(ui, &change, 12.0, kit::MEDIUM, move_color, width);
    let change_at = Pos2::new(left, hero_top + DETAIL_HERO - 18.0);
    let after = change_at.x + change_galley.size().x;
    ui.painter().galley(change_at, change_galley, move_color);
    if let Some(basis) = quote.and_then(|quote| quote.change_basis) {
        let basis = kit::galley(
            ui,
            &format!("· {}", basis.label()),
            11.0,
            kit::REGULAR,
            theme::text_tertiary(),
            (right - after - 6.0).max(10.0),
        );
        ui.painter().galley(
            Pos2::new(after + 6.0, change_at.y + 1.0),
            basis,
            theme::text_tertiary(),
        );
    }
    y += DETAIL_HERO + DETAIL_GAP;

    // The facts, two to a row.
    let freshness = quote.map_or(Freshness::Unavailable, |quote| quote.freshness);
    let updated = quote.and_then(|quote| quote.timestamp_ms).map_or_else(
        || watchlist_quote::MISSING.to_owned(),
        |timestamp| {
            crate::model::paper_format::timestamp(
                timestamp,
                crate::service::timezone::local_utc_offset_secs(timestamp),
            )
        },
    );
    let symbol = format!("{}:{}", source.watchlist_slug(), item.instrument.symbol);
    let volume = watchlist_quote::volume_text(quote);
    let tile_width = (width - DETAIL_TILE_GAP) / 2.0;
    let tiles = [
        ("Volume", volume.as_str(), None),
        (
            "Status",
            freshness.label(),
            Some(freshness_color(freshness)),
        ),
        ("Symbol", symbol.as_str(), None),
        ("Updated", updated.as_str(), None),
    ];
    for (index, (label, value, dot)) in tiles.into_iter().enumerate() {
        let (column, row) = (index % 2, index / 2);
        let rect = Rect::from_min_size(
            Pos2::new(
                left + column as f32 * (tile_width + DETAIL_TILE_GAP),
                y + row as f32 * (DETAIL_TILE + DETAIL_TILE_GAP),
            ),
            Vec2::new(tile_width, DETAIL_TILE),
        );
        detail_tile(
            ui,
            rect,
            Id::new(("watchlist-detail-tile", list.id, label)),
            label,
            value,
            dot,
        );
    }
    y += DETAIL_TILE * 2.0 + DETAIL_TILE_GAP + DETAIL_GAP;

    // Where else it lives.
    let names: Vec<&str> = input
        .lists
        .iter()
        .filter(|list| {
            list.instruments()
                .any(|candidate| candidate.instrument.is_same_market(&item.instrument))
        })
        .map(|list| list.name.as_str())
        .collect();
    let row_mid = y + DETAIL_LISTS / 2.0;
    let caption = kit::galley(
        ui,
        "IN LISTS",
        9.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
        width,
    );
    ui.painter().galley(
        Pos2::new(left, row_mid - caption.size().y / 2.0),
        caption.clone(),
        theme::text_tertiary(),
    );
    let mut x = left + caption.size().x + 10.0;
    if names.is_empty() {
        let none = kit::galley(
            ui,
            watchlist_quote::MISSING,
            12.0,
            kit::REGULAR,
            theme::text_tertiary(),
            20.0,
        );
        ui.painter().galley(
            Pos2::new(x, row_mid - none.size().y / 2.0),
            none,
            theme::text_tertiary(),
        );
    }
    for (index, name) in names.iter().enumerate() {
        let left_over = names.len() - index;
        let galley = kit::galley(
            ui,
            name,
            11.0,
            kit::MEDIUM,
            theme::text_secondary(),
            (right - x - 16.0).max(24.0),
        );
        let pill_width = galley.size().x + 16.0;
        // Keep room for a "+N" when the rest will not fit.
        let reserve = if left_over > 1 { 34.0 } else { 0.0 };
        if x + pill_width + reserve > right && index > 0 {
            detail_pill(
                ui,
                right.min(x + 34.0),
                row_mid,
                &format!("+{left_over}"),
                theme::text_tertiary(),
                theme::ink(8),
                None,
            );
            break;
        }
        let rect = detail_pill(
            ui,
            x + pill_width,
            row_mid,
            name,
            theme::text_secondary(),
            theme::ink(10),
            None,
        );
        x = rect.right() + 5.0;
    }
    y += DETAIL_LISTS + DETAIL_GAP;

    // What the provider said about the quote.
    if let Some(note) = quote.and_then(|quote| quote.note.as_ref()) {
        let rect = Rect::from_min_size(Pos2::new(left, y), Vec2::new(width, DETAIL_NOTE));
        ui.painter()
            .rect_filled(rect, 8.0, theme::with_alpha(theme::ORANGE, 0.12));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                Pos2::new(rect.left() + 14.0, rect.center().y),
                Vec2::splat(12.0),
            ),
            Icon::InfoCircle,
            theme::ORANGE,
        );
        let text = kit::galley(
            ui,
            note,
            11.0,
            kit::REGULAR,
            theme::text_secondary(),
            rect.width() - 34.0,
        );
        ui.painter().galley(
            Pos2::new(rect.left() + 28.0, rect.center().y - text.size().y / 2.0),
            text,
            theme::text_secondary(),
        );
        ui.interact(
            rect,
            Id::new(("watchlist-detail-note", list.id)),
            Sense::hover(),
        )
        .on_hover_text(note);
    }
}

// MARK: Menus

fn section_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    section: EntryId,
    title: &str,
    collapsed: bool,
    actions: &mut Vec<WatchlistAction>,
) {
    kit::menu_scope(ui, SUBMENU_WIDTH - MENU_FRAME);
    let (icon, label) = if collapsed {
        (Icon::ChevronDown, "Expand Section")
    } else {
        (Icon::ChevronUp, "Collapse Section")
    };
    if kit::menu_item(ui, icon, label, true, false) {
        actions.push(WatchlistAction::SetSectionCollapsed {
            list: list.id,
            section,
            collapsed: !collapsed,
        });
        ui.close();
    }
    if kit::menu_item(ui, Icon::Pencil, "Rename Section…", true, false) {
        state.open_prompt(
            PromptKind::RenameSection {
                list: list.id,
                section,
            },
            title.to_owned(),
        );
        ui.close();
    }
    if kit::menu_item(ui, Icon::Plus, "Add Section…", true, false) {
        state.open_prompt(PromptKind::NewSection(list.id), String::new());
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Delete Section", true, true) {
        actions.push(WatchlistAction::DeleteSection {
            list: list.id,
            section,
        });
        ui.close();
    }
}

/// The actions for one market, used by its context menu.
fn instrument_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    item: &WatchlistInstrument,
    actions: &mut Vec<WatchlistAction>,
) {
    kit::menu_scope(ui, MENU_WIDTH - MENU_FRAME);
    if kit::menu_item(ui, Icon::ChartLine, "Open in Focused Chart", true, false) {
        actions.push(WatchlistAction::Open(item.clone()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::Plus, "Add as New Chart", true, false) {
        actions.push(WatchlistAction::AddChart(item.clone()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::ArrowUpRightSquare, "Open in New Tab", true, false) {
        actions.push(WatchlistAction::OpenInNewTab(item.clone()));
        ui.close();
    }
    kit::menu_separator(ui);

    membership_menu(ui, input.lists, item, input.load_failed, actions, |item| {
        state.prompt_new_list_with(item);
    });

    if list.sections().next().is_some() {
        let owner = list.owning_section(item.id).map(|section| section.id);
        let row = kit::MenuRow {
            icon: Some(Icon::Layers),
            ..kit::MenuRow::new("Move to Section")
        };
        kit::menu_submenu(ui, row, SUBMENU_WIDTH, |ui| {
            let none = kit::menu_row(
                ui,
                kit::MenuRow {
                    checked: owner.is_none(),
                    ..kit::MenuRow::new("No Section")
                },
            );
            if none.clicked() {
                actions.push(WatchlistAction::MoveToSection {
                    list: list.id,
                    entry: item.id,
                    section: None,
                });
                ui.close();
            }
            for section in list.sections() {
                let target = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: owner == Some(section.id),
                        ..kit::MenuRow::new(&section.title)
                    },
                );
                if target.clicked() {
                    actions.push(WatchlistAction::MoveToSection {
                        list: list.id,
                        entry: item.id,
                        section: Some(section.id),
                    });
                    ui.close();
                }
            }
        });
    }

    let current = input.flags.get(&item.instrument.key()).copied();
    let row = kit::MenuRow {
        icon: Some(Icon::Tag),
        ..kit::MenuRow::new("Set Flag")
    };
    kit::menu_submenu(ui, row, SUBMENU_WIDTH, |ui| {
        for flag in WatchlistFlag::ALL {
            if flag_row(ui, flag, current == Some(flag)).clicked() {
                actions.push(WatchlistAction::SetFlag {
                    instrument: item.instrument.clone(),
                    flag: (current != Some(flag)).then_some(flag),
                });
                ui.close();
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Xmark, "Clear Flag", current.is_some(), false) {
            actions.push(WatchlistAction::SetFlag {
                instrument: item.instrument.clone(),
                flag: None,
            });
            ui.close();
        }
    });

    if !item.instrument.source.is_prediction_market()
        && item.instrument.source != crate::model::DataSource::CoinMarketCap
        && kit::menu_item(ui, Icon::Bell, "Create Price Alert…", true, false)
    {
        actions.push(WatchlistAction::CreateAlert(item.clone()));
        ui.close();
    }

    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::CopyDoc, "Copy Symbol", true, false) {
        actions.push(WatchlistAction::CopyText(item.instrument.symbol.clone()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::Clipboard, "Copy Qualified Symbol", true, false) {
        actions.push(WatchlistAction::CopyText(format!(
            "{}:{}",
            item.instrument.source.watchlist_slug(),
            item.instrument.symbol
        )));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Remove from Watchlist", true, true) {
        actions.push(WatchlistAction::RemoveEntry {
            list: list.id,
            entry: item.id,
        });
        ui.close();
    }
}

/// "Add to Watchlist": every list with a check where the market already is, then New
/// Watchlist…. Shared by the sidebar, chart cards and search results so membership
/// reads the same everywhere. `on_new_list` is called for "New Watchlist…"; the caller
/// decides where the name prompt lives. Ticking a list leaves the menu open, so a market can
/// join several in one visit.
pub fn membership_menu(
    ui: &mut egui::Ui,
    lists: &[Watchlist],
    item: &WatchlistInstrument,
    disabled: bool,
    actions: &mut Vec<WatchlistAction>,
    on_new_list: impl FnOnce(WatchlistInstrument),
) {
    let row = kit::MenuRow {
        icon: Some(Icon::Star),
        enabled: !disabled,
        ..kit::MenuRow::new("Add to Watchlist")
    };
    let response = kit::menu_submenu(ui, row, SUBMENU_WIDTH, |ui| {
        for list in lists {
            let member = list.contains(&item.instrument);
            let toggle = kit::menu_row(
                ui,
                kit::MenuRow {
                    checked: member,
                    ..kit::MenuRow::new(&list.name)
                },
            );
            if toggle.clicked() {
                actions.push(if member {
                    WatchlistAction::RemoveFromList {
                        list: list.id,
                        instrument: item.instrument.clone(),
                    }
                } else {
                    WatchlistAction::AddToList {
                        list: list.id,
                        item: item.clone(),
                    }
                });
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Plus, "New Watchlist…", true, false) {
            on_new_list(item.clone());
            ui.close();
        }
    });
    if disabled {
        response.on_hover_text("Watchlists are unavailable");
    }
}

// MARK: Modals

fn modals(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    prompt_window(ctx, state, actions);
    delete_window(ctx, state, list, actions);
    import_window(ctx, state, input, list, actions);
    if let Some(message) = input.error {
        sheet::show(ctx, Id::new("watchlist-error"), 340.0, |ui| {
            ui.add_space(20.0);
            sheet::inset(ui, |ui| {
                sheet::header(ui, Icon::Triangle, "Watchlist", message, theme::ORANGE);
                ui.add_space(20.0);
            });
            ui.separator();
            sheet::inset(ui, |ui| {
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        actions.push(WatchlistAction::DismissError);
                    }
                });
                ui.add_space(12.0);
            });
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            actions.push(WatchlistAction::DismissError);
        }
    }
}

const PROMPT_WIDTH: f32 = 400.0;

fn prompt_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    actions: &mut Vec<WatchlistAction>,
) {
    let Some(kind) = state.prompt.clone() else {
        return;
    };
    let mut commit = false;
    let mut cancel = false;
    let message = kind.message();
    let field_id = Id::new("watchlist-prompt-name");
    let valid = !state.prompt_text.trim().is_empty();
    let fresh = ctx.data_mut(|data| data.get_temp::<bool>(field_id).is_none());
    sheet::show(ctx, Id::new("watchlist-prompt"), PROMPT_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, Icon::List, kind.title(), &message, theme::ACCENT);
            ui.add_space(20.0);
            let focused = ui.memory(|memory| memory.has_focus(field_id));
            kit::field_frame(focused).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ui.available_width();
                let response = kit::plain_field(
                    ui,
                    field_id,
                    &mut state.prompt_text,
                    "Name",
                    kit::size::BODY,
                    kit::REGULAR,
                    width,
                );
                if fresh {
                    response.request_focus();
                    // Select the current name so typing replaces it.
                    if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), field_id) {
                        edit.cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(state.prompt_text.chars().count()),
                            )));
                        edit.store(ui.ctx(), field_id);
                    }
                }
            });
            ui.add_space(22.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if kit::sheet_button(ui, kind.confirm(), true, valid).clicked() {
                        commit = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        cancel = true;
                    }
                });
            });
            ui.add_space(14.0);
        });
    });
    ctx.data_mut(|data| data.insert_temp(field_id, true));
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    } else if valid && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        commit = true;
    }
    if commit || cancel {
        ctx.data_mut(|data| data.remove::<bool>(field_id));
    }
    if commit {
        let name = state.prompt_text.trim().to_owned();
        match kind {
            PromptKind::NewList => actions.push(WatchlistAction::CreateList { name, with: None }),
            PromptKind::NewListWith(item) => actions.push(WatchlistAction::CreateList {
                name,
                with: Some(*item),
            }),
            PromptKind::RenameList(id) => actions.push(WatchlistAction::RenameList { id, name }),
            PromptKind::CopyList(id) => actions.push(WatchlistAction::DuplicateList { id, name }),
            PromptKind::NewSection(list) => {
                actions.push(WatchlistAction::AddSection { list, title: name })
            }
            PromptKind::RenameSection { list, section } => {
                actions.push(WatchlistAction::RenameSection {
                    list,
                    section,
                    title: name,
                });
            }
        }
        state.prompt = None;
    } else if cancel {
        state.prompt = None;
    }
}

fn delete_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    if !state.confirm_delete {
        return;
    }
    let Some(list) = list else {
        state.confirm_delete = false;
        return;
    };
    let title = format!("Delete “{}”?", list.name);
    let result = kit::confirm(
        ctx,
        Id::new("watchlist-delete"),
        &Confirm {
            icon: Icon::Trash,
            title: &title,
            message: "Its symbols stay in your other watchlists. Charts, drawings and alerts are not affected.",
            destructive: "Delete",
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(WatchlistAction::DeleteList(list.id));
    }
    if result.is_some() {
        state.confirm_delete = false;
    }
}

fn import_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (Some(text), Some(list)) = (state.import_text.as_mut(), list) else {
        return;
    };
    let mut close = false;
    let mut submit = false;
    sheet::show(ctx, Id::new("watchlist-import"), 460.0, |ui| {
        ui.set_height(380.0);
        ui.add_space(20.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Download,
                &format!("Import into {}", list.name),
                "Paste symbols or open a text file.",
                theme::ACCENT,
            );
            ui.add_space(14.0);
            if let Some(report) = input.import_report {
                ui.label(RichText::new(report.summary()).strong().color(
                    if report.skipped.is_empty() {
                        theme::UP
                    } else {
                        theme::ORANGE
                    },
                ));
                if !report.skipped.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("SKIPPED")
                            .small()
                            .strong()
                            .color(ui.visuals().weak_text_color()),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(190.0)
                        .show(ui, |ui| {
                            for skipped in &report.skipped {
                                ui.label(RichText::new(&skipped.text).monospace().small());
                                ui.label(
                                    RichText::new(format!(
                                        "Line {} · {}",
                                        skipped.line, skipped.reason
                                    ))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                                );
                                ui.add_space(7.0);
                            }
                        });
                }
            } else {
                ui.label(RichText::new("One symbol per line as SOURCE:SYMBOL, for example BINANCE:BTCUSDT or COINBASE:ETH-USD. Lines starting with ### become sections. A single comma-separated line from TradingView is read too; markets DegenView has no provider for are skipped.").small().color(ui.visuals().weak_text_color()));
                ui.add_space(10.0);
                ui.add_sized(
                    [ui.available_width(), 155.0],
                    egui::TextEdit::multiline(text)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("BINANCE:BTCUSDT\nCOINBASE:ETH-USD"),
                );
                if let Some(error) = &state.import_error {
                    ui.label(RichText::new(error).small().color(theme::DOWN));
                }
            }
            ui.add_space(14.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if input.import_report.is_none()
                    && ui
                        .add_enabled(
                            state.import_picker.is_none(),
                            egui::Button::new("Open File…"),
                        )
                        .clicked()
                {
                    let (sender, receiver) = mpsc::channel();
                    state.import_picker = Some(receiver);
                    std::thread::spawn(move || {
                        let result =
                            crate::service::file_dialog::choose_csv("Choose watchlist symbols")
                                .and_then(|path| {
                                    path.map(|path| crate::service::file_dialog::read_text(&path))
                                        .transpose()
                                });
                        let _ = sender.send(result);
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if input.import_report.is_some() {
                        if ui.button("Done").clicked() {
                            close = true;
                        }
                    } else {
                        if ui
                            .add_enabled(!text.trim().is_empty(), egui::Button::new("Import"))
                            .clicked()
                        {
                            submit = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
            ui.add_space(12.0);
        });
    });
    if submit {
        actions.push(WatchlistAction::Import {
            list: list.id,
            text: text.clone(),
        });
    }
    if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.import_text = None;
        state.import_picker = None;
        state.import_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::watchlist::Watchlist;

    #[test]
    fn the_name_prompt_keeps_its_size_from_frame_to_frame() {
        let ctx = egui::Context::default();
        let mut state = WatchlistSidebarState {
            prompt: Some(PromptKind::NewSection(Watchlist::new("Crypto").id)),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let sizes: Vec<egui::Vec2> = (0..8)
            .map(|_| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::pos2(0.0, 0.0),
                            egui::vec2(900.0, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ui| prompt_window(ui.ctx(), &mut state, &mut actions),
                );
                output.textures_delta.clear();
                ctx.memory(|memory| {
                    memory
                        .area_rect(Id::new("watchlist-prompt"))
                        .map_or(egui::Vec2::ZERO, |rect| rect.size())
                })
            })
            .collect();
        let settled = sizes[3];
        assert!(settled.y > 100.0, "the sheet laid out: {sizes:?}");
        assert!(
            sizes[3..]
                .iter()
                .all(|size| (*size - settled).abs().max_elem() < 0.5),
            "a sheet that grows each frame is broken: {sizes:?}"
        );
        assert!(settled.x < PROMPT_WIDTH + 8.0, "width stays put: {sizes:?}");
    }

    fn lists() -> Vec<Watchlist> {
        let mut favorites = Watchlist::new("Favorites");
        favorites.is_favorites = true;
        vec![Watchlist::new("Crypto"), favorites]
    }

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(
            InstrumentId::new(DataSource::Binance, symbol),
            symbol,
            symbol,
        )
    }

    #[test]
    fn the_window_shows_its_pick_else_favorites_else_the_first() {
        let lists = lists();
        let mut state = WatchlistSidebarState::default();
        assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
        state.select_list(lists[0].id);
        assert_eq!(state.resolved(&lists).unwrap().name, "Crypto");
        // The picked list was deleted elsewhere: fall back, and remember the fallback.
        state.select_list(999);
        assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
        assert_eq!(state.selected_list_id(), Some(lists[1].id));
        assert!(state.resolved(&[]).is_none());
        assert_eq!(state.selected_list_id(), None);
        let only_one = vec![Watchlist::new("Solo")];
        assert_eq!(state.resolved(&only_one).unwrap().name, "Solo");
    }

    #[test]
    fn highlight_follows_the_focused_chart_and_only_resyncs_on_change() {
        let mut list = Watchlist::new("L");
        let btc = item("BTCUSDT");
        let eth = item("ETHUSDT");
        let (btc_id, eth_id) = (btc.id, eth.id);
        list.add(btc, None).unwrap();
        list.add(eth, None).unwrap();

        let mut state = WatchlistSidebarState::default();
        let focused = InstrumentId::new(DataSource::Binance, "ethusdt");
        state.sync_selection(&list, Some(&focused));
        assert_eq!(state.selected_entry, Some(eth_id));

        // A click selects BTC; with the same focus the next frame must not undo it.
        state.selected_entry = Some(btc_id);
        state.sync_selection(&list, Some(&focused));
        assert_eq!(state.selected_entry, Some(btc_id));

        // Focus moves to a market not in the list: nothing is highlighted.
        let elsewhere = InstrumentId::new(DataSource::Coinbase, "BTC-USD");
        state.sync_selection(&list, Some(&elsewhere));
        assert_eq!(state.selected_entry, None);
        state.sync_selection(&list, None);
        assert_eq!(state.selected_entry, None);
    }

    #[test]
    fn insertion_follows_row_middles() {
        let rect =
            |top: f32| Rect::from_min_size(Pos2::new(0.0, top), egui::Vec2::new(100.0, 40.0));
        let rects = [rect(0.0), rect(40.0), rect(80.0)];
        assert_eq!(insertion_index(&rects, -10.0), 0);
        assert_eq!(insertion_index(&rects, 19.0), 0);
        assert_eq!(insertion_index(&rects, 21.0), 1);
        assert_eq!(insertion_index(&rects, 95.0), 2);
        assert_eq!(insertion_index(&rects, 105.0), 3);
        assert_eq!(insertion_index(&[], 5.0), 0);
    }

    #[test]
    fn prompts_describe_themselves() {
        assert_eq!(PromptKind::NewList.confirm(), "Create");
        assert_eq!(PromptKind::RenameList(1).confirm(), "Rename");
        assert_eq!(PromptKind::CopyList(1).title(), "Make a Copy");
        assert_eq!(
            PromptKind::RenameSection {
                list: 1,
                section: 2
            }
            .message(),
            "Enter a new name."
        );
    }

    #[test]
    fn new_list_prompt_can_carry_a_market() {
        let mut state = WatchlistSidebarState::default();
        state.prompt_new_list_with(item("SOLUSDT"));
        assert!(matches!(state.prompt, Some(PromptKind::NewListWith(_))));
        assert_eq!(
            state.prompt.as_ref().unwrap().message(),
            "Name the new watchlist. SOLUSDT will be added to it."
        );
        assert!(state.prompt_text.is_empty());
    }

    /// The header's list selector, driven by pointer events.
    struct SelectorRig {
        ctx: egui::Context,
        state: WatchlistSidebarState,
        lists: Vec<Watchlist>,
        time: f64,
    }

    impl SelectorRig {
        const RECT: Rect = Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(160.0, 34.0));

        fn new(lists: Vec<Watchlist>) -> Self {
            Self {
                ctx: egui::Context::default(),
                state: WatchlistSidebarState::default(),
                lists,
                time: 0.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let flags = Flags::default();
            let quote = |_: &InstrumentId| None;
            let input = WatchlistSidebarInput {
                lists: &self.lists,
                flags: &flags,
                quote: &quote,
                focused_market: None,
                load_failed: false,
                error: None,
                import_report: None,
            };
            let state = &mut self.state;
            let mut output = self.ctx.run_ui(raw, |ui| {
                let list = state.resolved(input.lists);
                list_selector(ui, state, &input, list, Self::RECT);
            });
            output.textures_delta.clear();
        }

        fn click(&mut self, at: Pos2) {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            for _ in 0..3 {
                self.frame(Vec::new());
            }
        }

        fn open(&self) -> bool {
            egui::Popup::is_any_open(&self.ctx)
        }

        fn open_menu(&mut self) {
            self.frame(Vec::new());
            self.click(Self::RECT.left_center() + egui::vec2(10.0, 0.0));
            assert!(self.open(), "the selector opens its menu");
        }

        /// A point in row `index` of the list rows: under the selector, the frame's margin and
        /// the heading.
        fn list_row(&self, index: usize) -> Pos2 {
            Pos2::new(
                40.0,
                Self::RECT.center().y + 14.0 + 6.0 + 24.0 + 12.0 + 24.0 * index as f32,
            )
        }
    }

    #[test]
    fn the_list_menu_opens_from_the_title_and_a_click_picks_a_list() {
        let mut rig = SelectorRig::new(lists());
        let crypto = rig.lists[0].id;
        assert_ne!(rig.state.resolved(&rig.lists).map(|l| l.id), Some(crypto));
        rig.open_menu();
        rig.click(rig.list_row(0));
        assert_eq!(rig.state.selected_list_id(), Some(crypto));
        assert!(!rig.open(), "picking a list closes the menu");
    }

    #[test]
    fn clicking_outside_the_list_menu_closes_it_and_changes_nothing() {
        let mut rig = SelectorRig::new(lists());
        let before = rig.state.resolved(&rig.lists).map(|l| l.id);
        rig.open_menu();
        rig.click(Pos2::new(560.0, 560.0));
        assert!(!rig.open());
        assert_eq!(rig.state.resolved(&rig.lists).map(|l| l.id), before);
    }

    #[test]
    fn the_list_menu_asks_for_a_name_before_creating_a_list() {
        let mut rig = SelectorRig::new(lists());
        rig.open_menu();
        // Two lists, then the separator: the create row is the first row below them.
        let create = rig.list_row(2) + egui::vec2(0.0, 9.0);
        rig.click(create);
        assert!(matches!(rig.state.prompt, Some(PromptKind::NewList)));
    }

    #[test]
    fn the_detail_card_is_on_by_default() {
        assert!(WatchlistSidebarState::default().shows_detail);
    }

    /// Draws the detail card for a Binance market, with `quote`; returns the height it took.
    fn detail_card_height(quote: Option<WatchlistQuote>) -> f32 {
        let mut list = Watchlist::new("L");
        let btc = item("BTCUSDT");
        let id = btc.id;
        list.add(btc, None).unwrap();
        let lists = vec![list];
        let state = WatchlistSidebarState {
            selected_entry: Some(id),
            ..Default::default()
        };
        let flags = Flags::default();
        let lookup = |_: &InstrumentId| quote.clone();
        let input = WatchlistSidebarInput {
            lists: &lists,
            flags: &flags,
            quote: &lookup,
            focused_market: None,
            load_failed: false,
            error: None,
            import_report: None,
        };
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 600.0))),
                ..Default::default()
            },
            |ui| {
                detail_panel(ui, &state, &input, &lists[0]);
                height = ui.min_rect().height();
            },
        );
        output.textures_delta.clear();
        height
    }

    #[test]
    fn the_detail_card_is_as_tall_as_it_says_whatever_the_quote_holds() {
        let plain = detail_card_height(None);
        assert_eq!(plain, detail_height(None));
        let noted = WatchlistQuote::unsupported(Some("Delayed by the provider".to_owned()));
        let with_note = detail_card_height(Some(noted.clone()));
        assert_eq!(with_note, detail_height(Some(&noted)));
        assert!(with_note > plain, "a note takes a band of its own");
    }
}
