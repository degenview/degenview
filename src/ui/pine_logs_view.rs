//! The Pine logs pane: the messages a script wrote with `log.info()`, `log.warning()` and
//! `log.error()`, newest at the bottom, with the filters TradingView's pane has.
//!
//! A docked panel over `model::pine_logs_view`: a title, a toolbar (the script, the logging
//! switch, search, a start date and the levels), the list, and states for the times there is
//! nothing to list. It owns nothing of the messages: they are lent for the frame, and what the
//! person asks for is returned as a [`LogsAction`]. Only the rows on screen are laid out.

use crate::model::pine_logs_view::{
    FilteredLogs, LogFilter, LogsPrefs, MAX_HIGHLIGHTS, RowHeights, estimated_height, iso_utc,
};
use crate::pine::logs::{LogEvent, LogHistory, LogLevel};
use crate::ui::date_picker;
use crate::ui::icons::{self, Icon};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{
    self, Align, Color32, FontId, Layout, Rect, Sense, TextFormat, Ui, epaint::text::LayoutJob,
    pos2, vec2,
};
use std::sync::Arc;

const TITLE_HEIGHT: f32 = 44.0;
const TOOLBAR_HEIGHT: f32 = 46.0;
const BUTTON: f32 = 26.0;
const MARGIN: f32 = 12.0;
const FIELD_HEIGHT: f32 = 28.0;
const ROW_PAD_X: f32 = 12.0;
const ROW_PAD_Y: f32 = 5.0;
const TEXT_SIZE: f32 = 12.0;
const STAMP_SIZE: f32 = 10.5;
/// Rows laid out beyond the window, so a scroll does not show a blank edge.
const OVERSCAN: f32 = 240.0;
/// The row menu's width: its longest entry, "Scroll to bar", with its icon.
const MENU_WIDTH: f32 = 170.0;
/// The width from which the search bar and the date bar share a row.
const BARS_SIDE_BY_SIDE: f32 = 640.0;
/// The height of list a short pane keeps under the header when the search bars are open.
const MIN_LIST: f32 = 60.0;
const DOCS_URL: &str = "https://www.tradingview.com/pine-script-docs/writing/debugging/#pine-logs";

/// A script the pane can show: an instance on the chart.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEntry {
    pub id: u64,
    pub label: String,
}

/// What the pane reads of the selected script's run.
pub struct Feed<'a> {
    pub epoch: u64,
    pub history: &'a LogHistory,
    /// Whether the script keeps the messages it writes in realtime.
    pub logging: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LogsAction {
    Close,
    Select(u64),
    /// Turn the keeping of realtime messages on or off for a script.
    SetLogging(u64, bool),
    /// Show the source of the call that wrote the message with this sequence number.
    Source(u64),
    /// Bring the chart to the bar the message with this sequence number was written on.
    ScrollToBar(u64),
    /// Forget every message kept for the selected script.
    Clear,
}

#[derive(Default)]
pub struct LogsState {
    /// The script shown; the owner points it at one that exists before each frame.
    pub selected: Option<u64>,
    /// Shown inside something that has its own title (a tab): the title row and its close
    /// button are left out.
    pub embedded: bool,
    pub filter: LogFilter,
    pub(crate) search_open: bool,
    pub(crate) date_open: bool,
    /// The start date being typed or picked (`YYYY-MM-DD HH:MM`, UTC).
    date_text: String,
    date_error: bool,
    filtered: FilteredLogs,
    heights: RowHeights,
    /// What the rows were laid out for: the script, its run, the list, and how many rows.
    /// The panel's own id, which the ids of its fields hang from.
    base: Option<egui::Id>,
    layout_key: Option<(u64, u64)>,
    layout_revision: u64,
    listed: usize,
    jump_to_latest: bool,
}

impl LogsState {
    pub fn prefs(&self) -> LogsPrefs {
        LogsPrefs {
            levels: self.filter.levels,
            search: self.filter.options,
        }
    }

    /// Keeps `selected` on a script that exists: the first when it is gone or was never chosen.
    pub fn select_valid(&mut self, scripts: &[ScriptEntry]) {
        let valid = self
            .selected
            .is_some_and(|id| scripts.iter().any(|script| script.id == id));
        if !valid {
            self.selected = scripts.first().map(|script| script.id);
        }
    }

    fn clear_filters(&mut self) {
        self.filter.query.clear();
        self.filter.start_ms = None;
        self.filter.levels = Default::default();
    }
}

// MARK: Panel

pub fn show(
    ui: &mut Ui,
    state: &mut LogsState,
    scripts: &[ScriptEntry],
    feed: Option<Feed>,
) -> Vec<LogsAction> {
    let mut actions = Vec::new();
    ui.set_min_height(ui.available_height());
    state.base = Some(ui.id());
    // The header's rows touch: each is as tall as it needs to be, its content centred in it, so the
    // space above and below the toolbar is the same whatever is above it.
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    if !state.embedded {
        title(ui, &mut actions);
        divider(ui);
    }
    let bars = state.search_open || state.date_open || state.filter.start_ms.is_some();
    if bars {
        // The search and date rows may not push the list out of a short pane: past what is left
        // after a sliver of list, the header scrolls.
        let room = (ui.available_height() - MIN_LIST).max(TOOLBAR_HEIGHT);
        egui::ScrollArea::vertical()
            .id_salt("pine-logs-header")
            .max_height(room)
            .auto_shrink([false, true])
            .min_scrolled_height(0.0)
            .show(ui, |ui| {
                header(ui, state, scripts, feed.as_ref(), spacing, &mut actions);
            });
    } else {
        header(ui, state, scripts, feed.as_ref(), spacing, &mut actions);
    }
    divider(ui);
    ui.spacing_mut().item_spacing.y = spacing;
    match feed {
        None => no_script(ui),
        Some(feed) => {
            let script = state.selected.unwrap_or(0);
            body(ui, state, &feed, script, &mut actions);
        }
    }
    actions
}

/// The toolbar, and under it the search and date bars and the start-date tag when they are open.
fn header(
    ui: &mut Ui,
    state: &mut LogsState,
    scripts: &[ScriptEntry],
    feed: Option<&Feed>,
    spacing: f32,
    actions: &mut Vec<LogsAction>,
) {
    toolbar(ui, state, scripts, feed, actions);
    if state.search_open || state.date_open || state.filter.start_ms.is_some() {
        ui.spacing_mut().item_spacing.y = spacing;
        if state.search_open && state.date_open && ui.available_width() >= BARS_SIDE_BY_SIDE {
            // Wide enough, the two bars share a row rather than taking two.
            let half = ui.available_width() / 2.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.allocate_ui_with_layout(vec2(half, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = spacing;
                    search_bar(ui, state);
                });
                ui.allocate_ui_with_layout(vec2(half, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = spacing;
                    date_bar(ui, state);
                });
            });
        } else {
            if state.search_open {
                search_bar(ui, state);
            }
            if state.date_open {
                date_bar(ui, state);
            }
        }
        chips(ui, state);
    }
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::hairline());
}

fn title(ui: &mut Ui, actions: &mut Vec<LogsAction>) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TITLE_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(16.0);
            ui.label(kit::text(
                "Pine logs",
                kit::size::HEADLINE,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(10.0);
                let close = icons::button(
                    ui,
                    Icon::Xmark,
                    BUTTON,
                    theme::text_secondary(),
                    "Close Pine logs",
                );
                if close.clicked() {
                    actions.push(LogsAction::Close);
                }
            });
        },
    );
}

// MARK: Toolbar

fn toolbar(
    ui: &mut Ui,
    state: &mut LogsState,
    scripts: &[ScriptEntry],
    feed: Option<&Feed>,
    actions: &mut Vec<LogsAction>,
) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TOOLBAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
            ui.add_space(MARGIN);
            let buttons = BUTTON * 4.0 + 2.0 * 3.0;
            let picker = (width - MARGIN * 2.0 - buttons - 8.0).max(60.0);
            let labels: Vec<String> = scripts.iter().map(|script| script.label.clone()).collect();
            let current = scripts
                .iter()
                .position(|script| Some(script.id) == state.selected)
                .unwrap_or(0);
            if labels.is_empty() {
                let (rect, _) = ui.allocate_exact_size(vec2(picker, 24.0), Sense::hover());
                let galley = kit::galley(
                    ui,
                    "No scripts",
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_tertiary(),
                    picker - 16.0,
                );
                ui.painter().galley(
                    pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    theme::text_tertiary(),
                );
            } else if let Some(chosen) = kit::menu_picker(ui, picker, &labels, current, true)
                && scripts[chosen].id != state.selected.unwrap_or(0)
            {
                actions.push(LogsAction::Select(scripts[chosen].id));
            }
            ui.add_space(8.0);

            let logging = feed.is_none_or(|feed| feed.logging);
            let (icon, tip) = if logging {
                (Icon::PauseCircle, "Disable logging")
            } else {
                (Icon::PlayCircle, "Enable logging")
            };
            let tint = if logging {
                theme::text_secondary()
            } else {
                kit::ACCENT
            };
            let switch = ui.add_enabled_ui(feed.is_some(), |ui| {
                icons::button(ui, icon, BUTTON, tint, tip)
            });
            if switch.inner.clicked()
                && let Some(id) = state.selected
            {
                actions.push(LogsAction::SetLogging(id, !logging));
            }

            let searching = state.search_open || !state.filter.query.is_empty();
            let search = icons::button(
                ui,
                Icon::Magnifier,
                BUTTON,
                if searching {
                    kit::ACCENT
                } else {
                    theme::text_secondary()
                },
                "Search",
            );
            if search.clicked() {
                state.search_open = !state.search_open;
                if state.search_open {
                    let id = search_field_id(state);
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
            }

            let dated = state.date_open || state.filter.start_ms.is_some();
            let date = icons::button(
                ui,
                Icon::Clock,
                BUTTON,
                if dated {
                    kit::ACCENT
                } else {
                    theme::text_secondary()
                },
                "Start date",
            );
            if date.clicked() {
                state.date_open = !state.date_open;
                if state.date_open && state.date_text.is_empty() {
                    state.date_text = state
                        .filter
                        .start_ms
                        .map(|ms| crate::model::portfolio_editor::date_text(ms, 0))
                        .unwrap_or_default();
                }
            }

            let filtered = !state.filter.levels.all();
            let more = icons::button(
                ui,
                Icon::Ellipsis,
                BUTTON,
                if filtered {
                    kit::ACCENT
                } else {
                    theme::text_secondary()
                },
                "Filter levels",
            );
            levels_menu(&more, state);
        },
    );
}

fn levels_menu(button: &egui::Response, state: &mut LogsState) {
    kit::menu_popup(button).width(190.0).show(|ui| {
        kit::menu_scope(ui, 170.0);
        kit::menu_heading(ui, "Filter levels");
        for (level, label) in [
            (LogLevel::Info, "Info"),
            (LogLevel::Warning, "Warning"),
            (LogLevel::Error, "Error"),
        ] {
            let mut row = kit::MenuRow::new(label);
            row.checked = state.filter.levels.shows(level);
            if kit::menu_row(ui, row).clicked() {
                state.filter.levels.toggle(level);
            }
        }
    });
}

// MARK: Search and date

/// The ids are derived from the panel's own, so the same pane in two tabs or windows (one
/// context) never shares a field's focus or a calendar's open flag.
fn search_field_id(state: &LogsState) -> egui::Id {
    state
        .base
        .unwrap_or_else(|| egui::Id::new("pine-logs"))
        .with("search")
}

fn search_bar(ui: &mut Ui, state: &mut LogsState) {
    let invalid = state.filter.options.regex
        && !state.filter.query.is_empty()
        && state.filtered.search().is_invalid();
    ui.add_space(2.0);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FIELD_HEIGHT), Sense::hover());
    let well = Rect::from_min_max(
        pos2(rect.left() + MARGIN, rect.top()),
        pos2(rect.right() - MARGIN, rect.bottom()),
    );
    let id = search_field_id(state);
    let focused = ui.memory(|memory| memory.has_focus(id));
    ui.painter().rect_filled(well, 8.0, theme::ink(10));
    let (stroke_color, stroke_width) = if invalid {
        (theme::DOWN, 1.5)
    } else if focused {
        (kit::ACCENT, 1.5)
    } else {
        (theme::ink(14), 1.0)
    };
    ui.painter().rect_stroke(
        well,
        8.0,
        egui::Stroke::new(stroke_width, stroke_color),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(well.left() + 15.0, well.center().y), vec2(12.0, 12.0)),
        Icon::Magnifier,
        theme::text_tertiary(),
    );

    // From the right: clear, then the three options, each a small toggle.
    let mut right = well.right() - 6.0;
    let mut slot = |ui: &mut Ui, width: f32, id_part: &str| {
        let slot = Rect::from_min_max(
            pos2(right - width, well.top() + 3.0),
            pos2(right, well.bottom() - 3.0),
        );
        right -= width + 2.0;
        (slot, ui.interact(slot, id.with(id_part), Sense::click()))
    };
    if !state.filter.query.is_empty() {
        let (slot, clear) = slot(ui, 20.0, "clear");
        let clear = accessibility::label(clear, "Clear search").on_hover_text("Clear search");
        if clear.hovered() {
            ui.painter().rect_filled(slot, 10.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), vec2(8.0, 8.0)),
            Icon::Xmark,
            theme::text_secondary(),
        );
        if clear.clicked() {
            state.filter.query.clear();
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
    let options = &mut state.filter.options;
    for (label, tip, flag, key) in [
        (".*", "Use regular expression", &mut options.regex, "regex"),
        ("ab", "Match whole word", &mut options.whole_word, "word"),
        ("Aa", "Match case", &mut options.match_case, "case"),
    ] {
        let (slot, response) = slot(ui, 24.0, key);
        let response = accessibility::label(response, tip).on_hover_text(tip);
        if *flag {
            ui.painter()
                .rect_filled(slot, 5.0, kit::ACCENT.gamma_multiply(0.28));
        } else if response.hovered() {
            ui.painter().rect_filled(slot, 5.0, theme::ink(14));
        }
        ui.painter().text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.0),
            if *flag {
                kit::ACCENT
            } else {
                theme::text_secondary()
            },
        );
        if response.clicked() {
            *flag = !*flag;
        }
    }
    let field = Rect::from_min_max(
        pos2(well.left() + 28.0, well.top()),
        pos2(right - 2.0, well.bottom()),
    );
    let edit = egui::TextEdit::singleline(&mut state.filter.query)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text("Search")
        .font(egui::FontId::proportional(kit::size::BODY))
        .margin(egui::Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(field.width());
    let response = ui.put(field, edit);
    accessibility::label(response, "Search logs");
    if invalid {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(MARGIN + 2.0);
            ui.label(kit::text(
                "Not a regular expression this search can read.",
                kit::size::CAPTION,
                kit::REGULAR,
                theme::DOWN,
            ));
        });
    }
    // Escape closes the bar while the field has the focus, and keeps what was typed.
    if focused && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.search_open = false;
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }
    ui.add_space(4.0);
}

fn date_bar(ui: &mut Ui, state: &mut LogsState) {
    ui.add_space(2.0);
    let width = ui.available_width() - MARGIN * 2.0;
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        let id = state
            .base
            .unwrap_or_else(|| egui::Id::new("pine-logs"))
            .with("start-date");
        let apply_width = 54.0;
        date_picker::show(
            ui,
            id,
            &mut state.date_text,
            0,
            (width - apply_width - 6.0).max(120.0),
            date_picker::Look::Compact,
            state.date_error,
        );
        if kit::push_button(ui, None, "Apply", true).clicked() {
            match crate::model::portfolio_editor::parse_date_text(&state.date_text, 0) {
                Some(ms) => {
                    state.filter.start_ms = Some(ms);
                    state.date_error = false;
                    state.date_open = false;
                    date_picker::close(ui.ctx(), id);
                }
                None => state.date_error = true,
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(MARGIN + 2.0);
        ui.label(kit::text(
            "Show messages from this time on (UTC).",
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
    ui.add_space(4.0);
}

/// The start date as a tag above the list, with a way to take it away.
fn chips(ui: &mut Ui, state: &mut LogsState) {
    let Some(start) = state.filter.start_ms else {
        return;
    };
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        let text = format!(
            "From {}",
            crate::model::portfolio_editor::date_text(start, 0)
        );
        let galley = kit::galley(
            ui,
            &text,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_primary(),
            f32::INFINITY,
        );
        let size = vec2(galley.size().x + 36.0, 22.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        ui.painter()
            .rect_filled(rect, 11.0, kit::ACCENT.gamma_multiply(0.22));
        ui.painter().galley(
            pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
        let cross =
            Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(8.0, 8.0));
        icons::paint(ui.painter(), cross, Icon::Xmark, theme::text_secondary());
        let response = accessibility::label(response, "Remove the start date")
            .on_hover_text("Remove the start date");
        if response.clicked() {
            state.filter.start_ms = None;
            state.date_text.clear();
        }
    });
    ui.add_space(6.0);
}

// MARK: Body

fn body(
    ui: &mut Ui,
    state: &mut LogsState,
    feed: &Feed,
    script: u64,
    actions: &mut Vec<LogsAction>,
) {
    let buffer = feed.history.buffer();
    state.filtered.refresh(feed.epoch, buffer, &state.filter);
    if buffer.is_empty() {
        return if feed.logging {
            onboarding(ui)
        } else {
            logging_off(ui, script, actions)
        };
    }
    if state.filtered.indices().is_empty() {
        return no_results(ui, state);
    }
    list(ui, state, feed, script, actions);
}

fn list(
    ui: &mut Ui,
    state: &mut LogsState,
    feed: &Feed,
    script: u64,
    actions: &mut Vec<LogsAction>,
) {
    let buffer = feed.history.buffer();
    let width = ui.available_width();
    let text_width = (width - ROW_PAD_X * 2.0).max(40.0);
    let chars_per_line = (text_width / 6.4) as usize;
    let estimate = |position: usize| {
        let index = state.filtered.indices()[position] as usize;
        buffer.get(index).map_or((0, 0.0), |event| {
            (
                event.sequence,
                estimated_height(&event.message, 16.0, ROW_PAD_Y * 2.0 + 13.0, chars_per_line),
            )
        })
    };
    let key = (script, feed.epoch);
    let count = state.filtered.indices().len();
    let resized = (text_width - state.heights.width()).abs() > 0.5;
    let revision = state.filtered.revision();
    // Rows are known by position, so a list that was rebuilt (a filter, the oldest message
    // evicted) puts other messages at the same positions even when it is as long as before.
    let rebuilt = revision != state.layout_revision && state.filtered.rebuilt();
    let fresh = state.layout_key != Some(key);
    if fresh || rebuilt || resized || state.listed > count {
        state.heights.reset(text_width, (0..count).map(&estimate));
        state.layout_key = Some(key);
        state.jump_to_latest |= fresh;
    } else if state.listed < count {
        state.heights.extend((state.listed..count).map(&estimate));
    }
    state.listed = count;
    state.layout_revision = revision;

    let area = egui::ScrollArea::vertical()
        .id_salt("pine-logs-rows")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .animated(false)
        .stick_to_bottom(true);
    let jump = std::mem::take(&mut state.jump_to_latest);
    let search = &state.filtered;
    let heights = &mut state.heights;
    let mut remeasured = false;
    let output = area.show_viewport(ui, |ui, viewport| {
        let total = heights.total();
        let (all, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
        if jump {
            let end = Rect::from_min_max(pos2(all.left(), all.bottom() - 1.0), all.max);
            ui.scroll_to_rect(end, Some(Align::BOTTOM));
        }
        let range = heights.rows_between(viewport.min.y - OVERSCAN, viewport.max.y + OVERSCAN);
        for position in range {
            let Some(event) = buffer.get(search.indices()[position] as usize) else {
                continue;
            };
            let top = heights.top_of(position);
            let rect = Rect::from_min_size(
                all.min + vec2(0.0, top),
                vec2(width, heights.height_of(position)),
            );
            let galley = row_galley(ui, event, search, text_width);
            let height = galley.size().y + ROW_PAD_Y * 2.0;
            let id = ui.id().with(("log-row", event.sequence));
            let response = ui.interact(rect, id, Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::ink(8));
            }
            ui.painter().galley(
                rect.min + vec2(ROW_PAD_X, ROW_PAD_Y),
                galley,
                Color32::PLACEHOLDER,
            );
            ui.painter().hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                egui::Stroke::new(1.0, theme::ink(10)),
            );
            row_menu(&response, event, actions);
            if ui.rect_contains_pointer(rect) {
                row_buttons(ui, rect, event.sequence, actions);
            }
            if (height - heights.height_of(position)).abs() > 0.01 {
                heights.measure(position, event.sequence, height);
                remeasured = true;
            } else {
                heights.measure(position, event.sequence, height);
            }
        }
    });
    if remeasured {
        ui.ctx().request_repaint();
    }
    let at_bottom =
        output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 2.0;
    if !at_bottom {
        let pill = Rect::from_center_size(
            pos2(
                output.inner_rect.right() - 54.0,
                output.inner_rect.bottom() - 22.0,
            ),
            vec2(92.0, 26.0),
        );
        let response = ui.interact(pill, ui.id().with("jump-to-latest"), Sense::click());
        let fill = if response.hovered() {
            kit::ACCENT
        } else {
            kit::ACCENT.gamma_multiply(0.85)
        };
        ui.painter().rect_filled(pill, 13.0, fill);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(pill.left() + 17.0, pill.center().y), vec2(10.0, 10.0)),
            Icon::ArrowDown,
            Color32::WHITE,
        );
        ui.painter().text(
            pos2(pill.left() + 30.0, pill.center().y),
            egui::Align2::LEFT_CENTER,
            "Latest",
            FontId::proportional(11.5),
            Color32::WHITE,
        );
        if accessibility::label(response, "Scroll to the latest message").clicked() {
            state.jump_to_latest = true;
            ui.ctx().request_repaint();
        }
    }
}

fn level_color(level: LogLevel) -> Color32 {
    match level {
        LogLevel::Info => theme::text_secondary(),
        LogLevel::Warning => kit::ORANGE,
        LogLevel::Error => kit::RED,
    }
}

/// One message as the row draws it: its time in UTC, then the text under it, with the search
/// matches lit.
/// egui keeps the shaped result between frames, so scrolling over the same rows reshapes nothing.
fn row_galley(ui: &Ui, event: &LogEvent, filtered: &FilteredLogs, width: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    // A long token with no break in it wraps instead of widening the pane.
    job.wrap.break_anywhere = true;
    job.append(
        &iso_utc(event.time_ms),
        0.0,
        TextFormat {
            font_id: FontId::monospace(STAMP_SIZE),
            color: theme::text_tertiary(),
            ..Default::default()
        },
    );
    job.append(
        "\n",
        0.0,
        TextFormat {
            font_id: FontId::monospace(STAMP_SIZE),
            ..Default::default()
        },
    );
    let color = level_color(event.level);
    let plain = TextFormat {
        font_id: FontId::proportional(TEXT_SIZE),
        color,
        ..Default::default()
    };
    let lit = TextFormat {
        background: kit::ACCENT.gamma_multiply(0.45),
        color: match event.level {
            LogLevel::Info => theme::text_primary(),
            _ => color,
        },
        ..plain.clone()
    };
    let message = event.message.as_str();
    let mut at = 0;
    let mut lead = 0.0;
    for range in filtered.search().find(message, MAX_HIGHLIGHTS) {
        if range.start > at {
            job.append(&message[at..range.start], lead, plain.clone());
            lead = 0.0;
        }
        job.append(&message[range.clone()], lead, lit.clone());
        lead = 0.0;
        at = range.end;
    }
    if at < message.len() || message.is_empty() {
        job.append(&message[at..], lead, plain);
    }
    ui.painter().layout_job(job)
}

/// The two actions a hovered row offers, at the right end of its time line.
fn row_buttons(ui: &mut Ui, row: Rect, sequence: u64, actions: &mut Vec<LogsAction>) {
    let size = 22.0;
    for (slot, (icon, tip, action)) in [
        (Icon::Code, "Source code", LogsAction::Source(sequence)),
        (
            Icon::ScrollToBar,
            "Scroll to bar",
            LogsAction::ScrollToBar(sequence),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(
            pos2(
                row.right() - 8.0 - size * (2 - slot) as f32 - 2.0 * (1 - slot) as f32,
                row.top() + 2.0,
            ),
            vec2(size, size),
        );
        let response = ui.interact(
            rect,
            ui.id().with(("log-action", sequence, slot)),
            Sense::click(),
        );
        let response = accessibility::label(response, tip).on_hover_text(tip);
        let fill = if response.is_pointer_button_down_on() {
            theme::ink(34)
        } else if response.hovered() {
            theme::ink(22)
        } else {
            theme::chrome()
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(13.0, 13.0)),
            icon,
            theme::text_secondary(),
        );
        if response.clicked() {
            actions.push(action);
        }
    }
}

fn row_menu(response: &egui::Response, event: &LogEvent, actions: &mut Vec<LogsAction>) {
    kit::context_popup(response).width(MENU_WIDTH).show(|ui| {
        kit::menu_scope(ui, MENU_WIDTH - 20.0);
        if kit::menu_item(ui, Icon::Code, "Source code", true, false) {
            actions.push(LogsAction::Source(event.sequence));
            ui.close();
        }
        if kit::menu_item(ui, Icon::ScrollToBar, "Scroll to bar", true, false) {
            actions.push(LogsAction::ScrollToBar(event.sequence));
            ui.close();
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::CopyDoc, "Copy message", true, false) {
            ui.ctx().copy_text(event.message.clone());
            ui.close();
        }
        if kit::menu_item(ui, Icon::CopyDoc, "Copy with time", true, false) {
            ui.ctx()
                .copy_text(format!("{} {}", iso_utc(event.time_ms), event.message));
            ui.close();
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Trash, "Clear logs", true, true) {
            actions.push(LogsAction::Clear);
            ui.close();
        }
    });
}

// MARK: Empty states

fn no_script(ui: &mut Ui) {
    centered(ui, |ui| {
        heading(ui, "No scripts on this chart");
        detail(
            ui,
            "Add a Pine script that calls the log functions and its messages appear here.",
        );
    });
}

fn no_results(ui: &mut Ui, state: &mut LogsState) {
    centered(ui, |ui| {
        heading(ui, "No matching messages");
        detail(ui, "No message passes the level, date and search filters.");
        ui.add_space(10.0);
        if kit::push_button(ui, None, "Clear filters", true).clicked() {
            state.clear_filters();
        }
    });
}

fn logging_off(ui: &mut Ui, script: u64, actions: &mut Vec<LogsAction>) {
    centered(ui, |ui| {
        heading(ui, "Logging is disabled");
        detail(
            ui,
            "Messages the script writes in realtime are not kept while logging is off.",
        );
        ui.add_space(10.0);
        if kit::push_button(ui, Some(Icon::PlayCircle), "Enable logging", true).clicked() {
            actions.push(LogsAction::SetLogging(script, true));
        }
    });
}

fn onboarding(ui: &mut Ui) {
    centered(ui, |ui| {
        intro(
            ui,
            "How to work with logs",
            "See what your script does, bar by bar.",
            |ui| {
                step(ui, 1, |ui| {
                    ui.label(muted("Add the indicator to the chart."));
                });
                step(ui, 2, |ui| {
                    ui.label(muted("Use the "));
                    let link = ui.add(
                        egui::Label::new(
                            kit::text(
                                "log functions",
                                kit::size::SUBHEADLINE,
                                kit::MEDIUM,
                                kit::ACCENT,
                            )
                            .underline(),
                        )
                        .sense(Sense::click()),
                    );
                    if link
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(DOCS_URL));
                    }
                    ui.label(muted("."));
                });
                step(ui, 3, |ui| {
                    ui.label(muted("Enable logging."));
                });
            },
            example,
            illustration,
            |_| {},
        );
    });
}

/// The width from which an introduction puts its picture beside its steps.
const INTRO_WIDE: f32 = 520.0;

/// What a tab says while it has nothing to show: a picture and what the tab is for, a card of
/// numbered steps, an example, and whatever `footer` adds under the words (a button). Side by side
/// where the width allows, so it takes half the height; one column otherwise. Call it inside
/// [`centered`].
pub(crate) fn intro(
    ui: &mut Ui,
    heading: &str,
    subheading: &str,
    steps: impl FnOnce(&mut Ui),
    example: impl FnOnce(&mut Ui, f32),
    picture: impl FnOnce(&Ui, Rect),
    footer: impl FnOnce(&mut Ui),
) {
    let available = ui.available_width();
    let words = |ui: &mut Ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 84.0), Sense::hover());
        picture(ui, rect);
        ui.add_space(16.0);
        ui.label(kit::text(
            heading,
            14.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(3.0);
        ui.label(muted(subheading));
    };
    let card = |ui: &mut Ui, width: f32| {
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            egui::Frame::new()
                .fill(theme::ink(7))
                .stroke(egui::Stroke::new(1.0, theme::ink(14)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(12, 11))
                .show(ui, |ui| {
                    ui.set_width(width - 26.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 10.0);
                    steps(ui);
                });
            if width >= 220.0 {
                ui.add_space(10.0);
                example(ui, width);
            }
        });
    };
    if available >= INTRO_WIDE {
        let (left, right, gap) = (210.0, 268.0, 28.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(((available - left - right - gap) / 2.0).max(0.0));
            ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Center), |ui| {
                ui.set_width(left);
                words(ui);
                footer(ui);
            });
            card(ui, right);
        });
    } else {
        words(ui);
        ui.add_space(16.0);
        card(ui, available.min(268.0));
        footer(ui);
    }
}

/// A call as a script writes it, in the editor's colours.
fn example(ui: &mut Ui, width: f32) {
    let mono = |color: Color32| TextFormat {
        font_id: FontId::monospace(11.0),
        color,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append("log.info", 0.0, mono(theme::syntax::function()));
    job.append("(", 0.0, mono(theme::text_secondary()));
    job.append("\"close {0}\"", 0.0, mono(theme::syntax::string()));
    job.append(", ", 0.0, mono(theme::text_secondary()));
    job.append("close", 0.0, mono(theme::syntax::variable()));
    job.append(")", 0.0, mono(theme::text_secondary()));
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::well_fill());
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 9.0),
        galley,
        Color32::PLACEHOLDER,
    );
}

/// One numbered line of the instructions.
pub(crate) fn step(ui: &mut Ui, number: usize, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (badge, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        ui.painter()
            .circle_filled(badge.center(), 10.0, kit::ACCENT.gamma_multiply(0.16));
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            FontId::proportional(10.5),
            kit::ACCENT,
        );
        ui.add_space(10.0);
        content(ui);
    });
}

fn muted(text: &str) -> egui::RichText {
    kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
    )
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
}

fn detail(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_tertiary(),
    ));
}

/// Content in the middle of what is left of the pane, both ways. Its height is measured as it is
/// laid out and used the next frame, as the first frame cannot know it. When it is taller than the
/// pane it starts at the top and scrolls, rather than being cut off.
pub(crate) fn centered(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let id = ui.id().with("centered-height");
    let known = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0);
    let room = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            ui.add_space(((room - known) / 2.0).max(12.0));
            let top = ui.cursor().top();
            ui.vertical_centered(|ui| {
                ui.set_max_width((ui.available_width() - MARGIN * 3.0).max(120.0));
                content(ui);
            });
            let used = ui.cursor().top() - top;
            ui.add_space(12.0);
            if (used - known).abs() > 0.5 {
                ui.data_mut(|data| data.insert_temp(id, used));
                ui.ctx().request_repaint();
            }
        });
}

/// A page of log lines on a soft disc: a gray, an orange and a red message, the three levels.
fn illustration(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    let centre = rect.center();
    painter.circle_filled(centre, 40.0, kit::ACCENT.gamma_multiply(0.10));
    let card = Rect::from_center_size(centre, vec2(64.0, 54.0));
    painter.rect_filled(card, 9.0, theme::chrome());
    painter.rect_stroke(
        card,
        9.0,
        egui::Stroke::new(1.5, theme::text_tertiary()),
        egui::StrokeKind::Inside,
    );
    let rows = [
        (theme::text_tertiary(), 0.62),
        (kit::ORANGE, 0.46),
        (kit::RED, 0.70),
    ];
    for (index, (color, length)) in rows.into_iter().enumerate() {
        let y = card.top() + 14.0 + index as f32 * 13.0;
        painter.circle_filled(pos2(card.left() + 12.0, y), 2.6, color);
        painter.hline(
            card.left() + 20.0..=card.left() + 20.0 + (card.width() - 32.0) * length,
            y,
            egui::Stroke::new(2.2, color.gamma_multiply(0.75)),
        );
    }
}

#[cfg(test)]
mod tests;
