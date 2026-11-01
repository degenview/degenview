//! The sheets that make and manage portfolios: "New Portfolio" (a name and a base
//! currency) and "Manage Portfolios" (rename, reorder, duplicate, delete). Pure views over
//! plain values; the host owns the ledger and applies the actions.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{
    MAX_NAME_LENGTH, Portfolio, PortfolioCurrency, is_duplicate_name, is_valid_name, normalize_name,
};
use crate::model::portfolio_format::PORTFOLIO_CURRENCIES;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Sense, Vec2};

// MARK: New portfolio

#[derive(Clone, Debug, PartialEq)]
pub enum CreateOutcome {
    Create {
        name: String,
        currency: PortfolioCurrency,
    },
    Cancel,
}

pub struct CreateState {
    pub name: String,
    pub currency: PortfolioCurrency,
    focus: bool,
}

impl Default for CreateState {
    fn default() -> Self {
        Self {
            name: String::new(),
            currency: PortfolioCurrency::Usd,
            focus: true,
        }
    }
}

impl CreateState {
    pub fn is_valid(&self) -> bool {
        is_valid_name(&self.name)
    }

    pub fn is_duplicate(&self, existing: &[String]) -> bool {
        is_duplicate_name(&self.name, existing)
    }

    /// The name is cut to the limit as it is typed.
    pub fn clamp_name(&mut self) {
        if self.name.chars().count() > MAX_NAME_LENGTH {
            self.name = self.name.chars().take(MAX_NAME_LENGTH).collect();
        }
    }

    pub fn outcome(&self) -> Option<CreateOutcome> {
        self.is_valid().then(|| CreateOutcome::Create {
            name: normalize_name(&self.name),
            currency: self.currency,
        })
    }
}

/// Space between a section's label and its control.
const LABEL_GAP: f32 = 8.0;
/// Gap between the currency tiles, across and down.
const TILE_GAP: f32 = 8.0;
const TILE_HEIGHT: f32 = 48.0;
/// The line under the name field is always this tall, so the sheet does not resize when the
/// duplicate-name warning comes and goes.
const STATUS_HEIGHT: f32 = 16.0;

fn section_label(ui: &mut egui::Ui, title: &str) {
    ui.label(kit::text(
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        theme::text_primary(),
    ));
    ui.add_space(LABEL_GAP);
}

pub fn show_create(
    ctx: &egui::Context,
    state: &mut CreateState,
    existing_names: &[String],
) -> Option<CreateOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("portfolio-create"), 460.0, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Briefcase,
                "New Portfolio",
                "Group transactions and track them together.",
                theme::ACCENT,
            );
            ui.add_space(24.0);
            section_label(ui, "Name");
            let id = Id::new("portfolio-create-name");
            let focused = ui.memory(|memory| memory.has_focus(id));
            let response = kit::field_frame(focused)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        icons::inline(ui, Icon::Briefcase, 13.0, theme::text_secondary());
                        let width = ui.available_width();
                        kit::plain_field(
                            ui,
                            id,
                            &mut state.name,
                            "e.g. Long-term holds",
                            kit::size::BODY,
                            kit::REGULAR,
                            width,
                        )
                    })
                    .inner
                })
                .inner;
            if std::mem::take(&mut state.focus) {
                response.request_focus();
            }
            state.clamp_name();
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                outcome = state.outcome();
            }
            ui.add_space(6.0);
            let (status, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), STATUS_HEIGHT),
                Sense::hover(),
            );
            if state.is_duplicate(existing_names) {
                // The system orange is too pale to read as small text on a light sheet.
                let warning = theme::pick(kit::ORANGE, Color32::from_rgb(190, 100, 0));
                let message = kit::galley(
                    ui,
                    "A portfolio with this name already exists.",
                    kit::size::CAPTION + 1.0,
                    kit::REGULAR,
                    warning,
                    status.width() - 16.0,
                );
                icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        status.left_center() + Vec2::new(5.5, 0.0),
                        Vec2::splat(11.0),
                    ),
                    Icon::TriangleFill,
                    warning,
                );
                ui.painter().galley(
                    egui::pos2(
                        status.left() + 16.0,
                        status.center().y - message.size().y / 2.0,
                    ),
                    message,
                    Color32::PLACEHOLDER,
                );
            }
            ui.add_space(6.0);
            section_label(ui, "Base currency");
            currency_grid(ui, &mut state.currency);
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(kit::text(
                    format!(
                        "Transactions in this portfolio are valued in {}. You can still view it in any currency later.",
                        state.currency.code()
                    ),
                    kit::size::CAPTION + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
            ui.add_space(24.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Create Portfolio", true, state.is_valid()).clicked() {
                outcome = state.outcome();
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(CreateOutcome::Cancel);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(CreateOutcome::Cancel);
    }
    outcome
}

/// Three columns of selectable currency tiles: glyph, code and name. The tiles are one width
/// and [`TILE_GAP`] apart in both directions, and each is a button: focusable, labelled for
/// screen readers, with a ring while it has keyboard focus.
fn currency_grid(ui: &mut egui::Ui, selected: &mut PortfolioCurrency) {
    let columns = 3;
    let tile_width = (ui.available_width() - TILE_GAP * (columns - 1) as f32) / columns as f32;
    for (row_index, row) in PORTFOLIO_CURRENCIES.chunks(columns).enumerate() {
        if row_index > 0 {
            ui.add_space(TILE_GAP);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for currency in row {
                if currency_tile(ui, *currency, *selected == *currency, tile_width).clicked() {
                    *selected = *currency;
                }
            }
        });
    }
}

fn currency_tile(
    ui: &mut egui::Ui,
    currency: PortfolioCurrency,
    is_selected: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, TILE_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(
        response,
        format!("{}, {}", currency.code(), currency.display_name()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand);
    let tint = if is_selected {
        theme::ACCENT
    } else {
        theme::text_secondary()
    };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        10.0,
        if is_selected {
            theme::ACCENT.gamma_multiply(0.10)
        } else if response.is_pointer_button_down_on() {
            theme::ink(24)
        } else if response.hovered() {
            theme::ink(18)
        } else {
            theme::ink(8)
        },
    );
    painter.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(
            if is_selected { 1.5 } else { 1.0 },
            if is_selected {
                theme::ACCENT
            } else {
                theme::border()
            },
        ),
        egui::StrokeKind::Inside,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            12.0,
            egui::Stroke::new(2.0, theme::with_alpha(theme::text_secondary(), 0.5)),
            egui::StrokeKind::Outside,
        );
    }
    let badge = rect.left_center() + Vec2::new(12.0 + 14.0, 0.0);
    painter.circle_filled(badge, 14.0, tint.gamma_multiply(0.18));
    painter.text(
        badge,
        Align2::CENTER_CENTER,
        currency.glyph(),
        egui::FontId::proportional(14.0),
        tint,
    );
    let text_left = rect.left() + 12.0 + 28.0 + 10.0;
    let text_width = (rect.right() - 8.0 - text_left).max(0.0);
    let code = kit::galley(
        ui,
        currency.code(),
        kit::size::BODY,
        kit::SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        currency.display_name(),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        text_width,
    );
    let block = code.size().y + 1.0 + name.size().y;
    let top = rect.center().y - block / 2.0;
    let name_top = top + code.size().y + 1.0;
    ui.painter()
        .galley(egui::pos2(text_left, top), code, Color32::PLACEHOLDER);
    ui.painter()
        .galley(egui::pos2(text_left, name_top), name, Color32::PLACEHOLDER);
    response
}

// MARK: Manage portfolios

#[derive(Clone, Debug, PartialEq)]
pub enum ManageAction {
    Rename {
        id: u64,
        name: String,
    },
    Duplicate(u64),
    Delete(u64),
    /// Same meaning as `PortfolioLedger::reorder`: move `from` to sit before `destination`.
    Reorder {
        from: Vec<usize>,
        destination: usize,
    },
    /// "New Portfolio" was pressed; the host opens the create sheet.
    New,
    Done,
}

/// One portfolio's summary line input.
pub struct ManageRow<'a> {
    pub portfolio: &'a Portfolio,
    pub transactions: usize,
    pub assets: usize,
}

pub struct ManageInput<'a> {
    pub rows: &'a [ManageRow<'a>],
    pub current: Option<u64>,
}

#[derive(Default)]
pub struct ManageState {
    /// Name edits in progress, by portfolio id.
    drafts: std::collections::HashMap<u64, String>,
    pending_delete: Option<u64>,
    dragging: Option<usize>,
}

/// "USD · 3 transactions · 2 assets".
pub fn summary_line(currency: PortfolioCurrency, transactions: usize, assets: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} · {} · {}",
        currency.code(),
        plural(transactions, "transaction", "transactions"),
        plural(assets, "asset", "assets")
    )
}

/// The reorder that moves the item at `index` one place up, or `None` at the top.
pub fn move_up(index: usize) -> Option<(Vec<usize>, usize)> {
    (index > 0).then(|| (vec![index], index - 1))
}

/// The reorder that moves the item at `index` one place down, or `None` at the bottom. The
/// destination is an insertion point in the list as it is now, so it is two past the item.
pub fn move_down(index: usize, len: usize) -> Option<(Vec<usize>, usize)> {
    (index + 1 < len).then(|| (vec![index], index + 2))
}

/// A renamed portfolio's new name, or `None` when the draft is blank (put the old one back)
/// or unchanged.
pub fn rename_commit(draft: &str, current: &str) -> Option<String> {
    let cleaned = normalize_name(draft);
    (!cleaned.is_empty() && cleaned != current).then_some(cleaned)
}

pub fn delete_message(transactions: usize) -> String {
    format!(
        "This permanently deletes the portfolio and its {transactions} {}. It can't be undone.",
        if transactions == 1 {
            "transaction"
        } else {
            "transactions"
        }
    )
}

pub fn show_manage(
    ctx: &egui::Context,
    state: &mut ManageState,
    input: &ManageInput,
) -> Vec<ManageAction> {
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("portfolio-manage"), 560.0, |ui| {
        ui.set_height(520.0);
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Layers,
                "Manage Portfolios",
                &format!(
                    "{} · Drag to reorder, click a name to rename.",
                    if input.rows.len() == 1 {
                        "1 portfolio".to_owned()
                    } else {
                        format!("{} portfolios", input.rows.len())
                    }
                ),
                theme::ACCENT,
            );
        });
        ui.add_space(16.0);
        if input.rows.is_empty() {
            ui.allocate_ui(Vec2::new(ui.available_width(), 365.0), |ui| {
                ui.add_space(125.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No Portfolios").strong().size(16.0));
                    ui.label(
                        RichText::new("Create a portfolio to start tracking investments.").weak(),
                    );
                });
            });
        } else {
            let mut rects = Vec::with_capacity(input.rows.len());
            let mut drag_started = None;
            let mut drag_stopped = false;
            egui::ScrollArea::vertical()
                .max_height(365.0)
                .min_scrolled_height(365.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    sheet::inset(ui, |ui| {
                        for (index, row) in input.rows.iter().enumerate() {
                            let (rect, started, stopped) =
                                manage_row(ui, state, input, row, &mut actions);
                            rects.push(rect);
                            if started {
                                drag_started = Some(index);
                            }
                            drag_stopped |= stopped;
                        }
                    });
                });
            if let Some(index) = drag_started {
                state.dragging = Some(index);
            }
            if let Some(source) = state.dragging {
                let destination = ui
                    .input(|input| input.pointer.latest_pos())
                    .filter(|pointer| {
                        rects
                            .first()
                            .zip(rects.last())
                            .is_some_and(|(first, last)| {
                                pointer.x >= first.left() - 12.0
                                    && pointer.x <= first.right() + 12.0
                                    && pointer.y >= first.top() - 18.0
                                    && pointer.y <= last.bottom() + 18.0
                            })
                    })
                    .map(|pointer| {
                        rects
                            .iter()
                            .position(|rect| rect.center().y > pointer.y)
                            .unwrap_or(rects.len())
                    });
                if let Some(destination) = destination {
                    let y = rects.get(destination).map_or_else(
                        || rects.last().map_or(0.0, |rect| rect.bottom()),
                        |rect| rect.top(),
                    );
                    if let Some(first) = rects.first() {
                        ui.painter().line_segment(
                            [
                                egui::Pos2::new(first.left(), y),
                                egui::Pos2::new(first.right(), y),
                            ],
                            egui::Stroke::new(2.0, theme::text_secondary()),
                        );
                    }
                    if drag_stopped && destination != source && destination != source + 1 {
                        actions.push(ManageAction::Reorder {
                            from: vec![source],
                            destination,
                        });
                    }
                }
                if drag_stopped {
                    state.dragging = None;
                }
            }
        }
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if kit::push_button(ui, Some(Icon::Plus), "New Portfolio", true).clicked() {
                    actions.push(ManageAction::New);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Done").strong())
                                .min_size(Vec2::new(76.0, 30.0)),
                        )
                        .clicked()
                    {
                        actions.push(ManageAction::Done);
                    }
                });
            });
            ui.add_space(14.0);
        });
    });
    delete_dialog(ctx, state, input, &mut actions);
    actions
}

fn manage_row(
    ui: &mut egui::Ui,
    state: &mut ManageState,
    input: &ManageInput,
    row: &ManageRow,
    actions: &mut Vec<ManageAction>,
) -> (egui::Rect, bool, bool) {
    let portfolio = row.portfolio;
    let mut drag_started = false;
    let mut drag_stopped = false;
    let frame = egui::Frame::new()
        .fill(theme::ink(8))
        .stroke(egui::Stroke::new(1.0, theme::border()))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (handle, response) =
                    ui.allocate_exact_size(Vec2::new(14.0, 28.0), Sense::drag());
                let color = if response.hovered() {
                    theme::text_primary()
                } else {
                    theme::text_tertiary()
                };
                for offset in [-3.0, 0.0, 3.0] {
                    ui.painter().line_segment(
                        [
                            handle.center() + Vec2::new(-4.0, offset),
                            handle.center() + Vec2::new(4.0, offset),
                        ],
                        egui::Stroke::new(1.3, color),
                    );
                }
                drag_started = response.drag_started();
                drag_stopped = response.drag_stopped();
                response.on_hover_text("Drag to reorder");
                ui.vertical(|ui| {
                    let draft = state
                        .drafts
                        .entry(portfolio.id)
                        .or_insert_with(|| portfolio.name.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(draft)
                            .font(egui::TextStyle::Heading)
                            .frame(egui::Frame::NONE)
                            .desired_width(210.0),
                    );
                    if draft.chars().count() > MAX_NAME_LENGTH {
                        *draft = draft.chars().take(MAX_NAME_LENGTH).collect();
                    }
                    let committed = response.lost_focus();
                    if committed {
                        match rename_commit(draft, &portfolio.name) {
                            Some(name) => actions.push(ManageAction::Rename {
                                id: portfolio.id,
                                name,
                            }),
                            // A blank or unchanged name is put back.
                            None => *draft = portfolio.name.clone(),
                        }
                    } else if !response.has_focus() && *draft != portfolio.name {
                        // Renamed elsewhere while not editing: follow it.
                        *draft = portfolio.name.clone();
                    }
                    ui.label(
                        RichText::new(summary_line(
                            portfolio.base_currency,
                            row.transactions,
                            row.assets,
                        ))
                        .small()
                        .weak(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let weak = ui.visuals().weak_text_color();
                    if icons::button(ui, Icon::Trash, 28.0, weak, "Delete portfolio").clicked() {
                        state.pending_delete = Some(portfolio.id);
                    }
                    if icons::button(
                        ui,
                        Icon::CopyDoc,
                        28.0,
                        weak,
                        "Duplicate, including its transactions",
                    )
                    .clicked()
                    {
                        actions.push(ManageAction::Duplicate(portfolio.id));
                    }
                    if portfolio.is_archived {
                        ui.label(RichText::new("Archived").small().weak());
                    }
                    if input.current == Some(portfolio.id) {
                        ui.label(
                            RichText::new("Viewing")
                                .small()
                                .strong()
                                .color(theme::ACCENT),
                        );
                    }
                });
            });
        });
    ui.add_space(4.0);
    (frame.response.rect, drag_started, drag_stopped)
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut ManageState,
    input: &ManageInput,
    actions: &mut Vec<ManageAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(row) = input.rows.iter().find(|row| row.portfolio.id == id) else {
        state.pending_delete = None;
        return;
    };
    let title = format!("Delete {}?", row.portfolio.name);
    let message = delete_message(row.transactions);
    let result = kit::confirm(
        ctx,
        Id::new("portfolio-delete"),
        &Confirm {
            icon: Icon::Trash,
            title: &title,
            message: &message,
            destructive: "Delete Portfolio",
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(ManageAction::Delete(id));
    }
    if result.is_some() {
        state.pending_delete = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The height the create sheet settles at after a few frames with `name` typed.
    fn create_sheet_height(name: &str, existing: &[String]) -> f32 {
        let ctx = egui::Context::default();
        let mut state = CreateState {
            name: name.to_owned(),
            ..CreateState::default()
        };
        let mut height = 0.0;
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(900.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    show_create(ui.ctx(), &mut state, existing);
                },
            );
            output.textures_delta.clear();
            height = ctx
                .memory(|memory| memory.area_rect(Id::new("portfolio-create")))
                .map_or(0.0, |rect| rect.height());
        }
        height
    }

    #[test]
    fn the_create_sheet_keeps_its_height_when_the_duplicate_warning_appears() {
        let existing = vec!["Main".to_owned()];
        let plain = create_sheet_height("Savings", &existing);
        assert!(plain > 300.0, "the sheet did not lay out: {plain}");
        assert_eq!(create_sheet_height("main", &existing), plain);
        assert_eq!(create_sheet_height("", &existing), plain);
    }

    #[test]
    fn the_create_sheet_needs_a_name_and_trims_it() {
        let mut state = CreateState::default();
        assert!(!state.is_valid() && state.outcome().is_none());
        state.name = "   ".into();
        assert!(state.outcome().is_none());
        state.name = "  Long-term holds ".into();
        state.currency = PortfolioCurrency::Eur;
        assert_eq!(
            state.outcome(),
            Some(CreateOutcome::Create {
                name: "Long-term holds".into(),
                currency: PortfolioCurrency::Eur
            })
        );
    }

    #[test]
    fn names_are_cut_to_the_limit_and_duplicates_are_flagged_case_insensitively() {
        let mut state = CreateState {
            name: "x".repeat(60),
            ..CreateState::default()
        };
        state.clamp_name();
        assert_eq!(state.name.chars().count(), MAX_NAME_LENGTH);
        state.name = " main ".into();
        let existing = vec!["Main".to_owned(), "Other".to_owned()];
        assert!(state.is_duplicate(&existing));
        state.name = "Mainframe".into();
        assert!(!state.is_duplicate(&existing));
        state.name = "".into();
        assert!(
            !state.is_duplicate(&existing),
            "blank is invalid, not a duplicate"
        );
        assert_eq!(CreateState::default().currency, PortfolioCurrency::Usd);
    }

    #[test]
    fn summaries_pluralise_each_count() {
        assert_eq!(
            summary_line(PortfolioCurrency::Usd, 1, 1),
            "USD · 1 transaction · 1 asset"
        );
        assert_eq!(
            summary_line(PortfolioCurrency::Eur, 0, 3),
            "EUR · 0 transactions · 3 assets"
        );
    }

    #[test]
    fn arrows_reorder_by_insertion_point() {
        assert_eq!(move_up(0), None);
        assert_eq!(move_up(2), Some((vec![2], 1)));
        assert_eq!(move_down(2, 3), None);
        assert_eq!(
            move_down(0, 3),
            Some((vec![0], 2)),
            "insertion point is past the neighbour"
        );
        assert_eq!(move_down(1, 3), Some((vec![1], 3)));
    }

    /// `PortfolioLedger::reorder`'s rule: `destination` is an insertion point in the list as
    /// it stands, so removing items above it shifts it down.
    fn reorder(order: &mut Vec<u64>, from: &[usize], destination: usize) {
        let moving: Vec<u64> = from.iter().map(|&index| order[index]).collect();
        let mut descending = from.to_vec();
        descending.sort_unstable_by(|a, b| b.cmp(a));
        for index in descending {
            order.remove(index);
        }
        let removed_before = from.iter().filter(|&&index| index < destination).count();
        let insert_at = destination.saturating_sub(removed_before);
        order.splice(insert_at..insert_at, moving);
    }

    #[test]
    fn the_arrows_move_one_place_under_the_ledgers_reorder_rule() {
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_down(0, 3).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [2, 1, 3]);
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_up(2).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [1, 3, 2]);
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_down(1, 3).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [1, 3, 2]);
    }

    #[test]
    fn a_rename_commits_only_a_changed_non_blank_name() {
        assert_eq!(rename_commit("  Growth ", "Main"), Some("Growth".into()));
        assert_eq!(rename_commit("Main", "Main"), None);
        assert_eq!(
            rename_commit("   ", "Main"),
            None,
            "a blank name is put back"
        );
        assert_eq!(
            rename_commit(&"y".repeat(60), "Main")
                .unwrap()
                .chars()
                .count(),
            MAX_NAME_LENGTH
        );
    }

    #[test]
    fn deleting_warns_with_the_transaction_count() {
        assert!(delete_message(1).contains("its 1 transaction."));
        assert!(delete_message(5).contains("its 5 transactions."));
        assert!(delete_message(0).contains("0 transactions"));
    }
}
