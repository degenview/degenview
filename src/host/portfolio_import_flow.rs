//! The portfolio's CSV import and export, end to end: pick a file, parse it, (for CoinMarketCap)
//! match tickers and fetch historical fee rates, preview, and commit to the ledger — plus saving
//! exports. All slow work (file dialogs, auto-matching exchanges, FX lookups) runs on worker
//! threads and reports back over channels; `show` drains them each frame and draws whichever
//! sheet is open. The pure rules live in `model::portfolio_csv` and the sheets' state types.

// Transient: reached through the Portfolio tab's Import or Export menu.
#![allow(dead_code)]

use crate::host::portfolio_host::PortfolioHost;
use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency, PortfolioTransaction};
use crate::model::portfolio_csv::{self, CmcPreview, CsvPreview, Mint};
use crate::model::watchlist::new_id;
use crate::service::asset_auto_mapper::{self, asset_from_search_result};
use crate::service::file_dialog;
use crate::service::portfolio_quotes::PortfolioFxRates;
use crate::ui::icons::Icon;
use crate::ui::portfolio_asset_picker::{self, AssetPickerOutcome, AssetPickerState};
use crate::ui::portfolio_cmc_import::{self, CmcImportAction, CmcImportInput, CmcImportState};
use crate::ui::portfolio_import_preview::{
    self, ImportPreviewInput, ImportPreviewOutcome, ImportPreviewState,
};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Align};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

pub const CSV_FILENAME: &str = "degenview-portfolio-transactions.csv";
pub const HOLDINGS_FILENAME: &str = "degenview-portfolio-holdings.csv";
pub const HISTORY_FILENAME: &str = "degenview-portfolio-history.csv";
pub const NEEDS_DESTINATION: &str =
    "Select the destination portfolio before importing CoinMarketCap transactions.";

/// Looks a bare ticker up on the exchanges.
pub type Resolver = Arc<dyn Fn(&str, PortfolioCurrency) -> Option<PortfolioAsset> + Send + Sync>;

type FileResult = Result<Option<String>, String>;

enum Job {
    /// A chosen and read CSV, for the app's own format.
    Csv(Receiver<FileResult>),
    /// A chosen and read CoinMarketCap export, with its destination.
    Cmc {
        inbox: Receiver<FileResult>,
        portfolio_id: u64,
        currency: PortfolioCurrency,
    },
    /// A save panel and write, with the saved path.
    Export(Receiver<Result<Option<std::path::PathBuf>, String>>),
}

/// An import of a CoinMarketCap export, from the ticker matching to the preview.
pub struct CmcFlow {
    pub preview: CmcPreview,
    pub state: CmcImportState,
    pub portfolio_id: u64,
    pub currency: PortfolioCurrency,
    /// Tickers still being searched.
    auto_inbox: Option<Receiver<AutoMapEvent>>,
    /// Fee rates still being fetched.
    fx_inbox: Option<Receiver<FxEvent>>,
    /// The "Match…" picker, with the ticker it answers.
    picking: Option<(String, AssetPickerState)>,
}

enum AutoMapEvent {
    Found(String, Option<PortfolioAsset>),
    Done,
}

enum FxEvent {
    Rate {
        row_id: String,
        result: Result<f64, String>,
        progress: f32,
    },
    Done,
}

impl CmcFlow {
    /// Starts the work: reuse what the ledger already holds, then search the rest and fetch fee
    /// rates in the background.
    pub fn start(
        preview: CmcPreview,
        ledger: &[PortfolioTransaction],
        portfolio_id: u64,
        currency: PortfolioCurrency,
        fx: Arc<dyn PortfolioFxRates>,
        resolver: Resolver,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let mut state = CmcImportState::default();
        state.apply_existing_mappings(&preview, ledger);

        let symbols = state.symbols_to_auto_map(&preview);
        state.resolving.extend(symbols.iter().cloned());
        let auto_inbox = if symbols.is_empty() {
            state.did_auto_map = true;
            None
        } else {
            let (sender, receiver) = channel();
            let repaint = Arc::clone(&repaint);
            std::thread::spawn(move || {
                for symbol in symbols {
                    let found = resolver(&symbol, currency);
                    let _ = sender.send(AutoMapEvent::Found(symbol, found));
                    repaint();
                }
                let _ = sender.send(AutoMapEvent::Done);
                repaint();
            });
            Some(receiver)
        };

        let rows: Vec<(String, PortfolioCurrency, i64)> = state
            .fee_rows_to_resolve(&preview)
            .into_iter()
            .filter_map(|row| Some((row.id.clone(), row.fee_currency?, row.timestamp)))
            .collect();
        let fx_inbox = if rows.is_empty() {
            None
        } else {
            state.is_resolving_fee_fx = true;
            let (sender, receiver) = channel();
            std::thread::spawn(move || {
                let total = rows.len();
                for (index, (row_id, fee_currency, timestamp)) in rows.into_iter().enumerate() {
                    let result = fx
                        .conversion(fee_currency, PortfolioCurrency::Usd, Some(timestamp))
                        .map(|conversion| conversion.rate);
                    let _ = sender.send(FxEvent::Rate {
                        row_id,
                        result,
                        progress: (index + 1) as f32 / total as f32,
                    });
                    repaint();
                }
                let _ = sender.send(FxEvent::Done);
                repaint();
            });
            Some(receiver)
        };

        Self {
            preview,
            state,
            portfolio_id,
            currency,
            auto_inbox,
            fx_inbox,
            picking: None,
        }
    }

    /// Applies whatever the workers have reported.
    pub fn poll(&mut self) {
        if let Some(inbox) = &self.auto_inbox {
            loop {
                match inbox.try_recv() {
                    Ok(AutoMapEvent::Found(symbol, asset)) => {
                        self.state.resolving.remove(&symbol);
                        if let Some(asset) = asset {
                            self.state.auto_mapped(&symbol, asset);
                        }
                    }
                    Ok(AutoMapEvent::Done) | Err(TryRecvError::Disconnected) => {
                        self.state.resolving.clear();
                        self.state.did_auto_map = true;
                        self.auto_inbox = None;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
        }
        if let Some(inbox) = &self.fx_inbox {
            loop {
                match inbox.try_recv() {
                    Ok(FxEvent::Rate {
                        row_id,
                        result,
                        progress,
                    }) => {
                        self.state.fee_rate_resolved(&row_id, result);
                        self.state.fee_fx_progress = progress;
                    }
                    Ok(FxEvent::Done) | Err(TryRecvError::Disconnected) => {
                        self.state.is_resolving_fee_fx = false;
                        self.fx_inbox = None;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.auto_inbox.is_some() || self.fx_inbox.is_some()
    }
}

/// What the flow wants the tab to know.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowEvent {
    /// Transactions were committed: refresh quotes and rebuild history.
    Imported(usize),
}

pub struct PortfolioImportFlow {
    fx: Arc<dyn PortfolioFxRates>,
    resolver: Resolver,
    jobs: Vec<Job>,
    pub csv: Option<(CsvPreview, ImportPreviewState)>,
    pub cmc: Option<CmcFlow>,
    /// A problem to show in a small alert.
    pub notice: Option<String>,
}

impl PortfolioImportFlow {
    pub fn new(fx: Arc<dyn PortfolioFxRates>) -> Self {
        Self::with_resolver(fx, Arc::new(asset_auto_mapper::resolve_live))
    }

    pub fn with_resolver(fx: Arc<dyn PortfolioFxRates>, resolver: Resolver) -> Self {
        Self {
            fx,
            resolver,
            jobs: Vec::new(),
            csv: None,
            cmc: None,
            notice: None,
        }
    }

    pub fn is_active(&self) -> bool {
        !self.jobs.is_empty() || self.csv.is_some() || self.cmc.is_some()
    }

    // MARK: Starting

    fn read_file_job(ctx: &egui::Context, prompt: &'static str) -> Receiver<FileResult> {
        let (sender, receiver) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = match file_dialog::choose_csv(prompt) {
                Ok(Some(path)) => file_dialog::read_text(&path).map(Some),
                Ok(None) => Ok(None),
                Err(message) => Err(message),
            };
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        receiver
    }

    pub fn begin_import_csv(&mut self, ctx: &egui::Context) {
        self.jobs.push(Job::Csv(Self::read_file_job(
            ctx,
            "Choose a transactions CSV to import",
        )));
    }

    /// Picks a CoinMarketCap export for `destination` (the selected portfolio and its currency).
    pub fn begin_import_cmc(
        &mut self,
        ctx: &egui::Context,
        destination: Option<(u64, PortfolioCurrency)>,
    ) {
        let Some((portfolio_id, currency)) = destination else {
            self.notice = Some(NEEDS_DESTINATION.to_owned());
            return;
        };
        self.jobs.push(Job::Cmc {
            inbox: Self::read_file_job(ctx, "Choose a CoinMarketCap transactions export"),
            portfolio_id,
            currency,
        });
    }

    /// Asks where to save `text` and writes it.
    pub fn begin_export(&mut self, ctx: &egui::Context, text: String, default_name: &str) {
        let (sender, receiver) = channel();
        let ctx = ctx.clone();
        let default_name = default_name.to_owned();
        std::thread::spawn(move || {
            let outcome = match file_dialog::choose_save_path("Export", &default_name) {
                Ok(Some(path)) => file_dialog::write_text(&path, &text).map(|()| Some(path)),
                Ok(None) => Ok(None),
                Err(message) => Err(message),
            };
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        self.jobs.push(Job::Export(receiver));
    }

    // MARK: Per frame

    /// Finished file jobs: parse into previews, start the CoinMarketCap matching, report errors.
    fn poll_jobs(&mut self, ctx: &egui::Context, host: &PortfolioHost) {
        let mut remaining = Vec::new();
        for job in std::mem::take(&mut self.jobs) {
            match job {
                Job::Csv(inbox) => match inbox.try_recv() {
                    Ok(Ok(Some(text))) => {
                        let mut next = new_id;
                        let mut mint = Mint {
                            now_ms: crate::model::watchlist::now_ms(),
                            next_id: &mut next,
                        };
                        let preview =
                            portfolio_csv::preview(&text, &host.snapshot().portfolios, &mut mint);
                        self.csv = Some((preview, ImportPreviewState::default()));
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(message)) => self.notice = Some(message),
                    Err(TryRecvError::Empty) => remaining.push(Job::Csv(inbox)),
                    Err(TryRecvError::Disconnected) => {}
                },
                Job::Cmc {
                    inbox,
                    portfolio_id,
                    currency,
                } => match inbox.try_recv() {
                    Ok(Ok(Some(text))) => {
                        let preview = portfolio_csv::preview_coinmarketcap(&text);
                        let repaint_ctx = ctx.clone();
                        self.cmc = Some(CmcFlow::start(
                            preview,
                            &host.snapshot().transactions,
                            portfolio_id,
                            currency,
                            Arc::clone(&self.fx),
                            Arc::clone(&self.resolver),
                            Arc::new(move || repaint_ctx.request_repaint()),
                        ));
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(message)) => self.notice = Some(message),
                    Err(TryRecvError::Empty) => remaining.push(Job::Cmc {
                        inbox,
                        portfolio_id,
                        currency,
                    }),
                    Err(TryRecvError::Disconnected) => {}
                },
                Job::Export(inbox) => match inbox.try_recv() {
                    Ok(Ok(_)) => {}
                    Ok(Err(message)) => self.notice = Some(message),
                    Err(TryRecvError::Empty) => remaining.push(Job::Export(inbox)),
                    Err(TryRecvError::Disconnected) => {}
                },
            }
        }
        self.jobs = remaining;
    }

    /// Drives the flow and draws its open sheet. Returns what happened, if anything.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        host: &mut PortfolioHost,
        utc_offset_secs: i64,
    ) -> Option<FlowEvent> {
        self.poll_jobs(ctx, host);
        if !self.jobs.is_empty() || self.cmc.as_ref().is_some_and(CmcFlow::is_busy) {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        self.show_cmc(ctx, host);
        let event = self.show_preview(ctx, host, utc_offset_secs);
        self.show_notice(ctx);
        event
    }

    fn show_cmc(&mut self, ctx: &egui::Context, host: &PortfolioHost) {
        let Some(flow) = &mut self.cmc else {
            return;
        };
        flow.poll();
        // The "Match…" picker sits on top of the sheet.
        if let Some((symbol, picker)) = &mut flow.picking {
            match portfolio_asset_picker::show(ctx, picker) {
                Some(AssetPickerOutcome::Choose(result)) => {
                    let symbol = symbol.clone();
                    flow.state
                        .set_mapping(&symbol, asset_from_search_result(&result));
                    flow.picking = None;
                }
                Some(AssetPickerOutcome::Cancel) => flow.picking = None,
                None => {}
            }
            return;
        }
        let actions = portfolio_cmc_import::show(
            ctx,
            &mut flow.state,
            &CmcImportInput {
                preview: &flow.preview,
                portfolio_currency: flow.currency,
            },
        );
        let _ = host;
        for action in actions {
            match action {
                CmcImportAction::Match(symbol) => {
                    flow.picking = Some((
                        symbol.clone(),
                        AssetPickerState::new(
                            &format!("Match {symbol}"),
                            "Pick the market that matches this CoinMarketCap ticker.",
                            "Use Asset",
                        ),
                    ));
                }
                CmcImportAction::Continue => {
                    let mut next = new_id;
                    let mut mint = Mint {
                        now_ms: crate::model::watchlist::now_ms(),
                        next_id: &mut next,
                    };
                    let converted = flow
                        .state
                        .result(&flow.preview, flow.portfolio_id, &mut mint);
                    self.cmc = None;
                    self.csv = Some((converted, ImportPreviewState::default()));
                    return;
                }
                CmcImportAction::Cancel => {
                    self.cmc = None;
                    return;
                }
            }
        }
    }

    fn show_preview(
        &mut self,
        ctx: &egui::Context,
        host: &mut PortfolioHost,
        utc_offset_secs: i64,
    ) -> Option<FlowEvent> {
        let (preview, state) = self.csv.as_mut()?;
        let outcome = portfolio_import_preview::show(
            ctx,
            state,
            &ImportPreviewInput {
                preview,
                utc_offset_secs,
            },
        );
        match outcome {
            Some(ImportPreviewOutcome::Import) => {
                let transactions = preview.transactions.clone();
                let count = transactions.len();
                match host.import_transactions(transactions) {
                    Ok(()) => {
                        self.csv = None;
                        return Some(FlowEvent::Imported(count));
                    }
                    Err(error) => {
                        host.clear_error();
                        state.failed(error.message());
                    }
                }
            }
            Some(ImportPreviewOutcome::Cancel) => self.csv = None,
            None => {}
        }
        None
    }

    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some(message) = self.notice.clone() else {
            return;
        };
        sheet::show(ctx, egui::Id::new("portfolio-import-notice"), 380.0, |ui| {
            ui.add_space(20.0);
            sheet::inset(ui, |ui| {
                sheet::header(
                    ui,
                    Icon::Triangle,
                    "Import or Export",
                    &message,
                    theme::ORANGE,
                );
                ui.add_space(20.0);
            });
            ui.separator();
            sheet::inset(ui, |ui| {
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        self.notice = None;
                    }
                });
                ui.add_space(12.0);
            });
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.notice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::service::fx_rates::Conversion;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    struct FakeFx;

    impl PortfolioFxRates for FakeFx {
        fn conversion(
            &self,
            from: PortfolioCurrency,
            to: PortfolioCurrency,
            on: Option<i64>,
        ) -> Result<Conversion, String> {
            match from {
                PortfolioCurrency::Eur => Ok(Conversion {
                    rate: 1.1,
                    observation_ms: on.unwrap_or(0),
                    is_cached: false,
                }),
                _ => Err(format!("no {} → {} rate", from.code(), to.code())),
            }
        }

        fn conversions(
            &self,
            _: PortfolioCurrency,
            _: PortfolioCurrency,
            _: &[i64],
        ) -> HashMap<i64, Result<Conversion, String>> {
            HashMap::new()
        }
    }

    fn resolver() -> Resolver {
        Arc::new(|symbol, _| {
            (symbol == "BTC").then(|| PortfolioAsset {
                key: "Binance:BTCUSDT".into(),
                symbol: "BTC/USDT".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            })
        })
    }

    const CMC: &str = "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
        2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,--,,\n\
        2024-03-06 10:00:00,BTC,Buy,61000,1,61000,2,EUR,\n\
        2024-03-07 10:00:00,DOGE,Buy,0.1,100,10,1,CHF,";

    fn start(csv: &str) -> CmcFlow {
        CmcFlow::start(
            portfolio_csv::preview_coinmarketcap(csv),
            &[],
            4,
            PortfolioCurrency::Usd,
            Arc::new(FakeFx),
            resolver(),
            Arc::new(|| {}),
        )
    }

    fn settle(flow: &mut CmcFlow) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while flow.is_busy() && Instant::now() < deadline {
            flow.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        flow.poll();
    }

    #[test]
    fn tickers_are_matched_and_fee_rates_fetched_in_the_background() {
        let mut flow = start(CMC);
        assert!(flow.state.resolving.contains("BTC") && flow.state.resolving.contains("DOGE"));
        assert!(flow.state.is_resolving_fee_fx);
        assert!(!flow.state.did_auto_map);
        settle(&mut flow);
        assert!(flow.state.did_auto_map && flow.state.resolving.is_empty());
        assert!(!flow.state.is_resolving_fee_fx);
        assert_eq!(flow.state.mappings["BTC"].key, "Binance:BTCUSDT");
        assert!(flow.state.auto_mapped.contains("BTC"));
        assert!(
            !flow.state.mappings.contains_key("DOGE"),
            "not found: left for the user"
        );
        let eur_row = flow
            .preview
            .rows
            .iter()
            .find(|row| row.fee_currency == Some(PortfolioCurrency::Eur))
            .unwrap();
        assert_eq!(flow.state.fee_fx_rates[&eur_row.id], 1.1);
        let chf_row = flow
            .preview
            .rows
            .iter()
            .find(|row| row.fee_currency == Some(PortfolioCurrency::Chf))
            .unwrap();
        assert_eq!(flow.state.fee_fx_errors[&chf_row.id], "no CHF → USD rate");
        assert_eq!(flow.state.fee_fx_progress, 1.0);
        assert!(
            flow.state.can_continue(&flow.preview),
            "the mapped rows are ready"
        );
    }

    #[test]
    fn nothing_to_search_or_convert_is_ready_at_once() {
        let flow = start(
            "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
             2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,0,,",
        );
        assert!(
            flow.state.resolving.contains("BTC"),
            "BTC still has to be searched"
        );
        let held = start("Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes");
        assert!(held.state.did_auto_map && !held.is_busy());
    }

    #[test]
    fn a_continue_converts_the_mapped_rows_into_a_previewable_import() {
        let mut flow = start(CMC);
        settle(&mut flow);
        let mut next = 1;
        let mut ids = || {
            next += 1;
            next
        };
        let converted = flow.state.result(
            &flow.preview,
            flow.portfolio_id,
            &mut Mint {
                now_ms: 9,
                next_id: &mut ids,
            },
        );
        // BTC's two rows import; the EUR fee converts at 1.1. DOGE is unmapped and skipped.
        assert!(converted.is_valid(), "{:?}", converted.errors);
        assert_eq!(converted.transactions.len(), 2);
        let with_fee = converted.transactions.iter().find(|t| t.fee > 0.0).unwrap();
        assert!((with_fee.fee - 2.2).abs() < 1e-9);
        assert!(converted.warnings.iter().any(|w| w.contains("DOGE")));
    }

    #[test]
    fn a_cmc_import_needs_a_destination_portfolio() {
        let mut flow = PortfolioImportFlow::with_resolver(Arc::new(FakeFx), resolver());
        flow.begin_import_cmc(&egui::Context::default(), None);
        assert_eq!(flow.notice.as_deref(), Some(NEEDS_DESTINATION));
        assert!(!flow.is_active(), "no dialog opened");
    }
}
