//! The MCP activity log in Settings ▸ MCP Server: a short preview on the page, and the sheet it
//! opens, where the whole log can be searched, narrowed by client, outcome and date, and paged.
//!
//! A pure view over the rows the host already holds in memory ([`ActivityInput`]). It never reads
//! the store: the only thing it asks of the host is [`McpAction::ClearActivity`], and only after
//! the sheet's own "Clear the log?" question has been answered.

use crate::model::mcp::activity::{
    ACTIVITY_PAGE_SIZE, McpActivityClient, McpActivityFilter, McpActivityRange, McpActivityStatus,
    Paging, clients_in,
};
use crate::model::mcp::{McpAction, McpAuditEntry, McpAuditKind};
use crate::model::portfolio::editor::{date_text, parse_date_text};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::time_text;
use crate::ui::kit::{Tone, time_text::relative_time};
use crate::ui::settings::mcp::capsule;
use crate::ui::settings::window::{card, divider, section};
use crate::ui::{
    kit::{self, date_picker, sheet},
    theme,
};
use eframe::egui::{self, Align, Color32, Id, Layout, Sense, Stroke, StrokeKind, Ui, vec2};

/// Rows the page itself shows; the sheet has the rest.
pub const PREVIEW_ROWS: usize = 5;

const SHEET_MIN_WIDTH: f32 = 340.0;
const SHEET_MAX_WIDTH: f32 = 760.0;
/// Room the sheet leaves on each side of itself in a narrow window.
const SHEET_MARGIN: f32 = 24.0;
/// Width of the outcome tag on a row, so the tags form one column.
const TAG_WIDTH: f32 = 70.0;
/// Room for "Nov 7, 2026 11:01:19", the full stamp a wide row ends with.
const STAMP_WIDTH: f32 = 142.0;
/// Room for "12 min. ago", all a narrow row (or the page's preview) ends with.
const AGO_WIDTH: f32 = 82.0;
/// A row shows its full stamp only when the list is at least this wide.
const WIDE_LIST: f32 = 470.0;
const CLIENT_PICKER_WIDTH: f32 = 176.0;
const STATUS_PICKER_WIDTH: f32 = 128.0;
const RANGE_PICKER_WIDTH: f32 = 148.0;
const DATE_FIELD_WIDTH: f32 = 156.0;
/// What a custom range starts from when it is first chosen.
const DEFAULT_FROM_AGO_MS: i64 = 24 * 3_600_000;

/// What the page and the sheet draw from.
pub struct ActivityInput<'a> {
    /// The whole log, newest first.
    pub entries: &'a [McpAuditEntry],
    /// Rows in the log this version cannot read, and so cannot show.
    pub unreadable: usize,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

/// The sheet's open flag and everything the user has set in it. It lasts as long as the sheet
/// is open: closing it forgets the filters, so it never opens on a stale, empty-looking view.
#[derive(Default)]
pub struct ActivityState {
    open: bool,
    search: String,
    client_id: Option<String>,
    status: Option<McpActivityStatus>,
    range: RangeChoice,
    /// The custom range's ends as the date picker edits them (`YYYY-MM-DD HH:MM`); empty is
    /// "no limit on this side".
    from_text: String,
    to_text: String,
    page: usize,
    /// The footer is asking "Clear the log?".
    confirm_clear: bool,
}

/// The date menu's choice: a preset, or the two pickers below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RangeChoice {
    #[default]
    Any,
    Preset(McpActivityRange),
    Custom,
}

impl ActivityState {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            ..Self::default()
        };
    }

    /// The whole sheet, closed and forgotten.
    fn close(&mut self, ctx: &egui::Context) {
        date_picker::close(ctx, from_id());
        date_picker::close(ctx, to_id());
        *self = Self::default();
    }

    /// What the controls ask for right now.
    fn filter(&self, utc_offset_secs: i64) -> McpActivityFilter {
        McpActivityFilter {
            client_id: self.client_id.clone(),
            status: self.status,
            search: self.search.clone(),
            range: match self.range {
                RangeChoice::Any => McpActivityRange::Any,
                RangeChoice::Preset(range) => range,
                RangeChoice::Custom => McpActivityRange::Custom {
                    from_ms: parse_date_text(&self.from_text, utc_offset_secs),
                    to_ms: parse_date_text(&self.to_text, utc_offset_secs),
                },
            },
        }
    }

    /// Back to every row.
    fn clear_filters(&mut self, ctx: &egui::Context) {
        date_picker::close(ctx, from_id());
        date_picker::close(ctx, to_id());
        self.search.clear();
        self.client_id = None;
        self.status = None;
        self.range = RangeChoice::Any;
        self.from_text.clear();
        self.to_text.clear();
    }
}

fn from_id() -> Id {
    Id::new("mcp-activity-from")
}

fn to_id() -> Id {
    Id::new("mcp-activity-to")
}

/// What the sheet lists: the rows that match, and the page of them in view.
struct View<'a> {
    filter: McpActivityFilter,
    matching: Vec<&'a McpAuditEntry>,
    paging: Paging,
}

impl<'a> View<'a> {
    fn of(state: &mut ActivityState, input: &ActivityInput<'a>) -> Self {
        let filter = state.filter(input.utc_offset_secs);
        let matching = filter.select(input.entries, input.now_ms, input.utc_offset_secs);
        let paging = Paging::new(matching.len(), state.page, ACTIVITY_PAGE_SIZE);
        state.page = paging.page;
        Self {
            filter,
            matching,
            paging,
        }
    }

    fn page_rows(&self) -> &[&'a McpAuditEntry] {
        &self.matching[self.paging.items.clone()]
    }
}

// MARK: Rows

/// "degenview_get_pine_report" as "Get pine report".
fn tool_title(tool: &str) -> String {
    let words = tool
        .strip_prefix("degenview_")
        .unwrap_or(tool)
        .replace('_', " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// "chart_id" `3` as "chart 3".
fn target_text(kind: &str, id: &str) -> String {
    format!(
        "{} {id}",
        kind.strip_suffix("_id").unwrap_or(kind).replace('_', " ")
    )
}

fn status_tone(status: McpActivityStatus) -> Tone {
    match status {
        McpActivityStatus::Done | McpActivityStatus::Approved => Tone::Good,
        McpActivityStatus::Failed => Tone::Bad,
        McpActivityStatus::Denied
        | McpActivityStatus::Held
        | McpActivityStatus::Declined
        | McpActivityStatus::RateLimited => Tone::Warning,
        // Over without anything having gone wrong or been refused.
        McpActivityStatus::Cancelled | McpActivityStatus::Expired => Tone::Neutral,
    }
}

/// A row's tag: its text and tint.
fn outcome_tag(entry: &McpAuditEntry) -> (&'static str, Tone) {
    match (entry.status(), entry.kind) {
        (Some(status), _) => (status.label(), status_tone(status)),
        (None, McpAuditKind::Gap) => ("Gap", Tone::Warning),
        (None, _) => ("Cleared", Tone::Neutral),
    }
}

/// A row's headline.
fn row_title(entry: &McpAuditEntry) -> String {
    match entry.kind {
        McpAuditKind::Call | McpAuditKind::Confirmation => tool_title(&entry.tool),
        McpAuditKind::RateLimited => "Rate limit reached".to_owned(),
        McpAuditKind::AuthRefused => "Sign-in refused".to_owned(),
        McpAuditKind::Cleared => "Activity log cleared".to_owned(),
        McpAuditKind::Gap => "Some activity was not recorded".to_owned(),
    }
}

fn plural(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The small print under a headline: who, what it named, how it ended and how long it took.
fn row_detail(entry: &McpAuditEntry) -> String {
    let mut parts: Vec<String> = Vec::new();
    match entry.kind {
        McpAuditKind::Call => {
            if !entry.client_name.is_empty() {
                parts.push(entry.client_name.clone());
            }
            if !entry.targets.is_empty() {
                parts.push(
                    entry
                        .targets
                        .iter()
                        .map(|target| target_text(&target.kind, &target.id))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            if !entry.code.is_empty() {
                parts.push(entry.code.clone());
            }
            if !entry.peer.is_empty() {
                parts.push(format!("from {}", entry.peer));
            }
            parts.push(format!("{} ms", entry.duration_ms));
        }
        McpAuditKind::AuthRefused => parts.push(format!(
            "{} from {} refused within a minute",
            plural(entry.count, "request", "requests"),
            entry.peer
        )),
        McpAuditKind::Confirmation => {
            if !entry.client_name.is_empty() {
                parts.push(entry.client_name.clone());
            }
            parts.push(
                match entry.status() {
                    Some(McpActivityStatus::Approved) => "you approved it",
                    Some(McpActivityStatus::Declined) => "you declined it",
                    Some(McpActivityStatus::Expired) => "nobody answered in time",
                    _ => "waiting for you",
                }
                .to_owned(),
            );
        }
        McpAuditKind::RateLimited => {
            parts.push(entry.client_name.clone());
            if !entry.peer.is_empty() {
                parts.push(format!("from {}", entry.peer));
            }
            parts.push(format!(
                "{} refused within a minute",
                plural(entry.count, "call", "calls")
            ));
        }
        McpAuditKind::Gap => parts.push(format!(
            "{} dropped while the app was busy",
            plural(entry.count, "row", "rows")
        )),
        McpAuditKind::Cleared => parts.push("Earlier activity was deleted".to_owned()),
    }
    parts.join(" · ")
}

/// "Nov 7, 2026 11:01:19" in the viewer's time zone.
fn stamp(ms: i64, utc_offset_secs: i64) -> String {
    let seconds = (ms.div_euclid(1_000) + utc_offset_secs).rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}:{:02}",
        time_text::date_text(ms + utc_offset_secs * 1_000),
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// Everything a row holds, for its tooltip.
fn row_tooltip(entry: &McpAuditEntry, utc_offset_secs: i64) -> String {
    let mut lines = vec![stamp(entry.at_ms, utc_offset_secs)];
    if !entry.client_name.is_empty() || !entry.client_id.is_empty() {
        lines.push(format!(
            "Client: {} ({})",
            if entry.client_name.is_empty() {
                "unnamed"
            } else {
                &entry.client_name
            },
            entry.client_id
        ));
    }
    if !entry.peer.is_empty() {
        lines.push(format!("From: {}", entry.peer));
    }
    if !entry.tool.is_empty() {
        lines.push(format!("Tool: {}", entry.tool));
    }
    if let Some(status) = entry.status() {
        lines.push(if entry.code.is_empty() {
            format!("Result: {}", status.label())
        } else {
            format!("Result: {} ({})", status.label(), entry.code)
        });
    }
    if entry.kind == McpAuditKind::Call {
        lines.push(format!("Took {} ms", entry.duration_ms));
    }
    if !entry.targets.is_empty() {
        lines.push(format!(
            "Named: {}",
            entry
                .targets
                .iter()
                .map(|target| target_text(&target.kind, &target.id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !entry.request_id.is_empty() {
        lines.push(format!("Request id: {}", entry.request_id));
    }
    lines.push(format!("Entry id: {}", entry.id));
    lines.join("\n")
}

/// One row: tag, headline and small print, and when it happened.
fn row(ui: &mut Ui, entry: &McpAuditEntry, input: &ActivityInput, full_stamp: bool) {
    let (tag, tone) = outcome_tag(entry);
    let width = ui.available_width();
    let time_width = if full_stamp { STAMP_WIDTH } else { AGO_WIDTH };
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            capsule(ui, tag, tone, Some(TAG_WIDTH));
            let text_width =
                (width - TAG_WIDTH - time_width - 2.0 * ui.spacing().item_spacing.x).max(40.0);
            ui.vertical(|ui| {
                ui.set_width(text_width);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(
                    egui::Label::new(kit::text(
                        row_title(entry),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::REGULAR,
                        theme::text_primary(),
                    ))
                    .truncate(),
                );
                let detail = row_detail(entry);
                if !detail.is_empty() {
                    ui.add(
                        egui::Label::new(kit::text(
                            detail,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .truncate(),
                    );
                }
            });
            // A column of exactly the width kept for it, its lines flush right, so the row never
            // outgrows its card.
            time_column(ui, entry, input, time_width, full_stamp);
        })
        .response;
    response.on_hover_text(row_tooltip(entry, input.utc_offset_secs));
}

/// When the row happened: the full stamp (when there is room) over how long ago, right-aligned in
/// a column `width` wide.
fn time_column(
    ui: &mut Ui,
    entry: &McpAuditEntry,
    input: &ActivityInput,
    width: f32,
    full_stamp: bool,
) {
    let line = |ui: &Ui, text: String, color: Color32| {
        kit::galley(
            ui,
            &text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            color,
            width,
        )
    };
    let mut lines = Vec::new();
    if full_stamp {
        lines.push((
            line(
                ui,
                stamp(entry.at_ms, input.utc_offset_secs),
                theme::text_secondary(),
            ),
            theme::text_secondary(),
        ));
    }
    lines.push((
        line(
            ui,
            relative_time(input.now_ms, entry.at_ms),
            theme::text_tertiary(),
        ),
        theme::text_tertiary(),
    ));
    let height = lines.iter().map(|(galley, _)| galley.size().y).sum::<f32>()
        + 2.0 * (lines.len() - 1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut y = rect.top();
    for (galley, color) in lines {
        let size = galley.size();
        ui.painter()
            .galley(egui::pos2(rect.right() - size.x, y), galley, color);
        y += size.y + 2.0;
    }
}

/// A gray, underlined text button, as a link is drawn in this app.
fn quiet_link(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Label::new(
            kit::text(
                label,
                kit::size::SUBHEADLINE + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            )
            .underline(),
        )
        .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn caption(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            color,
        ))
        .wrap(),
    );
}

// MARK: Page preview

/// "Recent activity" on the Settings page: the newest few rows and the way into the sheet.
pub fn preview(ui: &mut Ui, state: &mut ActivityState, input: &ActivityInput) {
    section(
        ui,
        "Recent activity",
        Some(
            "What connected clients asked DegenView to do, newest first. Kept on this Mac only, and holds ids and results, never what a request contained.",
        ),
        |ui| {
            if input.entries.is_empty() {
                card(ui, |ui| {
                    ui.add_space(2.0);
                    caption(
                        ui,
                        "Nothing yet. Calls from connected clients appear here.",
                        theme::text_secondary(),
                    );
                    ui.add_space(2.0);
                });
                return;
            }
            card(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                for (index, entry) in input.entries.iter().take(PREVIEW_ROWS).enumerate() {
                    if index > 0 {
                        divider(ui);
                    }
                    row(ui, entry, input, false);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.set_min_height(28.0);
                caption(
                    ui,
                    if input.entries.len() > PREVIEW_ROWS {
                        format!(
                            "Showing the newest {PREVIEW_ROWS} of {}",
                            input.entries.len()
                        )
                    } else {
                        format!("{} in all", input.entries.len())
                    },
                    theme::text_secondary(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "View All Activity…", false, true).clicked() {
                        state.open();
                    }
                });
            });
        },
    );
}

// MARK: Sheet

/// The activity sheet, while it is open. Returns [`McpAction::ClearActivity`] when the user has
/// confirmed clearing the log.
pub fn sheet(
    ctx: &egui::Context,
    state: &mut ActivityState,
    input: &ActivityInput,
    actions: &mut Vec<McpAction>,
) {
    if !state.open {
        return;
    }
    let width =
        (ctx.content_rect().width() - 2.0 * SHEET_MARGIN).clamp(SHEET_MIN_WIDTH, SHEET_MAX_WIDTH);
    let list_height = (ctx.content_rect().height() - 400.0).clamp(150.0, 520.0);
    let mut close = false;
    let mut ask_to_clear = false;
    let mut cancel_clear = false;
    let mut clear = false;
    let mut page_move: i32 = 0;

    sheet::show(ctx, Id::new("mcp-activity"), width, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::List,
                "Activity log",
                "What connected clients asked DegenView to do. Kept on this Mac only; it holds ids and results, never what a request contained.",
                theme::text_secondary(),
            );
        });
        ui.add_space(16.0);
        sheet::inset(ui, |ui| {
            let clients = clients_in(input.entries);
            // A client whose rows are all gone (the log was cleared) cannot stay selected.
            if state
                .client_id
                .as_ref()
                .is_some_and(|id| clients.iter().all(|client| client.id != *id))
            {
                state.client_id = None;
            }
            let before = state.filter(input.utc_offset_secs);
            controls(ui, state, input, &clients);
            if state.filter(input.utc_offset_secs) != before {
                // A different question has a different number of pages.
                state.page = 0;
            }
        });
        ui.add_space(12.0);

        let view = View::of(state, input);
        sheet::inset(ui, |ui| {
            summary(ui, state, input, &view);
            ui.add_space(8.0);
            list(
                ui,
                state,
                input,
                &view,
                width - 2.0 * f32::from(sheet::INSET),
                list_height,
            );
            if view.paging.pages > 1 {
                ui.add_space(10.0);
                page_move = pager(ui, &view.paging);
            }
        });

        ui.add_space(16.0);
        let confirming = state.confirm_clear;
        let can_clear = !input.entries.is_empty();
        sheet::footer_split(
            ui,
            |ui| {
                if confirming {
                    ui.label(kit::text(
                        "Clear the whole log?",
                        kit::size::BODY,
                        kit::MEDIUM,
                        theme::text_primary(),
                    ));
                } else {
                    ui.add_enabled_ui(can_clear, |ui| {
                        if kit::destructive_sheet_button(ui, Icon::Trash, "Clear…").clicked() {
                            ask_to_clear = true;
                        }
                    });
                }
            },
            |ui| {
                if confirming {
                    if kit::destructive_sheet_button(ui, Icon::Trash, "Clear Log").clicked() {
                        clear = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        cancel_clear = true;
                    }
                } else if kit::sheet_button(ui, "Done", true, true).clicked() {
                    close = true;
                }
            },
        );
    });

    if ask_to_clear {
        state.confirm_clear = true;
    }
    if cancel_clear {
        state.confirm_clear = false;
    }
    if clear {
        actions.push(McpAction::ClearActivity);
        state.confirm_clear = false;
    }

    // PageUp / PageDown turn the page, unless a menu or calendar is open over it.
    if page_move == 0 && !popup_open(ctx) {
        ctx.input(|input| {
            if input.key_pressed(egui::Key::PageDown) {
                page_move = 1;
            } else if input.key_pressed(egui::Key::PageUp) {
                page_move = -1;
            }
        });
    }
    if page_move != 0 {
        state.page = state.page.saturating_add_signed(page_move as isize);
    }

    // Escape backs out of the clear question first, then of the sheet.
    if !popup_open(ctx) && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        if state.confirm_clear {
            state.confirm_clear = false;
        } else {
            close = true;
        }
    }
    if close {
        state.close(ctx);
    }
}

fn popup_open(ctx: &egui::Context) -> bool {
    egui::Popup::is_any_open(ctx)
        || date_picker::is_open(ctx, from_id())
        || date_picker::is_open(ctx, to_id())
}

// MARK: Controls

/// The search field and the three filters, and the custom range's two ends.
fn controls(
    ui: &mut Ui,
    state: &mut ActivityState,
    input: &ActivityInput,
    clients: &[McpActivityClient],
) {
    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
    kit::search_field(
        ui,
        Id::new("mcp-activity-search"),
        &mut state.search,
        "Search tool, id or error code",
        ui.available_width(),
    );

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        client_picker(ui, state, clients);
        status_picker(ui, state);
        range_picker(ui, state, input);
    });

    if state.range == RangeChoice::Custom {
        custom_range(ui, state, input);
    }
}

fn client_picker(ui: &mut Ui, state: &mut ActivityState, clients: &[McpActivityClient]) {
    let mut labels = vec!["All clients".to_owned()];
    labels.extend(clients.iter().map(|client| client.label.clone()));
    let selected = state
        .client_id
        .as_ref()
        .and_then(|id| clients.iter().position(|client| client.id == *id))
        .map_or(0, |index| index + 1);
    if let Some(choice) = kit::menu_picker(ui, CLIENT_PICKER_WIDTH, &labels, selected, true) {
        state.client_id = choice
            .checked_sub(1)
            .and_then(|index| clients.get(index))
            .map(|client| client.id.clone());
    }
}

fn status_picker(ui: &mut Ui, state: &mut ActivityState) {
    let mut labels = vec!["All outcomes".to_owned()];
    labels.extend(McpActivityStatus::ALL.map(|status| status.label().to_owned()));
    let selected = state.status.map_or(0, |status| {
        McpActivityStatus::ALL
            .iter()
            .position(|candidate| *candidate == status)
            .map_or(0, |index| index + 1)
    });
    if let Some(choice) = kit::menu_picker(ui, STATUS_PICKER_WIDTH, &labels, selected, true) {
        state.status = choice
            .checked_sub(1)
            .and_then(|index| McpActivityStatus::ALL.get(index).copied());
    }
}

fn range_picker(ui: &mut Ui, state: &mut ActivityState, input: &ActivityInput) {
    let mut labels: Vec<String> = McpActivityRange::PRESETS
        .iter()
        .map(|range| range.label().to_owned())
        .collect();
    let custom_index = labels.len();
    labels.push(
        McpActivityRange::Custom {
            from_ms: None,
            to_ms: None,
        }
        .label()
        .to_owned(),
    );
    let selected = match state.range {
        RangeChoice::Any => 0,
        RangeChoice::Preset(range) => McpActivityRange::PRESETS
            .iter()
            .position(|preset| *preset == range)
            .unwrap_or(0),
        RangeChoice::Custom => custom_index,
    };
    let Some(choice) = kit::menu_picker(ui, RANGE_PICKER_WIDTH, &labels, selected, true) else {
        return;
    };
    if choice == custom_index {
        if state.range != RangeChoice::Custom {
            state.range = RangeChoice::Custom;
            state.from_text = date_text(input.now_ms - DEFAULT_FROM_AGO_MS, input.utc_offset_secs);
            state.to_text.clear();
        }
    } else {
        state.range = match McpActivityRange::PRESETS[choice] {
            McpActivityRange::Any => RangeChoice::Any,
            preset => RangeChoice::Preset(preset),
        };
        date_picker::close(ui.ctx(), from_id());
        date_picker::close(ui.ctx(), to_id());
    }
}

fn custom_range(ui: &mut Ui, state: &mut ActivityState, input: &ActivityInput) {
    let inverted = state.filter(input.utc_offset_secs).range.is_inverted();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        let default_to = input.now_ms;
        bound(
            ui,
            "From",
            from_id(),
            &mut state.from_text,
            input.now_ms - DEFAULT_FROM_AGO_MS,
            input.utc_offset_secs,
            inverted,
        );
        bound(
            ui,
            "To",
            to_id(),
            &mut state.to_text,
            default_to,
            input.utc_offset_secs,
            inverted,
        );
    });
    if inverted {
        caption(
            ui,
            "The start is after the end, so nothing matches.",
            kit::ORANGE,
        );
    }
}

/// One end of the custom range: the date picker with a button that removes the limit, or "No
/// limit" until the user sets one.
fn bound(
    ui: &mut Ui,
    label: &str,
    id: Id,
    text: &mut String,
    default_ms: i64,
    utc_offset_secs: i64,
    error: bool,
) {
    ui.label(kit::text(
        label,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
    ));
    if text.is_empty() {
        let (rect, response) = ui.allocate_exact_size(vec2(DATE_FIELD_WIDTH, 24.0), Sense::click());
        let response =
            crate::ui::accessibility::label(response, format!("{label}: no limit. Set one"))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        let frame = kit::field_frame(false);
        ui.painter().rect_filled(rect, 6.0, frame.fill);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, frame.stroke.color),
            StrokeKind::Inside,
        );
        let galley = kit::galley(
            ui,
            "No limit",
            12.0,
            kit::REGULAR,
            theme::text_tertiary(),
            f32::INFINITY,
        );
        ui.painter().galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
        if response.clicked() {
            *text = date_text(default_ms, utc_offset_secs);
        }
    } else {
        date_picker::show(
            ui,
            id,
            text,
            utc_offset_secs,
            DATE_FIELD_WIDTH,
            date_picker::Look::Compact,
            error,
        );
        if icons::button(
            ui,
            Icon::Xmark,
            24.0,
            theme::text_secondary(),
            "Remove this limit",
        )
        .clicked()
        {
            text.clear();
            date_picker::close(ui.ctx(), id);
        }
    }
}

// MARK: Results

/// "163 entries", or how many of them match, and the way back to all of them.
fn summary(ui: &mut Ui, state: &mut ActivityState, input: &ActivityInput, view: &View) {
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        let all = input.entries.len();
        let mut text = if view.filter.is_active() {
            format!("{} of {} {}", view.matching.len(), all, noun(all))
        } else {
            format!("{all} {}", noun(all))
        };
        if input.unreadable > 0 {
            text.push_str(&format!(" · {} could not be read", input.unreadable));
        }
        caption(ui, text, theme::text_secondary());
        if view.filter.is_active() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if quiet_link(ui, "Clear filters").clicked() {
                    state.clear_filters(ui.ctx());
                    state.page = 0;
                }
            });
        }
    });
}

fn noun(count: usize) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

/// The page of rows, or the reason there are none.
fn list(
    ui: &mut Ui,
    state: &mut ActivityState,
    input: &ActivityInput,
    view: &View,
    content_width: f32,
    max_height: f32,
) {
    if view.matching.is_empty() {
        card(ui, |ui| {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                if input.entries.is_empty() {
                    caption(
                        ui,
                        "Nothing yet. Calls from connected clients appear here.",
                        theme::text_secondary(),
                    );
                } else {
                    caption(
                        ui,
                        "No entries match these filters.",
                        theme::text_secondary(),
                    );
                    ui.add_space(4.0);
                    if quiet_link(ui, "Clear filters").clicked() {
                        state.clear_filters(ui.ctx());
                        state.page = 0;
                    }
                }
            });
            ui.add_space(14.0);
        });
        return;
    }
    let full_stamp = content_width >= WIDE_LIST;
    egui::ScrollArea::vertical()
        .id_salt(("mcp-activity-rows", view.paging.page))
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            card(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                for (index, entry) in view.page_rows().iter().enumerate() {
                    if index > 0 {
                        divider(ui);
                    }
                    row(ui, entry, input, full_stamp);
                }
            });
        });
}

/// "26–50 of 163" and the previous/next buttons; returns how far the user asked to move.
fn pager(ui: &mut Ui, paging: &Paging) -> i32 {
    let mut step = 0;
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        caption(ui, paging.range_text(), theme::text_secondary());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_enabled_ui(paging.has_next(), |ui| {
                if icons::button(
                    ui,
                    Icon::ChevronRight,
                    24.0,
                    theme::text_primary(),
                    "Next page (Page Down)",
                )
                .clicked()
                {
                    step = 1;
                }
            });
            ui.label(kit::text(
                format!("Page {} of {}", paging.page + 1, paging.pages),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.add_enabled_ui(paging.has_previous(), |ui| {
                if icons::button(
                    ui,
                    Icon::ChevronLeft,
                    24.0,
                    theme::text_primary(),
                    "Previous page (Page Up)",
                )
                .clicked()
                {
                    step = -1;
                }
            });
        });
    });
    step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::{McpAuditOutcome, McpAuditTarget};
    use eframe::egui::{Event, Key, Pos2, Rect, Shape};

    const NOW: i64 = 10 * 86_400_000;

    fn call(n: u64, client: &str, outcome: McpAuditOutcome) -> McpAuditEntry {
        McpAuditEntry {
            id: n,
            at_ms: NOW - 1_000 * n as i64,
            client_id: client.into(),
            client_name: format!("Client {client}"),
            tool: "degenview_get_pine_report".into(),
            outcome,
            code: if outcome == McpAuditOutcome::Failed {
                "NOT_FOUND".into()
            } else {
                String::new()
            },
            duration_ms: 312,
            targets: vec![
                McpAuditTarget {
                    kind: "workspace_id".into(),
                    id: "46".into(),
                },
                McpAuditTarget {
                    kind: "chart_id".into(),
                    id: "3".into(),
                },
            ],
            count: 1,
            ..McpAuditEntry::default()
        }
    }

    /// `count` rows, newest first, alternating between clients "a" and "b" and cycling outcomes.
    fn log(count: usize) -> Vec<McpAuditEntry> {
        (1..=count as u64)
            .map(|n| {
                let outcome = [
                    McpAuditOutcome::Succeeded,
                    McpAuditOutcome::Failed,
                    McpAuditOutcome::Denied,
                    McpAuditOutcome::Cancelled,
                ][n as usize % 4];
                call(n, if n % 2 == 0 { "a" } else { "b" }, outcome)
            })
            .collect()
    }

    fn input(entries: &[McpAuditEntry]) -> ActivityInput<'_> {
        ActivityInput {
            entries,
            unreadable: 0,
            now_ms: NOW,
            utc_offset_secs: 0,
        }
    }

    // MARK: Rows

    #[test]
    fn rows_say_what_who_and_how_it_ended() {
        assert_eq!(tool_title("degenview_get_pine_report"), "Get pine report");
        assert_eq!(tool_title("custom_thing"), "Custom thing");
        assert_eq!(tool_title(""), "");
        assert_eq!(target_text("chart_id", "3"), "chart 3");
        assert_eq!(target_text("order_id", "9"), "order 9");

        let done = call(1, "a", McpAuditOutcome::Succeeded);
        assert_eq!(outcome_tag(&done).0, "Done");
        assert_eq!(row_title(&done), "Get pine report");
        assert_eq!(
            row_detail(&done),
            "Client a · workspace 46, chart 3 · 312 ms"
        );
        let failed = call(2, "a", McpAuditOutcome::Failed);
        assert_eq!(outcome_tag(&failed).0, "Failed");
        assert!(row_detail(&failed).contains("NOT_FOUND"));
        for (outcome, tag) in [
            (McpAuditOutcome::Denied, "Denied"),
            (McpAuditOutcome::Cancelled, "Cancelled"),
            (McpAuditOutcome::Held, "Held"),
            (McpAuditOutcome::Approved, "Approved"),
            (McpAuditOutcome::Declined, "Declined"),
            (McpAuditOutcome::Expired, "Expired"),
        ] {
            assert_eq!(outcome_tag(&call(3, "a", outcome)).0, tag);
        }

        let limited = McpAuditEntry {
            kind: McpAuditKind::RateLimited,
            client_name: "Claude Desktop".into(),
            count: 500,
            ..McpAuditEntry::default()
        };
        assert_eq!(outcome_tag(&limited).0, "Limited");
        assert_eq!(
            row_detail(&limited),
            "Claude Desktop · 500 calls refused within a minute"
        );
        let cleared = McpAuditEntry {
            kind: McpAuditKind::Cleared,
            ..McpAuditEntry::default()
        };
        assert_eq!(outcome_tag(&cleared).0, "Cleared");
        assert_eq!(row_title(&cleared), "Activity log cleared");
        let gap = McpAuditEntry {
            kind: McpAuditKind::Gap,
            count: 1,
            ..McpAuditEntry::default()
        };
        assert_eq!(outcome_tag(&gap).0, "Gap");
        assert_eq!(row_detail(&gap), "1 row dropped while the app was busy");
    }

    #[test]
    fn a_rows_tooltip_holds_everything_the_row_does() {
        let mut entry = call(7, "abc", McpAuditOutcome::Failed);
        entry.request_id = "req-9".into();
        let tip = row_tooltip(&entry, 0);
        for part in [
            "Client: Client abc (abc)",
            "Tool: degenview_get_pine_report",
            "Result: Failed (NOT_FOUND)",
            "Took 312 ms",
            "Named: workspace 46, chart 3",
            "Request id: req-9",
            "Entry id: 7",
        ] {
            assert!(tip.contains(part), "{part} missing from {tip}");
        }
        // The time is in the viewer's zone, to the second.
        assert_eq!(stamp(0, 0), "Jan 1, 1970 00:00:00");
        assert_eq!(stamp(1_000, 2 * 3_600), "Jan 1, 1970 02:00:01");
    }

    // MARK: The view

    #[test]
    fn the_view_narrows_and_pages_the_log() {
        let entries = log(60);
        let mut state = ActivityState::default();
        state.open();
        let view = View::of(&mut state, &input(&entries));
        assert_eq!((view.matching.len(), view.paging.pages), (60, 3));
        assert_eq!(view.page_rows().len(), ACTIVITY_PAGE_SIZE);
        assert_eq!(view.page_rows()[0].id, 1, "newest first");

        state.client_id = Some("a".into());
        state.status = Some(McpActivityStatus::Failed);
        state.page = 5;
        let view = View::of(&mut state, &input(&entries));
        // Even ids are client a, and id % 4 == 1 is Failed: no even id is, so nothing matches.
        assert!(view.matching.is_empty());
        assert_eq!(state.page, 0, "a page past the end is held to the last one");

        state.client_id = Some("b".into());
        let view = View::of(&mut state, &input(&entries));
        assert!(view.matching.iter().all(|row| row.client_id == "b"));
        assert_eq!(view.matching.len(), 15);
        assert_eq!(view.paging.pages, 1);
    }

    #[test]
    fn a_custom_range_reads_its_ends_in_the_viewers_zone() {
        let entries = log(5);
        // Rows are 1..5 seconds before NOW (a UTC midnight). In a zone 2 hours east, "00:00 on
        // the 11th" is 22:00 UTC the 10th, two hours before NOW: the rows are all later.
        let mut state = ActivityState {
            range: RangeChoice::Custom,
            from_text: "1970-01-11 00:00".into(),
            ..ActivityState::default()
        };
        let mut east = input(&entries);
        east.utc_offset_secs = 2 * 3_600;
        let view = View::of(&mut state, &east);
        assert_eq!(view.matching.len(), 5);
        let utc = input(&entries);
        assert!(
            View::of(&mut state, &utc).matching.is_empty(),
            "UTC midnight is after them"
        );
        // An empty end is no limit; a bad one is none either.
        state.from_text.clear();
        state.to_text = "not a date".into();
        assert_eq!(View::of(&mut state, &utc).matching.len(), 5);
    }

    #[test]
    fn a_client_with_no_rows_left_cannot_stay_selected() {
        let mut state = ActivityState::default();
        state.open();
        state.client_id = Some("gone".into());
        let ctx = egui::Context::default();
        let entries = log(3);
        drive(&ctx, &mut state, &input(&entries), Vec::new(), 0.0);
        assert_eq!(state.client_id, None);
    }

    // MARK: Driving the sheet

    /// Every text a frame painted, with where.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
        fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                // A right-aligned galley's rectangle reaches left of its anchor.
                Shape::Text(text) => out.push((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes
            .iter()
            .for_each(|clipped| walk(&clipped.shape, &mut out));
        out
    }

    struct Frame {
        actions: Vec<McpAction>,
        texts: Vec<(String, Rect)>,
    }

    fn drive(
        ctx: &egui::Context,
        state: &mut ActivityState,
        input: &ActivityInput,
        events: Vec<Event>,
        time: f64,
    ) -> Frame {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| sheet(ui.ctx(), state, input, &mut actions));
        output.textures_delta.clear();
        Frame {
            actions,
            texts: texts(&output.shapes),
        }
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A little session against one context: settled frames, key presses and clicks on text.
    struct Session<'a> {
        ctx: egui::Context,
        state: ActivityState,
        input: ActivityInput<'a>,
        time: f64,
        actions: Vec<McpAction>,
        last: Vec<(String, Rect)>,
    }

    impl<'a> Session<'a> {
        fn new(input: ActivityInput<'a>) -> Self {
            let mut state = ActivityState::default();
            state.open();
            let mut session = Self {
                ctx: egui::Context::default(),
                state,
                input,
                time: 0.0,
                actions: Vec::new(),
                last: Vec::new(),
            };
            // A modal places itself by the size it had last frame, and a scroll area learns its
            // content's over a few: let both settle.
            for _ in 0..8 {
                session.step(Vec::new());
            }
            session
        }

        fn step(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let frame = drive(&self.ctx, &mut self.state, &self.input, events, self.time);
            self.actions.extend(frame.actions);
            self.last = frame.texts;
        }

        fn press(&mut self, which: Key) {
            self.step(vec![key(which, true)]);
            self.step(vec![key(which, false)]);
        }

        fn centre_of(&self, text: &str) -> Pos2 {
            // The last one: a menu or alert is painted over the sheet, and its rows may repeat
            // words the list already shows ("Failed").
            self.last
                .iter()
                .rfind(|(shown, _)| shown == text)
                .unwrap_or_else(|| {
                    panic!(
                        "{text:?} is not on screen: {:?}",
                        self.last.iter().map(|t| &t.0).collect::<Vec<_>>()
                    )
                })
                .1
                .center()
        }

        fn shows(&self, text: &str) -> bool {
            self.last.iter().any(|(shown, _)| shown == text)
        }

        fn click_on(&mut self, text: &str) {
            // The sheet recentres itself for a frame or two when its height changes.
            for _ in 0..4 {
                self.step(Vec::new());
            }
            let at = self.centre_of(text);
            self.step(vec![Event::PointerMoved(at)]);
            self.step(vec![Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.step(vec![Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.step(Vec::new());
        }
    }

    #[test]
    fn the_sheet_lays_out_empty_short_and_full_at_every_width_and_filter() {
        let rows = {
            let mut rows = log(1000);
            // The longest things a row can hold must truncate, not widen the sheet.
            rows[7].client_name = "A client with a very long name ".repeat(4);
            rows[7].tool = "degenview_".to_owned() + &"long_tool_name_".repeat(4);
            rows
        };
        let filters: [fn(&mut ActivityState); 5] = [
            |_| {},
            |state| state.status = Some(McpActivityStatus::Failed),
            |state| state.client_id = Some("a".into()),
            |state| {
                state.range = RangeChoice::Custom;
                state.from_text = "1970-01-09 00:00".into();
                state.to_text = "1970-01-01 00:00".into();
            },
            |state| state.search = "nothing matches this".into(),
        ];
        for width in [320.0, 360.0, 560.0, 900.0] {
            for shown in [0, 1, 30, 1000] {
                for apply in filters {
                    let ctx = egui::Context::default();
                    let mut state = ActivityState::default();
                    state.open();
                    apply(&mut state);
                    let entries = &rows[..shown];
                    let input = input(entries);
                    for step in 0..3 {
                        let raw = egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0))),
                            time: Some(f64::from(step) * 0.1),
                            ..Default::default()
                        };
                        let mut actions = Vec::new();
                        let mut output =
                            ctx.run_ui(raw, |ui| sheet(ui.ctx(), &mut state, &input, &mut actions));
                        output.textures_delta.clear();
                        assert!(actions.is_empty(), "{shown} rows at {width}");
                    }
                    assert!(state.open, "laying out never closes it");
                }
            }
        }
    }

    #[test]
    fn the_preview_shows_the_newest_few_and_opens_the_sheet() {
        let entries = log(40);
        let ctx = egui::Context::default();
        let mut state = ActivityState::default();
        let input = input(&entries);
        let mut painted = Vec::new();
        for step in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
                time: Some(f64::from(step) * 0.1),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| preview(ui, &mut state, &input));
            output.textures_delta.clear();
            painted = texts(&output.shapes);
        }
        assert!(!state.open, "drawing the page never opens the sheet");
        let shown = |text: &str| painted.iter().any(|(t, _)| t == text);
        assert!(shown("Showing the newest 5 of 40"));
        assert!(shown("View All Activity…"));
        assert_eq!(
            painted
                .iter()
                .filter(|(t, _)| t == "Get pine report")
                .count(),
            PREVIEW_ROWS
        );
        let empty = &[];
        let input = ActivityInput {
            entries: empty,
            ..input
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
                ..Default::default()
            },
            |ui| preview(ui, &mut state, &input),
        );
        output.textures_delta.clear();
        assert!(
            texts(&output.shapes)
                .iter()
                .any(|(t, _)| t.starts_with("Nothing yet"))
        );
    }

    #[test]
    fn nothing_is_painted_outside_the_sheet() {
        // Screen 900 wide, so the 760-wide sheet spans 70 to 830.
        let mut entries = log(30);
        entries[2].tool = "degenview_".to_owned() + &"long_tool_name_".repeat(6);
        entries[2].client_name = "A client with a very long name ".repeat(5);
        let session = Session::new(input(&entries));
        assert!(session.last.len() > 50);
        for (text, rect) in &session.last {
            assert!(
                rect.left() >= 70.0 - 0.5 && rect.right() <= 830.0 + 0.5,
                "{text:?} is painted at {rect:?}"
            );
        }
    }

    #[test]
    fn page_up_and_page_down_turn_pages_and_stop_at_the_ends() {
        let entries = log(60);
        let mut session = Session::new(input(&entries));
        assert_eq!(session.state.page, 0);
        assert!(session.shows("Page 1 of 3"));
        session.press(Key::PageUp);
        assert_eq!(session.state.page, 0, "no page before the first");
        session.press(Key::PageDown);
        assert_eq!(session.state.page, 1);
        assert!(session.shows("Page 2 of 3") && session.shows("26–50 of 60"));
        session.press(Key::PageDown);
        session.press(Key::PageDown);
        assert_eq!(session.state.page, 2, "no page after the last");
        assert!(session.shows("51–60 of 60"));
        session.press(Key::PageUp);
        assert_eq!(session.state.page, 1);
        assert!(session.actions.is_empty());
    }

    #[test]
    fn choosing_a_filter_goes_back_to_the_first_page_and_counts_the_matches() {
        let entries = log(100);
        let mut session = Session::new(input(&entries));
        session.press(Key::PageDown);
        session.press(Key::PageDown);
        assert_eq!(session.state.page, 2);
        assert!(session.shows("100 entries"));

        session.click_on("All outcomes");
        session.click_on("Failed");
        assert_eq!(session.state.status, Some(McpActivityStatus::Failed));
        assert_eq!(session.state.page, 0);
        assert!(session.shows("25 of 100 entries"), "{:?}", session.last);
        assert!(session.shows("Clear filters"));

        session.click_on("Clear filters");
        assert_eq!(session.state.status, None);
        assert!(session.shows("100 entries"));
    }

    #[test]
    fn choosing_a_client_lists_the_clients_in_the_log() {
        let entries = log(10);
        let mut session = Session::new(input(&entries));
        session.click_on("All clients");
        assert!(session.shows("Client a") && session.shows("Client b"));
        session.click_on("Client a");
        assert_eq!(session.state.client_id.as_deref(), Some("a"));
        assert!(session.shows("5 of 10 entries"));
    }

    #[test]
    fn the_custom_range_offers_two_ends_that_can_each_be_left_open() {
        let entries = log(10);
        let mut session = Session::new(input(&entries));
        session.click_on("Any time");
        session.click_on("Custom range…");
        assert_eq!(session.state.range, RangeChoice::Custom);
        assert_eq!(
            session.state.from_text, "1970-01-10 00:00",
            "a day before now"
        );
        assert!(session.state.to_text.is_empty());
        assert!(session.shows("No limit"), "the open end says so");
        // Setting the open end fills it in; removing it opens it again.
        session.click_on("No limit");
        assert_eq!(session.state.to_text, "1970-01-11 00:00");
    }

    #[test]
    fn clearing_the_log_asks_first_and_acts_only_on_the_destructive_button() {
        let entries = log(3);
        let mut session = Session::new(input(&entries));
        assert!(session.shows("Clear…") && session.shows("Done"));

        session.click_on("Clear…");
        assert!(session.state.confirm_clear && session.shows("Clear the whole log?"));
        assert!(session.actions.is_empty(), "asking changes nothing");

        // Cancel, Escape and a stray Return all leave it alone.
        session.click_on("Cancel");
        assert!(!session.state.confirm_clear && session.actions.is_empty());
        session.click_on("Clear…");
        session.press(Key::Enter);
        session.press(Key::Escape);
        assert!(!session.state.confirm_clear && session.actions.is_empty());
        assert!(
            session.state.open,
            "the first Escape only backs out of the question"
        );

        session.click_on("Clear…");
        session.click_on("Clear Log");
        assert_eq!(session.actions, [McpAction::ClearActivity]);
        assert!(!session.state.confirm_clear);
        assert!(
            session.state.open,
            "the sheet stays to show the emptied log"
        );
    }

    #[test]
    fn an_empty_log_has_nothing_to_clear() {
        let mut session = Session::new(input(&[]));
        assert!(session.shows("Nothing yet. Calls from connected clients appear here."));
        session.click_on("Clear…");
        assert!(
            !session.state.confirm_clear,
            "a disabled button does nothing"
        );
        assert!(session.actions.is_empty());
    }

    #[test]
    fn done_and_escape_close_the_sheet_and_forget_its_filters() {
        let entries = log(10);
        let mut session = Session::new(input(&entries));
        session.state.status = Some(McpActivityStatus::Failed);
        session.state.search = "x".into();
        session.click_on("Done");
        assert!(!session.state.open);
        assert_eq!(session.state.status, None);
        assert!(session.state.search.is_empty());

        let mut session = Session::new(input(&entries));
        session.state.client_id = Some("a".into());
        session.press(Key::Escape);
        assert!(!session.state.open);
        assert_eq!(session.state.client_id, None);
        assert!(session.actions.is_empty());
        // Opening it again starts from every row.
        session.state.open();
        assert!(!session.state.filter(0).is_active());
    }

    #[test]
    fn an_unreadable_row_is_counted_not_hidden_silently() {
        let entries = log(3);
        let mut with_unreadable = input(&entries);
        with_unreadable.unreadable = 2;
        let session = Session::new(with_unreadable);
        assert!(session.shows("3 entries · 2 could not be read"));
    }
}
