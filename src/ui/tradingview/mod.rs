//! The TradingView import's sheet: reading, choosing what to bring and which markets stand for
//! its symbols, the preview, and the result. A pure view over what `host::tradingview` holds; it
//! answers with [`Action`]s.

mod choose;
mod preview;
mod result;
pub mod words;

#[cfg(test)]
mod gallery_tests;

use crate::i18n::tr;
use crate::model::Timeframe;
use crate::model::tradingview::plan::{Mappings, Plan, Selection, Summary};
use crate::model::tradingview::{Library, TvSymbol};
use crate::ui::chart::picker_kit::{art, widgets};
use crate::ui::kit::sheet;
use eframe::egui::{self, Id, Ui, Vec2};
use std::path::Path;

pub const SIZE: Vec2 = Vec2::new(760.0, 680.0);

const HEADER_PADDING: f32 = 24.0;
/// Room under the scrolling body for the footer: its hairline, padding and 28 pt buttons.
const FOOTER_HEIGHT: f32 = 58.0;

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

pub enum View<'a> {
    Reading,
    /// What went wrong, and what to do about it.
    Failed(&'a str, &'a str),
    Content(Content<'a>),
    Done(&'a Summary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Close,
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
    SetTimeframe(String, Timeframe),
    Match(TvSymbol),
    Skip(TvSymbol),
    Reset(TvSymbol),
    OpenLayout(u64),
}

pub fn show(ctx: &egui::Context, view: &View<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("tradingview-import"), SIZE.x, |ui| {
        body(ui, view, &mut actions);
    });
    // Escape steps back from the preview, else closes.
    if actions.is_empty() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        let previewing = matches!(view, View::Content(content) if content.step == Step::Preview);
        actions.push(if previewing {
            Action::Back
        } else {
            Action::Close
        });
    }
    actions
}

/// The sheet's inside, also what the gallery draws.
pub fn body(ui: &mut Ui, view: &View<'_>, actions: &mut Vec<Action>) {
    // The whole sheet, footer included, stays inside a small window.
    let tall = SIZE.y.min(ui.ctx().content_rect().height() - 40.0);
    ui.set_height(tall);
    ui.spacing_mut().item_spacing.y = 0.0;
    let top = ui.cursor().top();
    ui.add_space(HEADER_PADDING);
    let (title, detail) = headline(view);
    sheet::inset(ui, |ui| {
        widgets::header(ui, art::Scene::ChartImport, title, detail);
    });
    ui.add_space(16.0);
    // `set_height` made the sheet's own rect SIZE.y tall, so measure from the cursor.
    let height = tall - (ui.cursor().top() - top) - FOOTER_HEIGHT;
    match view {
        View::Reading => result::reading(ui, height, actions),
        View::Failed(message, hint) => result::failed(ui, height, message, hint, actions),
        View::Content(content) => match (content.step, content.plan) {
            (Step::Preview, Some(plan)) => preview::show(ui, height, content, plan, actions),
            _ => choose::show(ui, height, content, actions),
        },
        View::Done(summary) => result::done(ui, height, summary, actions),
    }
}

fn headline(view: &View<'_>) -> (&'static str, &'static str) {
    match view {
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

/// A scrolling body of `height`, inset like the header.
fn scrolling(ui: &mut Ui, height: f32, id: (&str, u64), add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            sheet::inset(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                add(ui);
                ui.add_space(12.0);
            });
        });
}
