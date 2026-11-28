//! The TradingView import from start to finish: read the app's data, let the person choose what
//! to bring and which markets it becomes, preview, and write. Reading, matching and writing all
//! run on the work queues; the frame only takes in their answers. The sheets (`ui::tradingview`)
//! draw the state held here and answer with [`Action`]s; the rules are in `model::tradingview`.

mod apply;
pub mod offer;

pub use apply::Targets;

use apply::Order;

use crate::i18n::tr;
use crate::model::tradingview::library::Contents;
use crate::model::tradingview::placement::Spot;
use crate::model::tradingview::plan::{
    self, Mappings, Match, Placements, Plan, Selection, Summary, Taken,
};
use crate::model::tradingview::{Library, TvSymbol};
use crate::model::watchlist::new_id;
use crate::model::{ChartAppearance, DataSource, Timeframe};
use crate::service::system::file_dialog;
use crate::service::tradingview::matcher::{self, Resolution, Search};
use crate::service::tradingview::{self, Candles, Scan, ScanError};
use crate::service::work_queue::{self, Pending, PendingMap};
use crate::service::{ProviderError, SearchResult};
use crate::ui::portfolio::asset_picker::{self, AssetPickerOutcome, AssetPickerState, PickerTab};
use crate::ui::tradingview::{self as sheets, Action, Content, Group, Step, View, Work};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Searches and price lookups under way at once. Few enough to leave the search workers to a
/// symbol picker, and what waits is still dropped when it stops being chosen.
const SEARCHES_AT_ONCE: usize = 4;
const PRICES_AT_ONCE: usize = 2;

/// Reading the app's data.
type Read = Pending<Result<Scan, ScanError>>;

/// Where the import looks things up.
#[derive(Clone)]
pub struct Providers {
    pub search: Arc<Search>,
    pub candles: Arc<Candles>,
}

/// Where an import starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Start {
    /// On its welcome: what it does and what there is to bring over.
    Welcome,
    /// Past it, on choosing what to bring: the person has just seen it, as the offer at launch.
    Choose,
}

/// The search for the chosen symbols' markets.
struct Matching {
    search: Arc<Search>,
    /// Searches begun and not yet answered.
    open: HashMap<TvSymbol, Resolution>,
    requests: PendingMap<TvSymbol, Result<Vec<SearchResult>, ProviderError>>,
}

impl Matching {
    fn new(search: Arc<Search>) -> Self {
        Self {
            search,
            open: HashMap::new(),
            requests: PendingMap::new(SEARCHES_AT_ONCE),
        }
    }

    /// Takes in the answers and asks what the `wanted` symbols still need; `true` when a symbol
    /// got its match.
    fn poll(&mut self, wanted: &[TvSymbol], mappings: &mut Mappings, ctx: &egui::Context) -> bool {
        for (symbol, answer) in self.requests.take_ended() {
            if let Some(resolution) = self.open.get_mut(&symbol) {
                match answer {
                    Some(answer) => resolution.answer(answer),
                    None => resolution.abandoned(),
                }
            }
        }
        let mut matched = false;
        for symbol in wanted {
            if mappings.has_answer(symbol) || self.requests.contains(symbol) {
                continue;
            }
            let resolution = self
                .open
                .entry(symbol.clone())
                .or_insert_with(|| Resolution::new(symbol));
            match resolution.step() {
                matcher::Step::Done(found) => {
                    mappings.found(symbol.clone(), found.clone());
                    self.open.remove(symbol);
                    matched = true;
                }
                matcher::Step::Ask(source, name) => {
                    let (search, name) = (Arc::clone(&self.search), name.to_owned());
                    self.requests.start_with(symbol, || {
                        work_queue::start_search(source, Some(ctx), move || search(source, &name))
                    });
                }
            }
        }
        matched
    }
}

/// The prices of drawings that sit beside a bar.
struct Placing {
    candles: Arc<Candles>,
    requests: PendingMap<Spot, Option<f64>>,
}

impl Placing {
    fn new(candles: Arc<Candles>) -> Self {
        Self {
            candles,
            requests: PendingMap::new(PRICES_AT_ONCE),
        }
    }

    /// As [`Matching::poll`], for the `wanted` places.
    fn poll(&mut self, wanted: &[Spot], placements: &mut Placements, ctx: &egui::Context) -> bool {
        let ended = self.requests.take_ended();
        let placed = !ended.is_empty();
        for (spot, price) in ended {
            placements.insert(spot, price.flatten());
        }
        for spot in wanted {
            if placements.contains_key(spot) {
                continue;
            }
            let (candles, asked) = (Arc::clone(&self.candles), spot.clone());
            self.requests.start_with(spot, || {
                work_queue::start_for(asked.source, Some(ctx), move || {
                    candles(&asked).and_then(|candles| asked.price(&candles))
                })
            });
        }
        placed
    }
}

/// The names in use and the look of a new chart, which the preview is planned against.
enum Surroundings {
    Unread,
    Reading(Pending<(Taken, ChartAppearance)>),
    /// Kept while the preview is open, since the plan is made anew with every answer that
    /// arrives.
    Read(Box<(Taken, ChartAppearance)>),
}

/// What is being chosen and previewed.
struct Review {
    /// Tells this reading apart from any other, for the sheet.
    opening: u64,
    folder: PathBuf,
    library: Library,
    /// The library in outline, for the welcome.
    contents: Contents,
    selection: Selection,
    mappings: Mappings,
    matching: Matching,
    placing: Placing,
    placements: Placements,
    step: Step,
    surroundings: Surroundings,
    /// Made once the preview's surroundings are read.
    plan: Option<Plan>,
    /// The "Match…" picker, with the symbol it answers.
    picking: Option<(TvSymbol, AssetPickerState)>,
}

enum Stage {
    Reading(Read),
    Failed {
        message: String,
        hint: &'static str,
    },
    Review(Box<Review>),
    /// Being written.
    Applying(Pending<Summary>),
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
    providers: Providers,
    stage: Stage,
    /// The welcome is still up. The data is read and matched behind it.
    welcoming: bool,
    /// The folder the person chose; `None` reads where the app keeps its data.
    chosen: Option<PathBuf>,
    /// The folder panel, while it is open. The sheet under it stays as it was.
    folder_panel: Option<Pending<Option<PathBuf>>>,
    /// What was chosen and decided before the data was read again.
    carried: Option<(Library, Selection, Mappings)>,
    /// Something was written and the tab has not been told yet.
    wrote: bool,
    /// An import added scripts and the tab has not been told yet.
    added_scripts: bool,
}

impl TradingViewImport {
    /// Opens the import and starts reading the app's data.
    pub fn open(targets: Targets, start: Start, ctx: &egui::Context) -> Self {
        let providers = Providers {
            search: Arc::new(crate::service::search),
            candles: Arc::new(tradingview::candles_around),
        };
        Self::with_providers(targets, providers, None, start, ctx)
    }

    pub fn with_providers(
        targets: Targets,
        providers: Providers,
        folder: Option<PathBuf>,
        start: Start,
        ctx: &egui::Context,
    ) -> Self {
        Self {
            targets,
            providers,
            welcoming: start == Start::Welcome,
            stage: read(folder.clone(), ctx),
            chosen: folder,
            folder_panel: None,
            carried: None,
            wrote: false,
            added_scripts: false,
        }
    }

    /// Whether an import added scripts since this was last asked.
    pub fn take_added_scripts(&mut self) -> bool {
        std::mem::take(&mut self.added_scripts)
    }

    /// Whether anything was written since this was last asked.
    pub fn take_wrote(&mut self) -> bool {
        std::mem::take(&mut self.wrote)
    }

    /// Reads the data again, keeping what was chosen so far.
    fn reread(&mut self, ctx: &egui::Context) {
        // An import under way or over is not replaced: a folder panel may answer that late.
        if matches!(self.stage, Stage::Applying(_) | Stage::Done(_)) {
            return;
        }
        if let Stage::Review(review) =
            std::mem::replace(&mut self.stage, read(self.chosen.clone(), ctx))
        {
            self.carried = Some((review.library, review.selection, review.mappings));
        }
    }

    /// Takes in what finished since the last frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(panel) = &mut self.folder_panel
            && let Some(answer) = panel.poll_or(|| None)
        {
            self.folder_panel = None;
            // A cancelled panel changes nothing.
            if let Some(folder) = answer {
                self.chosen = Some(folder);
                self.reread(ctx);
            }
        }
        if let Stage::Reading(job) = &mut self.stage {
            let lost = || {
                Err(ScanError::Unreadable(std::io::Error::other(tr!(
                    "the read stopped unexpectedly"
                ))))
            };
            if let Some(result) = job.poll_or(lost) {
                self.stage = match result {
                    Ok(scan) => Stage::Review(Box::new(self.review(scan))),
                    Err(error) => Stage::Failed {
                        message: error.to_string(),
                        hint: error.hint(),
                    },
                };
            }
        }
        match &mut self.stage {
            Stage::Review(review) => review.poll(ctx),
            Stage::Applying(job) => {
                let summary = job.poll();
                if summary.is_some() || !job.is_pending() {
                    self.finish(summary);
                }
            }
            Stage::Reading(_) | Stage::Failed { .. } | Stage::Done(_) => {}
        }
    }

    /// Ends the import with what was written; `None` when the job died on the way, which may have
    /// left some of it written.
    fn finish(&mut self, summary: Option<Summary>) {
        self.wrote = true;
        self.added_scripts = summary
            .as_ref()
            .is_none_or(|summary| !summary.scripts.is_empty());
        self.stage = Stage::Done(summary.unwrap_or_else(|| Summary {
            failures: vec![
                tr!("The import stopped unexpectedly. Some of it may already have been added.")
                    .to_owned(),
            ],
            ..Summary::default()
        }));
    }

    fn review(&mut self, scan: Scan) -> Review {
        let (selection, mappings) = match self.carried.take() {
            Some((before, selection, mappings)) => (
                selection.carried_to(&before, &scan.library),
                mappings.decisions(),
            ),
            None => (Selection::all(&scan.library), Mappings::default()),
        };
        Review {
            opening: new_id(),
            selection,
            folder: scan.folder,
            contents: scan.library.contents(),
            library: scan.library,
            mappings,
            matching: Matching::new(Arc::clone(&self.providers.search)),
            placing: Placing::new(Arc::clone(&self.providers.candles)),
            placements: Placements::new(),
            step: Step::Choose,
            surroundings: Surroundings::Unread,
            plan: None,
            picking: None,
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
            Stage::Reading(_) if self.welcoming => View::Welcome(None),
            Stage::Review(review) if self.welcoming => View::Welcome(Some(&review.contents)),
            Stage::Reading(_) => View::Working(Work::Reading),
            Stage::Failed { message, hint } => View::Failed(message, hint),
            Stage::Applying(_) => View::Working(Work::Importing),
            Stage::Review(review) if review.step == Step::Preview && review.plan.is_none() => {
                View::Working(Work::Preparing)
            }
            Stage::Review(review) => View::Content(Content {
                opening: review.opening,
                folder: &review.folder,
                library: &review.library,
                selection: &review.selection,
                mappings: &review.mappings,
                placing: review
                    .selection
                    .spots(&review.library, &review.mappings)
                    .iter()
                    .any(|spot| !review.placements.contains_key(spot)),
                unplaced: review.placements.values().any(Option::is_none),
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
            Action::ChooseFolder if self.folder_panel.is_none() => {
                self.folder_panel = Some(work_queue::SNAPSHOTS.start(Some(ctx), || {
                    file_dialog::choose_folder(tr!("Choose TradingView’s data folder"))
                        .ok()
                        .flatten()
                }));
            }
            Action::ChooseFolder => {}
            Action::Begin => self.welcoming = false,
            Action::Reload => self.reread(ctx),
            Action::OpenLayout(id) => return Outcome::OpenLayout(id),
            Action::Import => self.import(ctx),
            Action::Preview => {
                if let Stage::Review(review) = &mut self.stage
                    && review.step != Step::Preview
                {
                    review.step = Step::Preview;
                    review.plan = None;
                    let targets = self.targets.clone();
                    review.surroundings =
                        Surroundings::Reading(work_queue::start(Some(ctx), move || {
                            apply::surroundings(&targets)
                        }));
                }
            }
            other => {
                if let Stage::Review(review) = &mut self.stage {
                    review.edit(other);
                }
            }
        }
        Outcome::Open
    }

    /// Writes what the preview shows, which only a preview that is there can ask for.
    fn import(&mut self, ctx: &egui::Context) {
        // The review goes into the job, so a placeholder holds its place for a moment.
        let stage = std::mem::replace(&mut self.stage, Stage::Done(Summary::default()));
        self.stage = match stage {
            Stage::Review(review) if review.plan.is_some() => {
                let order = Order {
                    library: review.library,
                    selection: review.selection,
                    mappings: review.mappings,
                    placements: review.placements,
                };
                let targets = self.targets.clone();
                Stage::Applying(work_queue::start(Some(ctx), move || {
                    apply::commit(&order, &targets)
                }))
            }
            other => other,
        };
    }
}

impl Review {
    /// Takes in what finished since the last frame, and plans the preview once it can be.
    fn poll(&mut self, ctx: &egui::Context) {
        let wanted: Vec<TvSymbol> = self
            .selection
            .symbols(&self.library)
            .into_iter()
            .map(|(symbol, _)| symbol)
            .collect();
        if self.matching.poll(&wanted, &mut self.mappings, ctx) {
            self.plan = None;
        }
        let spots = self.selection.spots(&self.library, &self.mappings);
        if self.placing.poll(&spots, &mut self.placements, ctx) {
            self.plan = None;
        }
        if let Surroundings::Reading(job) = &mut self.surroundings {
            match job.poll() {
                Some(read) => self.surroundings = Surroundings::Read(Box::new(read)),
                // A read that died is asked for again by asking for the preview again.
                None if !job.is_pending() => self.edit(Action::Back),
                None => {}
            }
        }
        if self.step == Step::Preview
            && self.plan.is_none()
            && let Surroundings::Read(read) = &self.surroundings
        {
            let (taken, base) = read.as_ref();
            self.plan = Some(plan::plan(
                &self.library,
                &self.selection,
                &self.mappings,
                &self.placements,
                taken,
                base,
            ));
        }
    }

    /// The actions that change only what is chosen.
    fn edit(&mut self, action: Action) {
        let toggle = |set: &mut std::collections::BTreeSet<String>, id: String| {
            if !set.remove(&id) {
                set.insert(id);
            }
        };
        match action {
            Action::Back => {
                self.step = Step::Choose;
                self.surroundings = Surroundings::Unread;
            }
            Action::ToggleLayout(id) => toggle(&mut self.selection.layouts, id),
            Action::ToggleWatchlist(id) => toggle(&mut self.selection.watchlists, id),
            Action::ToggleScript(id) => toggle(&mut self.selection.scripts, id),
            Action::SelectAll(group, on) => self.select_all(group, on),
            Action::SetTimeframe(layout, timeframe) => self.set_timeframe(layout, timeframe),
            Action::Match(symbol) => {
                let title = tr!("Match {symbol}", symbol = symbol.as_str());
                // Searched for by its base asset, which every exchange spells alike.
                let query = symbol
                    .pair()
                    .map_or_else(|| symbol.ticker(), |(base, _)| base);
                // A TradingView symbol is never a prediction market.
                let mut picker = AssetPickerState::new(
                    &title,
                    tr!("Pick the market that stands in for this TradingView symbol."),
                    tr!("Use Market"),
                )
                .with_tabs(&[PickerTab::Crypto, PickerTab::Stocks])
                .with_query(query);
                if symbol.source() == Some(DataSource::Alpaca) {
                    picker.select_tab(PickerTab::Stocks);
                }
                self.picking = Some((symbol, picker));
            }
            Action::Skip(symbol) => self.mappings.skip(&symbol),
            Action::Reset(symbol) => self.mappings.reset(&symbol),
            Action::Retry => {
                for (symbol, _) in self.selection.symbols(&self.library) {
                    if matches!(self.mappings.status(&symbol), Match::Unmatched(_)) {
                        self.mappings.forget_found(&symbol);
                    }
                }
                self.placements.retain(|_, price| price.is_some());
            }
            _ => {}
        }
        self.plan = None;
    }

    fn select_all(&mut self, group: Group, on: bool) {
        let mut all = Selection::all(&self.library);
        if !on {
            all = Selection::default();
        }
        match group {
            Group::Layouts => self.selection.layouts = all.layouts,
            Group::Watchlists => self.selection.watchlists = all.watchlists,
            Group::Scripts => self.selection.scripts = all.scripts,
        }
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
