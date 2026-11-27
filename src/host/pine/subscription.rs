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
use crate::model::alert::pine::{
    ChartKey, PineAlertState, PineAlertSubscription, RearmBlock, ScriptFailure,
};
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
    on: ChartKey,
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

/// What the alert runtime needs from one open chart: its market, the bars its scripts run over
/// and the scripts. Those are the market's candles, or the bars the chart type constructs from
/// them, so a script's alert sees what the chart shows.
pub struct ChartFeed {
    pub on: ChartKey,
    pub symbol_key: String,
    pub timeframe: String,
    pub bars: Vec<Kline>,
    pub run: ChartRun,
    pub instances: Vec<PineInstance>,
}

impl ChartFeed {
    /// The applied indicator `subscription` was armed on, if this chart still has it.
    fn instance_of(&self, subscription: &PineAlertSubscription) -> Option<&PineInstance> {
        let id = subscription.instance_id?;
        self.instances.iter().find(|instance| instance.id == id)
    }

    /// Whether an alert whose tab is not open belongs here now: the chart of the same id, on the
    /// alert's market, running the very script it was armed on. Nothing less is taken, so an
    /// unrelated chart that happens to share the id never claims the alert. An active one also
    /// waits while another alert watches that indicator.
    fn can_take_over(
        &self,
        subscription: &PineAlertSubscription,
        subscriptions: &[PineAlertSubscription],
    ) -> bool {
        self.on.chart_id == subscription.chart_id
            && subscription.watches(&self.symbol_key, &self.timeframe)
            && self.instance_of(subscription).is_some_and(|instance| {
                has_source_of(subscription, instance)
                    && !(subscription.is_active()
                        && watched(subscriptions, subscription, self.on, instance.id))
            })
    }
}

/// Whether an alert other than `subscription` is armed on `instance_id` of `on` for its market.
fn watched(
    subscriptions: &[PineAlertSubscription],
    subscription: &PineAlertSubscription,
    on: ChartKey,
    instance_id: u64,
) -> bool {
    subscriptions.iter().any(|other| {
        other.id != subscription.id
            && other.arms(
                on,
                instance_id,
                &subscription.symbol_key,
                &subscription.timeframe,
            )
    })
}

fn has_source_of(subscription: &PineAlertSubscription, instance: &PineInstance) -> bool {
    crate::store::scripts::hash_source(&instance.source) == subscription.source_hash
}

/// Whether `instance` is the alert's script: the same saved script, whatever its text now; for one
/// that came from no saved script, the very same text.
fn is_script_of(subscription: &PineAlertSubscription, instance: &PineInstance) -> bool {
    match &subscription.script_id {
        Some(script_id) => instance.script_id.as_ref() == Some(script_id),
        None => has_source_of(subscription, instance),
    }
}

/// What a switched-off alert would be re-armed on.
struct RearmTarget {
    on: ChartKey,
    instance: PineInstance,
}

/// Whether the tab `subscription` names is open.
fn tab_open(subscription: &PineAlertSubscription, charts: &[ChartFeed]) -> bool {
    charts
        .iter()
        .any(|chart| Some(chart.on.workspace_id) == subscription.workspace_id)
}

/// The indicator a switched-off alert would be re-armed on: the one it was armed on if that is
/// still there, else the one copy of its script on its chart. With its tab closed, any open chart
/// of the same id may be that chart.
fn rearm_target(
    subscription: &PineAlertSubscription,
    subscriptions: &[PineAlertSubscription],
    charts: &[ChartFeed],
) -> Result<RearmTarget, RearmBlock> {
    if subscription.state == PineAlertState::ScriptDeleted {
        return Err(RearmBlock::ScriptDeleted);
    }
    let tab_open = tab_open(subscription, charts);
    let own: Vec<&ChartFeed> = charts
        .iter()
        .filter(|chart| {
            chart.on.chart_id == subscription.chart_id
                && (!tab_open || subscription.is_on(chart.on))
        })
        .collect();
    if own.is_empty() {
        return Err(RearmBlock::ChartNotOpen);
    }
    let on_market: Vec<&ChartFeed> = own
        .into_iter()
        .filter(|chart| subscription.watches(&chart.symbol_key, &chart.timeframe))
        .collect();
    if on_market.is_empty() {
        return Err(RearmBlock::OtherMarket);
    }
    let mut found: Vec<(ChartKey, &PineInstance)> = on_market
        .iter()
        .filter_map(|chart| {
            let instance = chart
                .instance_of(subscription)
                .filter(|instance| has_source_of(subscription, instance))?;
            Some((chart.on, instance))
        })
        .collect();
    if found.is_empty() {
        found = on_market
            .iter()
            .flat_map(|chart| {
                chart
                    .instances
                    .iter()
                    .filter(|instance| is_script_of(subscription, instance))
                    .map(|instance| (chart.on, instance))
            })
            .collect();
    }
    let (on, instance) = match found.as_slice() {
        [] => return Err(RearmBlock::ScriptNotApplied),
        [one] => *one,
        _ => return Err(RearmBlock::AppliedMoreThanOnce),
    };
    if watched(subscriptions, subscription, on, instance.id) {
        return Err(RearmBlock::AlreadyArmed);
    }
    Ok(RearmTarget {
        on,
        instance: instance.clone(),
    })
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
    /// What each switched-off alert can be re-armed on, as of the last [`Self::reconcile`].
    rearm: HashMap<u64, Result<RearmTarget, RearmBlock>>,
}

impl PineSubscriptionHost {
    pub fn load(store: &Store) -> Self {
        Self {
            subscriptions: store.pine_subscriptions(),
            running: HashMap::new(),
            rearm: HashMap::new(),
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

    /// Why a switched-off alert cannot be re-armed; `None` when it can.
    pub fn rearm_block(&self, subscription_id: u64) -> Option<RearmBlock> {
        match self.rearm.get(&subscription_id) {
            Some(target) => target.as_ref().err().copied(),
            // No chart has been looked at for it yet.
            None => Some(RearmBlock::ChartNotOpen),
        }
    }

    /// Arms a switched-off alert again on the indicator [`Self::reconcile`] found for it. Does
    /// nothing while [`Self::rearm_block`] says why it cannot be.
    pub fn rearm(&mut self, subscription_id: u64, store: &Store) {
        let Some(Ok(target)) = self.rearm.remove(&subscription_id) else {
            return;
        };
        let source_hash = crate::store::scripts::hash_source(&target.instance.source);
        let mut subscriptions = store.pine_subscriptions();
        for subscription in subscriptions
            .iter_mut()
            .filter(|subscription| subscription.id == subscription_id)
        {
            subscription.rearm_on(target.on, &target.instance, source_hash.clone());
        }
        let _ = store.replace_pine_subscriptions(&subscriptions);
        self.reload(store);
    }

    /// Forgets what was built for a subscription that is gone, switched off or now has another
    /// source, so arming it again starts from a fresh compile and shows no old failure.
    fn drop_stopped_runs(&mut self) {
        let subscriptions = &self.subscriptions;
        self.running.retain(|id, running| {
            subscriptions.iter().any(|subscription| {
                subscription.id == *id
                    && subscription.is_active()
                    && subscription.source_hash == running.key.source_hash
            })
        });
    }

    #[cfg(test)]
    pub fn subscriptions(&self) -> &[PineAlertSubscription] {
        &self.subscriptions
    }

    /// Brings the subscriptions up to date with every open chart, and works out what each
    /// switched-off one could be re-armed on.
    ///
    /// An alert whose tab is not open moves to a chart that runs its script on its market (a saved
    /// layout opened in another tab), and otherwise waits; paused alerts too, so they can be
    /// re-armed there. Active ones are pinned to the applied instance that created them: a removed
    /// instance or changed source becomes silent until explicitly re-armed, and current instance
    /// inputs are persisted so the independent host never falls back to input defaults.
    pub fn reconcile(&mut self, charts: &[ChartFeed], store: &Store) {
        let mut changed = false;
        // One at a time: each move is judged against the ones already made.
        for index in 0..self.subscriptions.len() {
            let subscription = &self.subscriptions[index];
            let moved_to = (!tab_open(subscription, charts))
                .then(|| {
                    charts
                        .iter()
                        .find(|chart| chart.can_take_over(subscription, &self.subscriptions))
                })
                .flatten()
                .map(|chart| chart.on.workspace_id);
            let subscription = &mut self.subscriptions[index];
            if let Some(workspace_id) = moved_to {
                subscription.workspace_id = Some(workspace_id);
                changed = true;
            }
            if !subscription.is_active() {
                continue;
            }
            let Some(chart) = charts.iter().find(|chart| subscription.is_on(chart.on)) else {
                continue;
            };
            let Some(instance) = chart.instance_of(subscription) else {
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
        self.rearm = self
            .subscriptions
            .iter()
            .filter(|subscription| !subscription.is_active())
            .map(|subscription| {
                let target = rearm_target(subscription, &self.subscriptions, charts);
                (subscription.id, target)
            })
            .collect();
    }

    /// [`Self::run_active_on`] for a time-based chart on the first tab.
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
        let feed = ChartFeed {
            on: ChartKey {
                workspace_id: 1,
                chart_id,
            },
            symbol_key: symbol_key.to_owned(),
            timeframe: timeframe.to_owned(),
            bars: bars.to_vec(),
            run: ChartRun::default(),
            instances: instances.to_vec(),
        };
        self.run_active_on(&feed, now_ms)
    }

    /// Feeds one chart's bars to every active subscription on it that watches its market,
    /// returning `(subscription, alert)` pairs for the alerts raised by the new realtime tick or
    /// close. Each subscription owns one persistent controller, so a forming bar is re-evaluated
    /// as a tick (`freq_all`, `freq_once_per_bar`) and `freq_once_per_bar_close` waits for the
    /// real close. The first snapshot, and any rebuild after the market, source or inputs
    /// changed, replays history silently. A subscription without a stored source is skipped. The
    /// chart may be price-based: `bars` are the bars the chart's scripts run over, and `run` says
    /// how they were made.
    pub fn run_active_on(
        &mut self,
        feed: &ChartFeed,
        now_ms: i64,
    ) -> Vec<(PineAlertSubscription, AlertEvent)> {
        let ChartFeed {
            on,
            symbol_key,
            timeframe,
            bars,
            run: chart,
            instances,
        } = feed;
        let symbol = chart.symbol(symbol_key);
        let mut fired = Vec::new();
        self.drop_stopped_runs();
        for subscription in &self.subscriptions {
            if !subscription.is_active()
                || !subscription.is_on(*on)
                || !subscription.watches(symbol_key, timeframe)
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
                on: *on,
                instance_id: subscription.instance_id,
                symbol_key: symbol_key.clone(),
                timeframe: timeframe.clone(),
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
            workspace_id: Some(1),
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

    /// Chart 1 of a tab, on the alert's market.
    fn chart(workspace_id: u64, instances: &[PineInstance]) -> ChartFeed {
        ChartFeed {
            on: ChartKey {
                workspace_id,
                chart_id: 1,
            },
            ..feed(&bars(), &ChartRun::default(), instances)
        }
    }

    fn feed(bars: &[Kline], run: &ChartRun, instances: &[PineInstance]) -> ChartFeed {
        ChartFeed {
            on: ChartKey {
                workspace_id: 1,
                chart_id: 1,
            },
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            bars: bars.to_vec(),
            run: run.clone(),
            instances: instances.to_vec(),
        }
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
                host.reconcile(&[chart(1, &applied)], &store);
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

    /// A corrected script goes on the chart as a new instance: its new alert fires, and the old
    /// one reads as changed until it is re-armed.
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
        host.reconcile(&[chart(1, &applied)], &store);
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
    fn another_tabs_chart_with_the_same_id_neither_silences_nor_feeds_an_alert() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        // The other tab comes first and has nothing applied.
        let mut charts = [chart(2, &[]), chart(1, &[instance(source)])];
        host.reconcile(&charts, &store);
        assert_eq!(store.pine_subscriptions()[0].state, PineAlertState::Active);
        assert_eq!(store.pine_subscriptions()[0].workspace_id, Some(1));

        // Even running the same script, it is not the alert's chart.
        charts[0] = chart(2, &[instance(source)]);
        for feed in &charts {
            assert!(host.run_active_on(feed, LIVE_NOW).is_empty());
        }
        charts[0].bars[4].close += 1.0;
        assert!(host.run_active_on(&charts[0], LIVE_NOW + 1).is_empty());
        charts[1].bars[4].close += 1.0;
        assert_eq!(host.run_active_on(&charts[1], LIVE_NOW + 1).len(), 1);

        // Its own tab losing the indicator is a change, whatever another tab still runs.
        charts[1] = chart(1, &[]);
        host.reconcile(&charts, &store);
        assert_eq!(
            store.pine_subscriptions()[0].state,
            PineAlertState::ScriptChanged
        );
    }

    #[test]
    fn an_alert_whose_tab_is_not_open_moves_only_to_a_chart_running_its_script() {
        let source = "//@version=6\nindicator(\"S\")\nplot(close)";
        let applied = [instance(source)];
        // A closed tab, and a row saved before alerts named their tab; paused ones move as well.
        for (tab, state) in [
            (Some(9), PineAlertState::Active),
            (None, PineAlertState::Active),
            (None, PineAlertState::Paused),
        ] {
            let store = Store::open_in_memory().expect("open");
            let mut orphan = subscription(1, source);
            orphan.workspace_id = tab;
            orphan.state = state;
            store.replace_pine_subscriptions(&[orphan]).expect("write");
            let mut host = PineSubscriptionHost::load(&store);

            let mut other_market = chart(3, &applied);
            other_market.symbol_key = "binance:ETHUSDT".into();
            let mut other_script = instance("plot(open)");
            other_script.id = 10;
            host.reconcile(
                &[chart(2, &[]), other_market, chart(4, &[other_script])],
                &store,
            );
            let waiting = &store.pine_subscriptions()[0];
            assert_eq!((waiting.workspace_id, waiting.state), (tab, state));

            // The saved layout opened again, in a new tab.
            host.reconcile(&[chart(2, &[]), chart(5, &applied)], &store);
            let moved = &store.pine_subscriptions()[0];
            assert_eq!((moved.workspace_id, moved.state), (Some(5), state));
        }
    }

    #[test]
    fn an_alert_never_moves_onto_an_indicator_another_active_alert_watches() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let on_tab = |id: u64, tab: u64| PineAlertSubscription {
            workspace_id: Some(tab),
            ..subscription(id, source)
        };
        let applied = [instance(source)];
        // One layout armed in two tabs, one of them closed since; and two closed tabs that
        // would land on the same chart.
        for (subscriptions, tabs) in [
            (vec![on_tab(1, 1), on_tab(2, 2)], [Some(1), Some(2)]),
            (vec![on_tab(1, 8), on_tab(2, 9)], [Some(2), Some(9)]),
        ] {
            let (store, mut host) = host_of(&subscriptions);
            let mut live = chart(2, &applied);
            host.reconcile(std::slice::from_ref(&live), &store);
            let stored: Vec<_> = store
                .pine_subscriptions()
                .iter()
                .map(|subscription| subscription.workspace_id)
                .collect();
            assert_eq!(stored, tabs);
            assert!(host.run_active_on(&live, LIVE_NOW).is_empty());
            live.bars[4].close += 1.0;
            assert_eq!(host.run_active_on(&live, LIVE_NOW + 1).len(), 1);
        }
    }

    #[test]
    fn a_waiting_alert_moves_in_once_the_watching_one_is_paused() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let on_tab = |id: u64, tab: u64, state| PineAlertSubscription {
            workspace_id: Some(tab),
            state,
            ..subscription(id, source)
        };
        let (store, mut host) = host_of(&[
            on_tab(1, 1, PineAlertState::Active),
            on_tab(2, 2, PineAlertState::Active),
        ]);
        let mut live = chart(2, &[instance(source)]);
        host.reconcile(std::slice::from_ref(&live), &store);
        host.run_active_on(&live, LIVE_NOW);

        store
            .replace_pine_subscriptions(&[
                on_tab(1, 1, PineAlertState::Active),
                on_tab(2, 2, PineAlertState::Paused),
            ])
            .expect("write");
        host.reload(&store);
        host.reconcile(std::slice::from_ref(&live), &store);
        assert!(host.run_active_on(&live, LIVE_NOW + 1).is_empty());
        live.bars[4].close += 1.0;
        let fired = host.run_active_on(&live, LIVE_NOW + 2);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0.id, 1);
    }

    /// A row as saved before alerts named their tab.
    #[test]
    fn a_row_saved_before_alerts_named_their_tab_fires_on_its_chart() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let mut saved = serde_json::to_value(subscription(1, source)).expect("encode");
        saved
            .as_object_mut()
            .expect("object")
            .remove("workspace_id")
            .expect("field");
        let old: PineAlertSubscription = serde_json::from_value(saved).expect("decode");
        let (store, mut host) = host_of(&[old]);
        let mut live = chart(4, &[instance(source)]);
        host.reconcile(std::slice::from_ref(&live), &store);
        assert_eq!(store.pine_subscriptions()[0].workspace_id, Some(4));
        host.run_active_on(&live, LIVE_NOW);
        live.bars[4].close += 1.0;
        assert_eq!(host.run_active_on(&live, LIVE_NOW + 1).len(), 1);
    }

    const SAYS_OLD: &str = "//@version=6\nindicator(\"S\")\nalert(\"old\", alert.freq_all)";
    const SAYS_NEW: &str = "//@version=6\nindicator(\"S\")\nlen = input.int(2)\n\
        alert(\"new\", alert.freq_all)";

    /// A copy of the saved script the alerts were armed on.
    fn copy(id: u64, source: &str) -> PineInstance {
        PineInstance::new(id, Some("saved-script".into()), "S v2", source)
    }

    fn armed(id: u64, state: PineAlertState) -> PineAlertSubscription {
        PineAlertSubscription {
            script_id: Some("saved-script".into()),
            state,
            ..subscription(id, SAYS_OLD)
        }
    }

    fn host_of(subscriptions: &[PineAlertSubscription]) -> (Store, PineSubscriptionHost) {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(subscriptions)
            .expect("write");
        let host = PineSubscriptionHost::load(&store);
        (store, host)
    }

    #[test]
    fn rearm_takes_the_same_script_as_the_chart_has_it_now_and_stays_armed() {
        use crate::model::pine::instance::StoredInput;

        let (store, mut host) = host_of(&[armed(1, PineAlertState::Active)]);
        let mut newer = copy(11, SAYS_NEW);
        newer.inputs.insert("len".into(), StoredInput::Int(9));
        let charts = [chart(1, &[newer])];
        host.reconcile(&charts, &store);
        assert_eq!(
            store.pine_subscriptions()[0].state,
            PineAlertState::ScriptChanged
        );
        assert_eq!(host.rearm_block(1), None);

        host.rearm(1, &store);
        host.reconcile(&charts, &store);
        let rearmed = &store.pine_subscriptions()[0];
        assert_eq!(rearmed.state, PineAlertState::Active, "it stays armed");
        assert_eq!(
            (rearmed.instance_id, rearmed.source.as_str()),
            (Some(11), SAYS_NEW)
        );
        assert_eq!(
            rearmed.source_hash,
            crate::store::scripts::hash_source(SAYS_NEW)
        );
        assert_eq!(rearmed.inputs.get("len"), Some(&StoredInput::Int(9)));
        assert_eq!(rearmed.script_name, "S v2");

        let mut live = chart(1, &charts[0].instances);
        assert!(host.run_active_on(&live, LIVE_NOW).is_empty());
        live.bars[4].close += 1.0;
        let fired = host.run_active_on(&live, LIVE_NOW + 1);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].1.message, "new");
    }

    #[test]
    fn rearm_needs_exactly_one_indicator_to_take_and_prefers_the_one_it_was_armed_on() {
        let other_script = PineInstance::new(11, Some("another".into()), "T", SAYS_OLD);
        let mut other_market = chart(1, &[copy(10, SAYS_OLD)]);
        other_market.symbol_key = "binance:ETHUSDT".into();
        for (charts, block) in [
            (vec![], Some(RearmBlock::ChartNotOpen)),
            (vec![other_market], Some(RearmBlock::OtherMarket)),
            (
                vec![chart(1, &[other_script])],
                Some(RearmBlock::ScriptNotApplied),
            ),
            (
                vec![chart(1, &[copy(11, SAYS_NEW), copy(12, SAYS_NEW)])],
                Some(RearmBlock::AppliedMoreThanOnce),
            ),
            // Paused beside a second copy of its script: it goes back to its own.
            (
                vec![chart(1, &[copy(10, SAYS_OLD), copy(11, SAYS_NEW)])],
                None,
            ),
            // The id was given to another copy of the script since.
            (vec![chart(1, &[copy(10, SAYS_NEW)])], None),
        ] {
            let (store, mut host) = host_of(&[armed(1, PineAlertState::Paused)]);
            host.reconcile(&charts, &store);
            assert_eq!(host.rearm_block(1), block);
            let expected = match (block, charts.first()) {
                (None, Some(chart)) => (PineAlertState::Active, chart.instances[0].source.clone()),
                _ => (PineAlertState::Paused, SAYS_OLD.to_owned()),
            };
            host.rearm(1, &store);
            let after = &store.pine_subscriptions()[0];
            assert_eq!((after.state, after.source.clone()), expected, "{block:?}");
        }
    }

    #[test]
    fn rearm_never_arms_one_indicator_twice_or_a_deleted_script() {
        let mut newer = armed(2, PineAlertState::Active);
        newer.instance_id = Some(11);
        newer.source_hash = crate::store::scripts::hash_source(SAYS_NEW);
        let (store, mut host) = host_of(&[
            armed(1, PineAlertState::ScriptChanged),
            newer,
            armed(3, PineAlertState::ScriptDeleted),
        ]);
        host.reconcile(&[chart(1, &[copy(11, SAYS_NEW)])], &store);
        assert_eq!(host.rearm_block(1), Some(RearmBlock::AlreadyArmed));
        assert_eq!(host.rearm_block(3), Some(RearmBlock::ScriptDeleted));
    }

    /// A changed alert saved before alerts named their tab, or whose tab was closed, matches no
    /// chart exactly: it is re-armed on the one open chart of its id that has its script.
    #[test]
    fn rearm_finds_the_tab_of_a_changed_alert_that_has_none() {
        for tab in [None, Some(9)] {
            let mut orphan = armed(1, PineAlertState::ScriptChanged);
            orphan.workspace_id = tab;
            let (store, mut host) = host_of(&[orphan]);
            let both = [
                chart(2, &[copy(11, SAYS_NEW)]),
                chart(3, &[copy(11, SAYS_NEW)]),
            ];
            host.reconcile(&both, &store);
            assert_eq!(host.rearm_block(1), Some(RearmBlock::AppliedMoreThanOnce));

            host.reconcile(&[chart(2, &[]), chart(3, &[copy(11, SAYS_NEW)])], &store);
            assert_eq!(host.rearm_block(1), None, "{tab:?}");
            host.rearm(1, &store);
            assert_eq!(store.pine_subscriptions()[0].workspace_id, Some(3));
            assert_eq!(store.pine_subscriptions()[0].state, PineAlertState::Active);
        }
    }

    #[test]
    fn a_new_source_forgets_what_the_old_one_failed_on() {
        let (store, mut host) = host_of(&[subscription(1, UNCOMPILABLE)]);
        let applied = [instance(UNCOMPILABLE)];
        host.run_active(1, "binance:BTCUSDT", "1h", &bars(), &applied, LIVE_NOW);
        assert!(host.failure(1).is_some());

        store
            .replace_pine_subscriptions(&[subscription(1, SAYS_NEW)])
            .expect("write");
        host.reload(&store);
        assert_eq!(host.failure(1), None);
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
        host.reconcile(&[chart(1, &[applied.clone()])], &store);
        assert_eq!(
            store.pine_subscriptions()[0].inputs.get("len"),
            Some(&StoredInput::Int(9))
        );

        applied.source.push_str("\nplot(open)");
        host.reconcile(&[chart(1, &[applied])], &store);
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
        host.run_active_on(&feed(&candles, &plain, &applied), LIVE_NOW);
        candles[4].close += 0.5;
        let fired = host.run_active_on(&feed(&candles, &plain, &applied), LIVE_NOW + 1);
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
            host.run_active_on(&feed(&bricks, &chart, &applied), LIVE_NOW + 2)
                .is_empty()
        );
        candles[4].close += 2.0;
        let bricks = build_with(construction, &candles, LIVE_NOW + 3).klines();
        assert!(
            host.run_active_on(&feed(&bricks, &chart, &applied), LIVE_NOW + 3)
                .is_empty(),
            "no brick is wide"
        );
        // Back on candles: another silent restart, then the candles' own alerts again.
        assert!(
            host.run_active_on(&feed(&candles, &plain, &applied), LIVE_NOW + 4)
                .is_empty()
        );
        candles[4].close += 0.5;
        assert_eq!(
            host.run_active_on(&feed(&candles, &plain, &applied), LIVE_NOW + 5)
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
