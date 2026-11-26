//! The TradingView import from start to finish: read the app's data (off the frame), let the
//! person choose what to bring and which markets it becomes (matched on a thread of its own),
//! preview, and write. The sheets (`ui::tradingview`) draw the state held here and answer with
//! [`Action`]s; the rules are in `model::tradingview`.

mod apply;

pub use apply::Targets;

use crate::i18n::tr;
use crate::model::tradingview::plan::{self, Mappings, Match, Plan, Selection, Summary, Taken};
use crate::model::tradingview::{Library, TvSymbol};
use crate::model::watchlist::new_id;
use crate::model::{ChartAppearance, DataSource, Timeframe};
use crate::service::system::file_dialog;
use crate::service::tradingview::matcher::Search;
use crate::service::tradingview::{self, Scan, ScanError, matcher};
use crate::service::work_queue::{self, Pending};
use crate::ui::portfolio::asset_picker::{self, AssetPickerOutcome, AssetPickerState, PickerTab};
use crate::ui::tradingview::{self as sheets, Action, Content, Step, View};
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Reading the app's data.
type Read = Pending<Result<Scan, ScanError>>;

/// The thread that matches symbols, one at a time. Dropping it ends the thread after the symbol it
/// is on.
struct Resolver {
    requests: Sender<TvSymbol>,
    answers: Receiver<(TvSymbol, Match)>,
    asked: HashSet<TvSymbol>,
}

impl Resolver {
    fn start(search: Arc<Search>, ctx: egui::Context) -> Self {
        let (requests, wanted) = channel::<TvSymbol>();
        let (answered, answers) = channel();
        std::thread::spawn(move || {
            for symbol in wanted {
                let result = matcher::resolve(&symbol, &*search);
                if answered.send((symbol, result)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            requests,
            answers,
            asked: HashSet::new(),
        }
    }

    fn ask(&mut self, symbols: impl IntoIterator<Item = TvSymbol>) {
        for symbol in symbols {
            if self.asked.insert(symbol.clone()) {
                // The thread only ends when this is dropped.
                let _ = self.requests.send(symbol);
            }
        }
    }

    /// Forgets the symbols that could not be matched, so that they are asked again.
    fn forget(&mut self, symbols: &[TvSymbol]) {
        for symbol in symbols {
            self.asked.remove(symbol);
        }
    }
}

/// What is being chosen and previewed.
struct Review {
    /// Tells this reading apart from any other, for the sheet.
    opening: u64,
    folder: PathBuf,
    library: Library,
    selection: Selection,
    mappings: Mappings,
    resolver: Resolver,
    step: Step,
    /// Built on entering the preview.
    plan: Option<Plan>,
    /// The "Match…" picker, with the symbol it answers.
    picking: Option<(TvSymbol, AssetPickerState)>,
}

enum Stage {
    Reading(Read),
    ChoosingFolder(Pending<Option<PathBuf>>),
    Failed(String),
    Review(Box<Review>),
    Done(Summary),
}

/// What the tab does after a frame of the import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Still open.
    Open,
    Closed,
    /// Closed, and this saved layout is to be opened.
    OpenLayout(u64),
}

pub struct TradingViewImport {
    targets: Targets,
    search: Arc<Search>,
    stage: Stage,
    /// An import added scripts and the tab has not been told yet.
    added_scripts: bool,
}

impl TradingViewImport {
    /// Opens the import and starts reading the app's data.
    pub fn open(targets: Targets, ctx: &egui::Context) -> Self {
        Self::with_search(targets, Arc::new(crate::service::search), None, ctx)
    }

    pub fn with_search(
        targets: Targets,
        search: Arc<Search>,
        folder: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> Self {
        Self {
            targets,
            search,
            stage: read(folder, ctx),
            added_scripts: false,
        }
    }

    /// Whether an import added scripts since this was last asked.
    pub fn take_added_scripts(&mut self) -> bool {
        std::mem::take(&mut self.added_scripts)
    }

    /// Takes in what finished since the last frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        match &mut self.stage {
            Stage::Reading(job) => {
                let lost = || {
                    Err(ScanError::Unreadable(std::io::Error::other(tr!(
                        "the read stopped unexpectedly"
                    ))))
                };
                let Some(result) = job.poll_or(lost) else {
                    return;
                };
                self.stage = match result {
                    Ok(scan) => Stage::Review(Box::new(self.review(scan, ctx))),
                    Err(error) => Stage::Failed(error.to_string()),
                };
            }
            Stage::ChoosingFolder(job) => {
                if let Some(folder) = job.poll_or(|| None) {
                    self.stage = read(folder, ctx);
                }
            }
            Stage::Review(review) => {
                while let Ok((symbol, result)) = review.resolver.answers.try_recv() {
                    review.mappings.found(symbol, result);
                    review.plan = None;
                }
                let wanted = review.selection.symbols(&review.library);
                review
                    .resolver
                    .ask(wanted.into_iter().map(|(symbol, _)| symbol));
            }
            Stage::Failed(_) | Stage::Done(_) => {}
        }
    }

    fn review(&self, scan: Scan, ctx: &egui::Context) -> Review {
        Review {
            opening: new_id(),
            selection: Selection::all(&scan.library),
            folder: scan.folder,
            library: scan.library,
            mappings: Mappings::default(),
            resolver: Resolver::start(Arc::clone(&self.search), ctx.clone()),
            step: Step::Choose,
            plan: None,
            picking: None,
        }
    }

    /// The names an import must not take over, and the look a new chart gets.
    fn surroundings(&self) -> (Taken, ChartAppearance) {
        let (layouts, base) = {
            let store = crate::lock::lock(&self.targets.store);
            (
                store
                    .saved_views()
                    .into_iter()
                    .map(|view| view.name)
                    .collect(),
                store.default_chart_template().look.unwrap_or_default(),
            )
        };
        let watchlists = crate::lock::lock(&self.targets.watchlist)
            .lists()
            .iter()
            .map(|list| list.name.clone())
            .collect();
        let scripts = (self.targets.scripts)()
            .and_then(|mut library| library.all_scripts().ok())
            .map(|scripts| scripts.into_iter().map(|script| script.name).collect())
            .unwrap_or_default();
        (
            Taken {
                layouts,
                watchlists,
                scripts,
            },
            base,
        )
    }

    fn build_plan(&mut self) {
        let (taken, base) = self.surroundings();
        if let Stage::Review(review) = &mut self.stage {
            review.plan = Some(plan::plan(
                &review.library,
                &review.selection,
                &review.mappings,
                &taken,
                &base,
            ));
        }
    }

    /// Draws the sheet, applies what was done on it and says whether it is still open.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        self.poll(ctx);
        if let Stage::Review(review) = &mut self.stage
            && let Some((symbol, picker)) = &mut review.picking
        {
            match asset_picker::show(ctx, picker, &self.targets.store) {
                Some(AssetPickerOutcome::Choose(result)) => {
                    review.mappings.choose(symbol, matcher::market_of(&result));
                    review.picking = None;
                    review.plan = None;
                }
                Some(AssetPickerOutcome::Cancel) => review.picking = None,
                None => {}
            }
            return Outcome::Open;
        }
        // A choice on the preview (a layout's timeframe) makes the plan anew.
        if let Stage::Review(review) = &self.stage
            && review.step == Step::Preview
            && review.plan.is_none()
        {
            self.build_plan();
        }
        let actions = sheets::show(ctx, &self.view());
        let mut outcome = Outcome::Open;
        for action in actions {
            outcome = self.act(action, ctx);
            if outcome != Outcome::Open {
                break;
            }
        }
        outcome
    }

    fn view(&self) -> View<'_> {
        match &self.stage {
            Stage::Reading(_) | Stage::ChoosingFolder(_) => View::Reading,
            Stage::Failed(message) => View::Failed(message),
            Stage::Review(review) => View::Content(Content {
                opening: review.opening,
                folder: &review.folder,
                library: &review.library,
                selection: &review.selection,
                mappings: &review.mappings,
                step: review.step,
                plan: review.plan.as_ref(),
            }),
            Stage::Done(summary) => View::Done(summary),
        }
    }

    pub fn act(&mut self, action: Action, ctx: &egui::Context) -> Outcome {
        if action == Action::Close {
            return Outcome::Closed;
        }
        match action {
            Action::ChooseFolder => {
                let job = work_queue::SNAPSHOTS.start(Some(ctx), || {
                    file_dialog::choose_folder(tr!("Choose TradingView’s data folder"))
                        .ok()
                        .flatten()
                });
                self.stage = Stage::ChoosingFolder(job);
            }
            Action::Reload => {
                let folder = match &self.stage {
                    Stage::Review(review) => Some(review.folder.clone()),
                    _ => None,
                };
                self.stage = read(folder, ctx);
            }
            Action::OpenLayout(id) => return Outcome::OpenLayout(id),
            Action::Import => {
                if let Stage::Review(review) = &self.stage
                    && let Some(plan) = &review.plan
                {
                    let summary = apply::apply(plan, &self.targets);
                    self.added_scripts = summary.scripts > 0;
                    self.stage = Stage::Done(summary);
                }
            }
            Action::Preview => {
                if let Stage::Review(review) = &mut self.stage {
                    review.step = Step::Preview;
                    review.plan = None;
                }
                self.build_plan();
            }
            other => {
                if let Stage::Review(review) = &mut self.stage {
                    review.edit(other);
                }
            }
        }
        Outcome::Open
    }
}

impl Review {
    /// The actions that change only what is chosen.
    fn edit(&mut self, action: Action) {
        let toggle = |set: &mut std::collections::BTreeSet<String>, id: String| {
            if !set.remove(&id) {
                set.insert(id);
            }
        };
        match action {
            Action::Back => self.step = Step::Choose,
            Action::ToggleLayout(id) => toggle(&mut self.selection.layouts, id),
            Action::ToggleWatchlist(id) => toggle(&mut self.selection.watchlists, id),
            Action::ToggleScript(id) => toggle(&mut self.selection.scripts, id),
            Action::SetTimeframe(layout, timeframe) => self.set_timeframe(layout, timeframe),
            Action::Match(symbol) => {
                let title = tr!("Match {symbol}", symbol = symbol.as_str());
                // Searched for by its base asset, which every exchange spells alike.
                let query = symbol
                    .pair()
                    .map_or_else(|| symbol.ticker(), |(base, _)| base);
                let mut picker = AssetPickerState::new(
                    &title,
                    tr!("Pick the market that stands in for this TradingView symbol."),
                    tr!("Use Market"),
                )
                .with_query(query);
                if symbol.source() == Some(DataSource::Alpaca) {
                    picker.select_tab(PickerTab::Stocks);
                }
                self.picking = Some((symbol, picker));
            }
            Action::Skip(symbol) => self.mappings.skip(&symbol),
            Action::Reset(symbol) => self.mappings.reset(&symbol),
            Action::Retry => {
                let failed: Vec<TvSymbol> = self
                    .selection
                    .symbols(&self.library)
                    .into_iter()
                    .map(|(symbol, _)| symbol)
                    .filter(|symbol| matches!(self.mappings.status(symbol), Match::Unmatched(_)))
                    .collect();
                for symbol in &failed {
                    self.mappings.found(symbol.clone(), Match::Searching);
                }
                self.resolver.forget(&failed);
            }
            _ => {}
        }
        self.plan = None;
    }

    fn set_timeframe(&mut self, layout: String, timeframe: Timeframe) {
        self.selection.timeframes.insert(layout, timeframe);
    }
}

fn read(folder: Option<PathBuf>, ctx: &egui::Context) -> Stage {
    Stage::Reading(
        work_queue::SNAPSHOTS.start(Some(ctx), move || tradingview::scan(folder.as_deref())),
    )
}

#[cfg(test)]
mod tests;
