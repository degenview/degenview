//! Pine alert-subscription runtime host: loads applied subscriptions and feeds each active one's
//! persistent [`ExecutionController`] the chart's latest candles, returning the alerts raised by
//! realtime ticks and closes. Webhook/notification dispatch is the caller's (the app), which
//! already owns the webhook + notification services.
//!
//! Whether a script can run (it compiles, and does not stop with a runtime error) is a fact about
//! this build of the app, the libraries it imports and the bars it is fed, not about the
//! subscription, so it is kept here with the run and never stored: an update that accepts the
//! script again, or a replay that gets through, brings the alert back by itself.

use crate::host::pine::KlineSecurityDataProvider;
use crate::model::alert::pine::{PineAlertState, PineAlertSubscription, ScriptFailure};
use crate::model::chart::synthetic::Construction;
use crate::model::market::bucket::Grid;
use crate::model::pine::instance::PineInstance;
use crate::model::{Kline, Timeframe};
use crate::pine::Diagnostic;
use crate::pine::compiler::program::SymbolInfo;
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::runtime::state::AlertEvent;
use crate::pine::syntax::token::Limits;
use crate::pine::time::bucket::seconds_of;
use crate::store::Store;
use crate::store::scripts::registry::LibraryRegistry;
use std::collections::HashMap;

/// What a running controller was built from. Any difference means the controller is stale and
/// must be rebuilt (without notifying for the history it replays).
#[derive(PartialEq)]
struct RunKey {
    chart_id: u64,
    instance_id: Option<u64>,
    symbol_key: String,
    timeframe: String,
    source_hash: String,
    inputs: HashMap<String, crate::model::pine::instance::StoredInput>,
    /// The chart's constructed bars the script runs over, if it is price-based: a change of
    /// chart type or of its construction starts the script over (silently), like any other change
    /// of what it runs on.
    chart: ChartRun,
}

/// What the chart a script alert runs on feeds it. A price-based chart gives the script its
/// constructed bars (`bars` are those, and `bar_seconds` is the market candles' length, which
/// their times cannot tell); every other chart type gives the market's own candles. Either way
/// `timeframe_seconds` is the length of the timeframe the chart is set to. Price alerts
/// are not affected: they read the market's quotes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartRun {
    pub construction: Option<Construction>,
    pub bar_seconds: Option<f64>,
    pub timeframe_seconds: Option<f64>,
}

impl ChartRun {
    /// What a script alert on a chart drawn with `construction` runs over: the bars it builds
    /// from `candles` and how they were made, or the candles themselves for a chart of candles.
    pub fn feed(
        construction: Option<Construction>,
        candles: &[Kline],
        timeframe: Timeframe,
        now_ms: i64,
    ) -> (Vec<Kline>, Self) {
        let timeframe_seconds = Some(seconds_of(timeframe));
        let Some(construction) = construction else {
            return (
                candles.to_vec(),
                Self {
                    timeframe_seconds,
                    ..Self::default()
                },
            );
        };
        let bars =
            crate::model::chart::synthetic::build_with(construction, candles, now_ms).klines();
        (
            bars,
            Self {
                construction: Some(construction),
                bar_seconds: timeframe_seconds,
                timeframe_seconds,
            },
        )
    }

    fn symbol(&self, symbol_key: &str) -> SymbolInfo {
        SymbolInfo {
            chart: self
                .construction
                .map_or(crate::model::ChartKind::Standard, |construction| {
                    construction.chart_type().kind()
                }),
            ..symbol_info(symbol_key)
        }
    }
}

struct Running {
    key: RunKey,
    /// The controller, or why the source does not compile: the alert cannot fire then, and its
    /// status says so. Compiled again when the key changes or the alert is armed anew. A
    /// controller keeps the runtime error its script stopped with, until a replay gets through.
    controller: Result<ExecutionController, Diagnostic>,
}

pub struct PineSubscriptionHost {
    subscriptions: Vec<PineAlertSubscription>,
    running: HashMap<u64, Running>,
}

impl PineSubscriptionHost {
    pub fn load(store: &Store) -> Self {
        Self {
            subscriptions: store.pine_subscriptions(),
            running: HashMap::new(),
        }
    }

    /// Re-reads the subscriptions after the store changed (created, paused, deleted …).
    pub fn reload(&mut self, store: &Store) {
        self.subscriptions = store.pine_subscriptions();
        self.drop_stopped_runs();
    }

    /// Why a subscription's script cannot run, once its chart has fed it. `None` while it runs,
    /// and before its chart has shown the market it watches.
    pub fn failure(&self, subscription_id: u64) -> Option<ScriptFailure> {
        match &self.running.get(&subscription_id)?.controller {
            Err(error) => Some(ScriptFailure::Compile(error.clone())),
            Ok(controller) => controller.error().cloned().map(ScriptFailure::Runtime),
        }
    }

    /// Forgets what was built for a subscription that is gone or switched off, so arming it
    /// again starts from a fresh compile.
    fn drop_stopped_runs(&mut self) {
        let subscriptions = &self.subscriptions;
        self.running.retain(|id, _| {
            subscriptions
                .iter()
                .any(|subscription| subscription.id == *id && subscription.is_active())
        });
    }

    #[cfg(test)]
    pub fn subscriptions(&self) -> &[PineAlertSubscription] {
        &self.subscriptions
    }

    /// Pins active subscriptions to the applied instance that created them. A removed instance
    /// or changed source becomes silent until explicitly re-armed. Current instance inputs are
    /// persisted so the independent host never falls back to input defaults.
    pub fn reconcile_chart(&mut self, chart_id: u64, instances: &[PineInstance], store: &Store) {
        let mut changed = false;
        for subscription in self
            .subscriptions
            .iter_mut()
            .filter(|subscription| subscription.chart_id == chart_id && subscription.is_active())
        {
            let Some(instance_id) = subscription.instance_id else {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            };
            let Some(instance) = instances.iter().find(|instance| instance.id == instance_id)
            else {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            };
            let current_hash = crate::store::scripts::hash_source(&instance.source);
            if subscription.source_hash != current_hash {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            }
            if subscription.inputs != instance.inputs {
                subscription.inputs = instance.inputs.clone();
                changed = true;
            }
            if subscription.script_id != instance.script_id {
                subscription.script_id = instance.script_id.clone();
                changed = true;
            }
        }
        if changed {
            let _ = store.replace_pine_subscriptions(&self.subscriptions);
        }
    }

    /// [`Self::run_active_on`] for a time-based chart.
    #[cfg(test)]
    pub fn run_active(
        &mut self,
        chart_id: u64,
        symbol_key: &str,
        timeframe: &str,
        bars: &[Kline],
        instances: &[PineInstance],
        now_ms: i64,
    ) -> Vec<(PineAlertSubscription, AlertEvent)> {
        self.run_active_on(
            chart_id,
            symbol_key,
            timeframe,
            bars,
            &ChartRun::default(),
            instances,
            now_ms,
        )
    }

    /// Feeds `bars` to every active subscription that watches `symbol_key`, returning
    /// `(subscription, alert)` pairs for the alerts raised by the new realtime tick or close.
    /// Each subscription owns one persistent controller, so a forming bar is re-evaluated as a
    /// tick (`freq_all`, `freq_once_per_bar`) and `freq_once_per_bar_close` waits for the real
    /// close. The first snapshot, and any rebuild after the market, source or inputs changed,
    /// replays history silently. A subscription without a stored source is skipped. The chart
    /// may be price-based: `bars` are the bars the chart's scripts run over, and `chart` says how
    /// they were made.
    #[allow(clippy::too_many_arguments)]
    pub fn run_active_on(
        &mut self,
        chart_id: u64,
        symbol_key: &str,
        timeframe: &str,
        bars: &[Kline],
        chart: &ChartRun,
        instances: &[PineInstance],
        now_ms: i64,
    ) -> Vec<(PineAlertSubscription, AlertEvent)> {
        let symbol = chart.symbol(symbol_key);
        let mut fired = Vec::new();
        self.drop_stopped_runs();
        for subscription in &self.subscriptions {
            if !subscription.is_active()
                || subscription.chart_id != chart_id
                || subscription.symbol_key != symbol_key
                || subscription.timeframe != timeframe
                || subscription.source.is_empty()
                || !subscription.instance_id.is_some_and(|instance_id| {
                    instances.iter().any(|instance| {
                        instance.id == instance_id
                            && crate::store::scripts::hash_source(&instance.source)
                                == subscription.source_hash
                    })
                })
            {
                continue;
            }
            let key = RunKey {
                chart_id,
                instance_id: subscription.instance_id,
                symbol_key: symbol_key.to_owned(),
                timeframe: timeframe.to_owned(),
                source_hash: subscription.source_hash.clone(),
                inputs: subscription.inputs.clone(),
                chart: chart.clone(),
            };
            if self
                .running
                .get(&subscription.id)
                .is_none_or(|running| running.key != key)
            {
                let controller = ExecutionController::new(
                    &subscription.source,
                    subscription.input_values(),
                    symbol.clone(),
                    None,
                    Limits::default(),
                    &LibraryRegistry,
                    Some(Box::new(KlineSecurityDataProvider)),
                    None,
                )
                .map(ExecutionController::with_alert_mode)
                .map(|controller| match chart.timeframe_seconds {
                    Some(seconds) => controller.with_timeframe_seconds(seconds),
                    None => controller,
                })
                .map(|controller| match chart.bar_seconds {
                    Some(seconds) => controller.with_bar_seconds(seconds),
                    None => controller,
                });
                self.running
                    .insert(subscription.id, Running { key, controller });
            }
            let Some(controller) = self
                .running
                .get_mut(&subscription.id)
                .and_then(|running| running.controller.as_mut().ok())
            else {
                continue;
            };
            let update = controller.update(bars, None, now_ms);
            if update.rebuilt {
                continue;
            }
            fired.extend(
                update
                    .alerts
                    .into_iter()
                    .map(|alert| (subscription.clone(), alert)),
            );
        }
        fired
    }
}

/// Builds the `SymbolInfo` a subscription's `"<source>:<ticker>"` key implies. Ticker id is the
/// full key; the base currency is left `None` (the runtime treats it as a non-pair).
pub fn symbol_info(symbol_key: &str) -> SymbolInfo {
    let ticker = symbol_key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .unwrap_or(symbol_key);
    let source = symbol_key
        .split_once(':')
        .and_then(|(prefix, _)| crate::model::market::venue::source_for_prefix(prefix));
    SymbolInfo {
        ticker: ticker.to_owned(),
        ticker_id: symbol_key.to_owned(),
        currency: "USD".to_owned(),
        kind: "crypto".to_owned(),
        base_currency: None,
        grid: source.map_or(Grid::Utc, |source| source.grid()),
        ..SymbolInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::pine::PineAlertState;

    fn bars() -> Vec<Kline> {
        (0..5)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1_000.0,
            })
            .collect()
    }

    fn subscription(id: u64, source: &str) -> PineAlertSubscription {
        PineAlertSubscription {
            id,
            chart_id: 1,
            instance_id: Some(10),
            script_id: None,
            script_name: "S".into(),
            source: source.into(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: crate::store::scripts::hash_source(source),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: Vec::new(),
        }
    }

    fn instance(source: &str) -> PineInstance {
        PineInstance::new(10, Some("saved-script".into()), "S", source)
    }

    const LIVE_NOW: i64 = 17_000_000;

    #[test]
    fn active_alert_fires_on_realtime_updates_not_the_initial_snapshot() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source), subscription(2, "")])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let applied = [instance(source)];
        let mut data = bars();
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW)
                .is_empty()
        );

        data[4].close += 1.0;
        let fired = host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + 1);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0.id, 1);
        assert_eq!(fired[0].1.message, "hi");

        assert!(
            host.run_active(2, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW)
                .is_empty()
        );
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1d", &data, &applied, LIVE_NOW)
                .is_empty()
        );
    }

    #[test]
    fn close_frequency_waits_for_the_real_bar_close() {
        let source =
            "//@version=6\nindicator(\"S\")\nalert(\"done\", alert.freq_once_per_bar_close)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let applied = [instance(source)];
        let mut data = bars();
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        data[4].close += 1.0;
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + 1)
                .is_empty()
        );
        let fired = host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, 18_000_000);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].1.message, "done");
    }

    #[test]
    fn a_paused_or_other_symbol_subscription_does_not_fire() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"x\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        let mut paused = subscription(1, source);
        paused.state = PineAlertState::Paused;
        store.replace_pine_subscriptions(&[paused]).expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let data = bars();
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &[], LIVE_NOW)
                .is_empty()
        );
        assert!(
            host.run_active(1, "binance:ETHUSDT", "1h", &data, &[], LIVE_NOW)
                .is_empty()
        );
    }

    /// Uses a name nothing declares, which no build compiles.
    const UNCOMPILABLE: &str =
        "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)\nplot(nope)";

    #[test]
    fn a_script_that_does_not_compile_says_why_and_stays_armed() {
        let strategy = "//@version=6\nstrategy(\"S\", calc_on_every_tick = true)\n\
            alert(\"hi\", alert.freq_all)\nplot(nope)";
        for source in [UNCOMPILABLE, strategy] {
            let store = Store::open_in_memory().expect("open");
            store
                .replace_pine_subscriptions(&[subscription(1, source)])
                .expect("write");
            let mut host = PineSubscriptionHost::load(&store);
            assert_eq!(host.failure(1), None, "nothing has compiled it yet");
            let applied = [instance(source)];
            let mut data = bars();
            for tick in 0..3 {
                data[4].close += 1.0;
                let now = LIVE_NOW + tick;
                host.reconcile_chart(1, &applied, &store);
                assert!(
                    host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, now)
                        .is_empty()
                );
            }
            let Some(ScriptFailure::Compile(error)) = host.failure(1) else {
                panic!("a compile failure: {:?}", host.failure(1));
            };
            assert!(error.message.contains("nope"), "{}", error.message);
            assert_eq!(error.range.start.line, 4);
            // The stored alert is untouched: a build that accepts the script runs it again.
            assert_eq!(store.pine_subscriptions()[0].state, PineAlertState::Active);
        }
    }

    #[test]
    fn pausing_forgets_a_compile_error_and_arming_again_compiles_anew() {
        let store = Store::open_in_memory().expect("open");
        let broken = subscription(1, UNCOMPILABLE);
        store
            .replace_pine_subscriptions(std::slice::from_ref(&broken))
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let data = bars();
        let applied = [instance(UNCOMPILABLE)];
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        assert!(host.failure(1).is_some());

        // An edit that leaves it armed keeps what is known.
        let mut noted = broken.clone();
        noted.note = "Breakout".into();
        store.replace_pine_subscriptions(&[noted]).expect("write");
        host.reload(&store);
        assert!(host.failure(1).is_some());

        let mut paused = broken.clone();
        paused.state = PineAlertState::Paused;
        store.replace_pine_subscriptions(&[paused]).expect("write");
        host.reload(&store);
        assert!(host.failure(1).is_none());

        store.replace_pine_subscriptions(&[broken]).expect("write");
        host.reload(&store);
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        assert!(host.failure(1).is_some());
    }

    /// The way a user fixes a script: the corrected one goes on the chart as a new instance,
    /// with a new alert.
    #[test]
    fn a_fixed_script_fires_and_the_broken_alert_reads_as_changed() {
        let fixed = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, UNCOMPILABLE)])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let mut data = bars();
        let applied = [instance(UNCOMPILABLE)];
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        assert!(host.failure(1).is_some());

        let mut rearmed = subscription(2, fixed);
        rearmed.instance_id = Some(11);
        let mut subscriptions = store.pine_subscriptions();
        subscriptions.push(rearmed);
        store
            .replace_pine_subscriptions(&subscriptions)
            .expect("write");
        host.reload(&store);
        let applied = [PineInstance::new(
            11,
            Some("saved-script".into()),
            "S",
            fixed,
        )];
        host.reconcile_chart(1, &applied, &store);
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        data[4].close += 1.0;
        let fired = host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + 1);

        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0.id, 2);
        assert!(host.failure(2).is_none());
        assert!(host.failure(1).is_none());
        assert_eq!(
            store.pine_subscriptions()[0].state,
            PineAlertState::ScriptChanged
        );
    }

    #[test]
    fn reload_picks_up_a_subscription_added_after_load() {
        let store = Store::open_in_memory().expect("open");
        let mut host = PineSubscriptionHost::load(&store);
        assert!(host.subscriptions().is_empty());
        store
            .replace_pine_subscriptions(&[subscription(1, "plot(close)")])
            .expect("write");
        host.reload(&store);
        assert_eq!(host.subscriptions().len(), 1);
    }

    #[test]
    fn reconciliation_persists_inputs_and_silences_changed_or_removed_instances() {
        use crate::model::pine::instance::StoredInput;

        let source = "//@version=6\nindicator(\"S\")\nlen = input.int(2)\nplot(len)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut applied = instance(source);
        applied.inputs.insert("len".into(), StoredInput::Int(9));
        let mut host = PineSubscriptionHost::load(&store);
        host.reconcile_chart(1, &[applied.clone()], &store);
        assert_eq!(
            store.pine_subscriptions()[0].inputs.get("len"),
            Some(&StoredInput::Int(9))
        );

        applied.source.push_str("\nplot(open)");
        host.reconcile_chart(1, &[applied], &store);
        assert_eq!(
            store.pine_subscriptions()[0].state,
            PineAlertState::ScriptChanged
        );
    }

    #[test]
    fn script_alerts_follow_the_constructed_bars_and_a_chart_type_change_starts_over_silently() {
        use crate::model::chart::kind::{ConstructionSource, SizeMethod, SizeRule};
        use crate::model::chart::synthetic::{Construction, build_with};

        let source = "//@version=6\nindicator(\"S\")\nif high - low > 1.5\n    alert(\"wide\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let applied = [instance(source)];
        // Candles two points wide, the last one still forming.
        let mut candles: Vec<Kline> = (0..5)
            .map(|i| {
                let close = 100.0 + 3.0 * f64::from(i);
                Kline {
                    open_time: i64::from(i) * 3_600_000,
                    close_time: (i64::from(i) + 1) * 3_600_000,
                    open: close - 1.0,
                    high: close + 1.0,
                    low: close - 1.0,
                    close,
                    volume: 1.0,
                }
            })
            .collect();

        let plain = ChartRun::default();
        host.run_active_on(
            1,
            "binance:BTCUSDT",
            "1h",
            &candles,
            &plain,
            &applied,
            LIVE_NOW,
        );
        candles[4].close += 0.5;
        let fired = host.run_active_on(
            1,
            "binance:BTCUSDT",
            "1h",
            &candles,
            &plain,
            &applied,
            LIVE_NOW + 1,
        );
        assert_eq!(fired.len(), 1, "a wide candle raises the alert");

        // The same chart switched to Renko: bricks are one point tall, so the script's condition
        // never holds. The switch itself replays history without alerting.
        let construction = Construction::Renko {
            size: SizeRule {
                method: SizeMethod::Traditional,
                amount: 1.0,
                ..SizeRule::default()
            },
            source: ConstructionSource::Close,
            wicks: false,
        };
        let chart = ChartRun {
            construction: Some(construction),
            bar_seconds: Some(3_600.0),
            timeframe_seconds: Some(3_600.0),
        };
        let bricks = build_with(construction, &candles, LIVE_NOW + 2).klines();
        assert!(bricks.len() > candles.len());
        assert!(
            host.run_active_on(
                1,
                "binance:BTCUSDT",
                "1h",
                &bricks,
                &chart,
                &applied,
                LIVE_NOW + 2
            )
            .is_empty()
        );
        candles[4].close += 2.0;
        let bricks = build_with(construction, &candles, LIVE_NOW + 3).klines();
        assert!(
            host.run_active_on(
                1,
                "binance:BTCUSDT",
                "1h",
                &bricks,
                &chart,
                &applied,
                LIVE_NOW + 3
            )
            .is_empty(),
            "no brick is wide"
        );
        // Back on candles: another silent restart, then the candles' own alerts again.
        assert!(
            host.run_active_on(
                1,
                "binance:BTCUSDT",
                "1h",
                &candles,
                &plain,
                &applied,
                LIVE_NOW + 4
            )
            .is_empty()
        );
        candles[4].close += 0.5;
        assert_eq!(
            host.run_active_on(
                1,
                "binance:BTCUSDT",
                "1h",
                &candles,
                &plain,
                &applied,
                LIVE_NOW + 5
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_subscriptions_symbol_carries_the_grid_of_its_market() {
        // A stock (by its exchange or by Alpaca's slug) is cut on the session, a crypto pair on the
        // UTC day, a key with no market the UTC day too.
        for key in ["NASDAQ:AAPL", "ALPACA:AAPL", "NYSE:IBM"] {
            assert_eq!(symbol_info(key).grid, Grid::UsEquities, "{key}");
        }
        for key in ["BINANCE:BTCUSDT", "COINBASE:ETH-USD", "BTCUSDT"] {
            assert_eq!(symbol_info(key).grid, Grid::Utc, "{key}");
        }
    }

    /// Fine on history, then stops on a live tick that closes above 102.5.
    const FAILS_ON_A_TICK: &str = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)\n\
        if barstate.isrealtime and close > 102.5\n    runtime.error(\"late\")";

    fn runtime_message(host: &PineSubscriptionHost) -> Option<String> {
        match host.failure(1) {
            Some(ScriptFailure::Runtime(error)) => Some(error.message),
            Some(other) => panic!("not a runtime failure: {other:?}"),
            None => None,
        }
    }

    fn host_with(source: &str) -> (Store, PineSubscriptionHost) {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let host = PineSubscriptionHost::load(&store);
        (store, host)
    }

    #[test]
    fn a_script_that_stops_on_a_live_tick_says_why_and_stays_armed() {
        let (store, mut host) = host_with(FAILS_ON_A_TICK);
        let applied = [instance(FAILS_ON_A_TICK)];
        let mut data = bars();
        let mut feed = |host: &mut PineSubscriptionHost, close: f64, now: i64| {
            data[4].close = close;
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, now)
                .len()
        };
        feed(&mut host, 102.0, LIVE_NOW);
        assert_eq!(feed(&mut host, 102.1, LIVE_NOW + 1), 1);
        assert_eq!(runtime_message(&host), None);

        assert_eq!(feed(&mut host, 103.0, LIVE_NOW + 2), 0);
        assert_eq!(runtime_message(&host).as_deref(), Some("late"));
        // The script stays stopped, as on TradingView: the price coming back changes nothing, and
        // neither does a frame that brings no new data.
        for now in [LIVE_NOW + 3, LIVE_NOW + 4] {
            assert_eq!(feed(&mut host, 102.0, now), 0);
            assert_eq!(runtime_message(&host).as_deref(), Some("late"));
        }
        assert_eq!(store.pine_subscriptions()[0].state, PineAlertState::Active);
    }

    #[test]
    fn a_replay_that_gets_through_clears_a_runtime_error() {
        let (_store, mut host) = host_with(FAILS_ON_A_TICK);
        let applied = [instance(FAILS_ON_A_TICK)];
        let mut data = bars();
        let run = |host: &mut PineSubscriptionHost, data: &[Kline]| {
            host.run_active(1, "binance:BTCUSDT", "1h", data, &applied, LIVE_NOW)
                .len()
        };
        run(&mut host, &data);
        data[4].close = 103.0;
        run(&mut host, &data);
        assert!(host.failure(1).is_some());

        // Older bars were corrected, so the history is replayed, silently, and the script runs.
        data[0].close = 101.0;
        data[4].close = 102.0;
        assert_eq!(run(&mut host, &data), 0);
        assert_eq!(host.failure(1), None);
        data[4].close = 102.1;
        assert_eq!(run(&mut host, &data), 1);
    }

    #[test]
    fn a_script_that_stops_on_history_says_why_until_armed_anew() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)\n\
            if bar_index == 2\n    runtime.error(\"early\")";
        let (store, mut host) = host_with(source);
        let applied = [instance(source)];
        let mut data = bars();
        for tick in 0..3 {
            data[4].close += 1.0;
            let fired =
                host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + tick);
            assert!(fired.is_empty());
            assert_eq!(runtime_message(&host).as_deref(), Some("early"));
        }

        let mut paused = subscription(1, source);
        paused.state = PineAlertState::Paused;
        store.replace_pine_subscriptions(&[paused]).expect("write");
        host.reload(&store);
        assert_eq!(host.failure(1), None);
    }
}
