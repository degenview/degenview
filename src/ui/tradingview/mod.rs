//! The TradingView import's sheets: the welcome, reading, choosing what to bring and which markets
//! stand for its symbols, the preview, and the result. A pure view over what `host::tradingview` holds; it
//! answers with [`Action`]s.

mod choose;
mod done;
mod pictures;
mod preview;
mod result;
pub mod welcome;
pub mod words;

#[cfg(test)]
mod gallery_tests;

use crate::i18n::{tr, tr_c};
use crate::model::Timeframe;
use crate::model::tradingview::library::Contents;
use crate::model::tradingview::plan::{Mappings, Plan, Selection, Summary};
use crate::model::tradingview::{Library, TvSymbol};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::scroll::{Beyond, fade_edges};
use crate::ui::kit::{self, sheet};
use crate::ui::theme;
use eframe::egui::{self, Id, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::cmp::Ordering;
use std::path::Path;

pub const SIZE: Vec2 = Vec2::new(760.0, 680.0);

const HEADER_PADDING: f32 = 24.0;
/// Room under the scrolling body for the footer: its hairline, padding and 28 pt buttons.
const FOOTER_HEIGHT: f32 = 58.0;
/// The same under the two steps, whose buttons are the large ones beside two lines of words.
const STEP_FOOTER_HEIGHT: f32 = 62.0;
/// A step's segment in the header, and the room between them.
const STEP_WIDTH: f32 = 72.0;
const STEP_GAP: f32 = 6.0;

/// A kind of thing there is to bring over.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Group {
    Layouts,
    Watchlists,
    Scripts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// What to bring over, and which markets.
    Choose,
    Preview,
}

/// What was found and what is chosen of it.
pub struct Content<'a> {
    /// Differs between openings of the import, so that its lists start at the top each time.
    pub opening: u64,
    pub folder: &'a Path,
    pub library: &'a Library,
    pub selection: &'a Selection,
    pub mappings: &'a Mappings,
    /// Whether drawings are still being given their place.
    pub placing: bool,
    /// Whether a drawing's place could not be looked up.
    pub unplaced: bool,
    pub step: Step,
    /// Present on the preview.
    pub plan: Option<&'a Plan>,
}

/// What the sheet is waiting for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Work {
    /// The data is being read again.
    Reading,
    /// What the preview is planned against is being read.
    Preparing,
    /// The import is being written, which cannot be left halfway.
    Importing,
}

pub enum View<'a> {
    /// The first page, with what was found once the data is read.
    Welcome(Option<&'a Contents>),
    Working(Work),
    /// What went wrong, and what to do about it.
    Failed(&'a str, &'a str),
    Content(Content<'a>),
    Done(&'a Summary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Close,
    /// Leave the welcome for choosing what to bring.
    Begin,
    /// Read another folder.
    ChooseFolder,
    /// Read the same folder again.
    Reload,
    Back,
    Preview,
    Import,
    /// Search again for the symbols that failed.
    Retry,
    ToggleLayout(String),
    ToggleWatchlist(String),
    ToggleScript(String),
    /// Take all of a kind, or none of it.
    SelectAll(Group, bool),
    SetTimeframe(String, Timeframe),
    Match(TvSymbol),
    Skip(TvSymbol),
    Reset(TvSymbol),
    OpenLayout(u64),
}

pub fn show(ctx: &egui::Context, view: &View<'_>) -> Vec<Action> {
    // The welcome is a sheet of its own shape.
    if let View::Welcome(found) = view {
        return welcome::show(ctx, *found, welcome::Occasion::Opened)
            .map(|choice| match choice {
                welcome::Choice::Import => Action::Begin,
                welcome::Choice::Dismiss => Action::Close,
            })
            .into_iter()
            .collect();
    }
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("tradingview-import"), SIZE.x, |ui| {
        body(ui, view, &mut actions);
    });
    if actions.is_empty() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        actions.extend(escape(view));
    }
    actions
}

/// Escape steps back from the preview, else closes; an import being written is not left.
fn escape(view: &View<'_>) -> Option<Action> {
    match view {
        View::Working(Work::Importing) => None,
        View::Working(Work::Preparing) => Some(Action::Back),
        View::Content(content) if content.step == Step::Preview => Some(Action::Back),
        _ => Some(Action::Close),
    }
}

/// The sheet's inside, also what the gallery draws.
pub fn body(ui: &mut Ui, view: &View<'_>, actions: &mut Vec<Action>) {
    // The whole sheet, footer included, stays inside a small window.
    let tall = SIZE.y.min(ui.ctx().content_rect().height() - 40.0);
    ui.set_height(tall);
    ui.spacing_mut().item_spacing.y = 0.0;
    let top = ui.cursor().top();
    ui.add_space(HEADER_PADDING - 4.0);
    header(ui, view);
    let footer = match view {
        View::Content(_) | View::Done(_) => STEP_FOOTER_HEIGHT,
        _ => FOOTER_HEIGHT,
    };
    // `set_height` made the sheet's own rect SIZE.y tall, so measure from the cursor.
    let height = tall - (ui.cursor().top() - top) - footer;
    match view {
        // Drawn by `welcome`, never inside this sheet.
        View::Welcome(_) => {}
        View::Working(work) => result::working(ui, height, *work, actions),
        View::Failed(message, hint) => result::failed(ui, height, message, hint, actions),
        View::Content(content) => match (content.step, content.plan) {
            (Step::Preview, Some(plan)) => preview::show(ui, height, content, plan, actions),
            _ => choose::show(ui, height, content, actions),
        },
        View::Done(summary) => done::show(ui, height, summary, actions),
    }
}

/// The title and what the step is for, with where it stands among the three at the trailing
/// edge.
fn header(ui: &mut Ui, view: &View<'_>) {
    let (title, detail) = headline(view);
    let current = match view {
        View::Content(content) if content.step == Step::Preview => Some(1),
        View::Working(Work::Preparing) => Some(1),
        View::Content(_) | View::Working(Work::Reading) => Some(0),
        View::Working(Work::Importing) | View::Done(_) => Some(2),
        View::Welcome(_) | View::Failed(..) => None,
    };
    sheet::inset(ui, |ui| {
        let steps = [
            tr_c!("import step", "Choose"),
            tr_c!("import step", "Preview"),
            tr_c!("import step", "Done"),
        ];
        let across = current.map_or(0.0, |_| STEP_WIDTH * 3.0 + STEP_GAP * 2.0);
        let room = ui.available_width() - across - 20.0;
        let title = kit::paragraph(
            ui,
            title,
            kit::size::TITLE2 + 3.0,
            kit::SEMIBOLD,
            theme::text_primary(),
            room,
        );
        let detail = kit::paragraph(
            ui,
            detail,
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
            room,
        );
        let words = title.size().y + 4.0 + detail.size().y;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), words), Sense::hover());
        let below = row.top() + title.size().y + 4.0;
        ui.painter().galley(row.min, title, theme::text_primary());
        ui.painter()
            .galley(pos2(row.left(), below), detail, theme::text_secondary());
        let Some(current) = current else {
            return;
        };
        let mut left = row.right() - across;
        for (index, label) in steps.into_iter().enumerate() {
            let (color, bar) = match index.cmp(&current) {
                Ordering::Equal => (theme::text_primary(), theme::text_primary()),
                Ordering::Less => (theme::text_secondary(), theme::text_tertiary()),
                Ordering::Greater => (theme::text_tertiary(), theme::separator()),
            };
            let text = kit::galley(
                ui,
                label,
                kit::size::CAPTION + 0.5,
                kit::MEDIUM,
                color,
                STEP_WIDTH,
            );
            ui.painter().galley(
                pos2(
                    left + (STEP_WIDTH - text.size().x) / 2.0,
                    row.center().y - text.size().y - 1.0,
                ),
                text,
                color,
            );
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(left, row.center().y + 4.0), vec2(STEP_WIDTH, 2.0)),
                1.0,
                bar,
            );
            left += STEP_WIDTH + STEP_GAP;
        }
    });
    ui.add_space(16.0);
    ui.separator();
}

fn headline(view: &View<'_>) -> (&'static str, &'static str) {
    match view {
        View::Working(Work::Preparing) => (
            tr!("Import Preview"),
            tr!("What will be added. Nothing of yours is replaced."),
        ),
        View::Content(content) if content.step == Step::Preview => (
            tr!("Import Preview"),
            tr!("What will be added. Nothing of yours is replaced."),
        ),
        View::Done(_) => (tr!("Import Complete"), tr!("From the TradingView app.")),
        _ => (
            tr!("Import from TradingView"),
            tr!("Layouts, drawings, watchlists and scripts the TradingView app has loaded."),
        ),
    }
}

/// A heading of a step: a bare glyph and a title, with what follows it on the same line left to
/// `add`, which is given the row and where the title ends.
fn section_title(ui: &mut Ui, icon: Icon, title: &str, add: impl FnOnce(&Ui, Rect, f32)) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(row.left() + 8.0, row.center().y), Vec2::splat(16.0)),
        icon,
        theme::text_secondary(),
    );
    let title = kit::galley(
        ui,
        title,
        kit::size::HEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        row.width() - 24.0,
    );
    let end = row.left() + 24.0 + title.size().x;
    ui.painter().galley(
        pos2(row.left() + 24.0, row.center().y - title.size().y / 2.0),
        title,
        egui::Color32::PLACEHOLDER,
    );
    add(ui, row, end);
}

/// A scrolling body of `height`, inset like the header.
fn scrolling(ui: &mut Ui, height: f32, id: (&str, u64), add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        kit::scroll::keep_bars_visible(ui);
        // As wide as the header, whether or not the bar is there; it sits in the right inset.
        let width = ui.available_width() - 2.0 * f32::from(sheet::INSET);
        let shown = egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: sheet::INSET,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(16.0);
                        add(ui);
                        ui.add_space(16.0);
                    });
            });
        let beyond = Beyond::of(
            shown.state.offset.y,
            shown.content_size.y,
            shown.inner_rect.height(),
        );
        fade_edges(ui.painter(), shown.inner_rect, theme::chrome(), beyond);
    });
}
