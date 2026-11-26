//! The TradingView import from start to finish: read the app's data (off the frame), let the
//! person choose what to bring and which markets it becomes (matched on a thread of its own),
//! preview, and write. The sheets (`ui::tradingview`) draw the state held here and answer with
//! [`Action`]s; the rules are in `model::tradingview`.

mod apply;

pub use apply::Targets;

use crate::i18n::tr;
use crate::model::tradingview::placement::Spot;
use crate::model::tradingview::plan::{
    self, Mappings, Match, Placements, Plan, Selection, Summary, Taken,
};
use crate::model::tradingview::{Library, TvSymbol};
use crate::model::watchlist::new_id;
use crate::model::{ChartAppearance, DataSource, Timeframe};
use crate::service::system::file_dialog;
use crate::service::tradingview::matcher::Search;
use crate::service::tradingview::{self, Candles, Scan, ScanError, matcher};
use crate::service::work_queue::{self, Pending};
use crate::ui::portfolio::asset_picker::{self, AssetPickerOutcome, AssetPickerState, PickerTab};
use crate::ui::tradingview::{self as sheets, Action, Content, Step, View};
use eframe::egui;
use std::collections::HashSet;
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Reading the app's data.
type Read = Pending<Result<Scan, ScanError>>;

/// A thread that answers questions one at a time, each asked once. Dropping it ends the thread
/// after the question it is on.
struct Lookup<Q, A> {
    requests: Sender<Q>,
    answers: Receiver<(Q, A)>,
    asked: HashSet<Q>,
}

impl<Q, A> Lookup<Q, A>
where
    Q: Clone + Eq + Hash + Send + 'static,
    A: Send + 'static,
{
    fn start(answer: impl Fn(&Q) -> A + Send + 'static, ctx: egui::Context) -> Self {
        let (requests, wanted) = channel::<Q>();
        let (answered, answers) = channel();
        std::thread::spawn(move || {
            for question in wanted {
                let result = answer(&question);
                if answered.send((question, result)).is_err() {
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

    fn ask(&mut self, questions: impl IntoIterator<Item = Q>) {
        for question in questions {
            if self.asked.insert(question.clone()) {
                // The thread only ends when this is dropped.
                let _ = self.requests.send(question);
            }
        }
    }

    /// Forgets questions whose answer did not help, so that they are asked again.
    fn forget(&mut self, questions: &[Q]) {
        for question in questions {
            self.asked.remove(question);
        }
    }
}

/// Where the import looks things up.
#[derive(Clone)]
pub struct Providers {
    pub search: Arc<Search>,
    pub candles: Arc<Candles>,
}

/// What is being chosen and previewed.
struct Review {
    /// Tells this reading apart from any other, for the sheet.
    opening: u64,
    folder: PathBuf,
    library: Library,
    selection: Selection,
    mappings: Mappings,
    /// Matches symbols to markets.
    resolver: Lookup<TvSymbol, Match>,
    /// Finds the price of drawings that sit beside a bar.
    placer: Lookup<Spot, Option<f64>>,
    placements: Placements,
    step: Step,
    /// The names in use and the look of a new chart, as read when the preview was entered; kept
    /// while it is open, since it is rebuilt with every answer that arrives.
    surroundings: Option<(Taken, ChartAppearance)>,
    /// Built on entering the preview.
    plan: Option<Plan>,
    /// The "Match…" picker, with the symbol it answers.
    picking: Option<(TvSymbol, AssetPickerState)>,
}

enum Stage {
    Reading(Read),
    Failed { message: String, hint: &'static str },
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
    providers: Providers,
    stage: Stage,
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
    pub fn open(targets: Targets, ctx: &egui::Context) -> Self {
        let providers = Providers {
            search: Arc::new(crate::service::search),
            candles: Arc::new(tradingview::candles_around),
        };
        Self::with_providers(targets, providers, None, ctx)
    }

    pub fn with_providers(
        targets: Targets,
        providers: Providers,
        folder: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> Self {
        Self {
            targets,
            providers,
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
                    Ok(scan) => Stage::Review(Box::new(self.review(scan, ctx))),
                    Err(error) => Stage::Failed {
                        message: error.to_string(),
                        hint: error.hint(),
                    },
                };
            }
        }
        match &mut self.stage {
            Stage::Review(review) => {
                while let Ok((symbol, result)) = review.resolver.answers.try_recv() {
                    review.mappings.found(symbol, result);
                    review.plan = None;
                }
                let wanted = review.selection.symbols(&review.library);
                review
                    .resolver
                    .ask(wanted.into_iter().map(|(symbol, _)| symbol));
                while let Ok((spot, price)) = review.placer.answers.try_recv() {
                    review.placements.insert(spot, price);
                    review.plan = None;
                }
                let spots = review.selection.spots(&review.library, &review.mappings);
                review.placer.ask(spots);
            }
            Stage::Reading(_) | Stage::Failed { .. } | Stage::Done(_) => {}
        }
    }

    fn review(&mut self, scan: Scan, ctx: &egui::Context) -> Review {
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
            library: scan.library,
            mappings,
            resolver: {
                let search = Arc::clone(&self.providers.search);
                Lookup::start(
                    move |symbol| matcher::resolve(symbol, &*search),
                    ctx.clone(),
                )
            },
            placer: {
                let candles = Arc::clone(&self.providers.candles);
                Lookup::start(
                    move |spot: &Spot| candles(spot).and_then(|candles| spot.price(&candles)),
                    ctx.clone(),
                )
            },
            placements: Placements::new(),
            step: Step::Choose,
            surroundings: None,
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
            .map(|library| library.names_in_use().into_iter().collect())
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
        let read = match &self.stage {
            Stage::Review(review) if review.surroundings.is_none() => Some(self.surroundings()),
            _ => None,
        };
        if let Stage::Review(review) = &mut self.stage {
            if let Some(read) = read {
                review.surroundings = Some(read);
            }
            let Some((taken, base)) = &review.surroundings else {
                return;
            };
            review.plan = Some(plan::plan(
                &review.library,
                &review.selection,
                &review.mappings,
                &review.placements,
                taken,
                base,
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
            Stage::Reading(_) => View::Reading,
            Stage::Failed { message, hint } => View::Failed(message, hint),
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
            Action::Reload => self.reread(ctx),
            Action::OpenLayout(id) => return Outcome::OpenLayout(id),
            Action::Import => {
                // Planned again, against the names in use now.
                if let Stage::Review(review) = &mut self.stage {
                    review.surroundings = None;
                }
                self.build_plan();
                if let Stage::Review(review) = &self.stage
                    && let Some(plan) = &review.plan
                {
                    let summary = apply::apply(plan, &self.targets);
                    self.wrote = true;
                    self.added_scripts = summary.scripts > 0;
                    self.stage = Stage::Done(summary);
                }
            }
            Action::Preview => {
                if let Stage::Review(review) = &mut self.stage {
                    review.step = Step::Preview;
                    review.surroundings = None;
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
                let unplaced: Vec<Spot> = self
                    .placements
                    .iter()
                    .filter(|(_, price)| price.is_none())
                    .map(|(spot, _)| spot.clone())
                    .collect();
                for spot in &unplaced {
                    self.placements.remove(spot);
                }
                self.placer.forget(&unplaced);
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
