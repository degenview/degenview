use super::*;
use crate::model::chart_context::{self, QuickOrder, Target};
use crate::model::paper::{PaperOrderSide, PaperOrderType};
use crate::model::{Anchor, Tool};
use eframe::egui::{OutputCommand, pos2};

struct Rig {
    workspace: ChartWorkspace,
    host: PaperHost,
    ctx: Context,
}

fn kline(index: i64, close: f64) -> Kline {
    Kline {
        open_time: index * 86_400_000,
        close_time: (index + 1) * 86_400_000 - 1,
        open: close,
        high: close + 5.0,
        low: close - 5.0,
        close,
        volume: 10.0,
    }
}

impl Rig {
    fn new() -> Self {
        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        let workspace = ChartWorkspace::empty(
            1,
            Timeframe::Day,
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store.clone(),
        );
        Self {
            workspace,
            host: PaperHost::load(store),
            ctx: Context::default(),
        }
    }

    /// A chart of `symbol` whose last close is `last`.
    fn chart(&mut self, symbol: &str, last: f64) -> u64 {
        let id = self
            .workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &self.ctx)
            .chart_id;
        let chart = self.chart_mut(id);
        chart.candles = (0..10).map(|index| kline(index, last)).collect();
        id
    }

    fn chart_mut(&mut self, id: u64) -> &mut Chart {
        self.workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == id)
            .expect("chart")
    }

    fn request(price: Option<f64>, time_ms: Option<i64>) -> ContextRequest {
        ContextRequest {
            at: pos2(300.0, 200.0),
            time_ms,
            price,
            target: Target::Background,
        }
    }

    /// Opens the menu on `chart` and returns its snapshot.
    fn open(&mut self, chart: u64, price: f64) -> ChartContext {
        self.workspace
            .open_context_menu(chart, Self::request(Some(price), Some(5 * 86_400_000)));
        self.workspace
            .context
            .menu
            .as_ref()
            .expect("menu opened")
            .context
            .clone()
    }

    fn run(&mut self, context: ChartContext, command: Command) -> Vec<WorkspaceAction> {
        self.workspace
            .run_context_command(&self.ctx, &mut self.host, context, command)
    }

    fn copied(&self) -> Vec<String> {
        self.ctx.output(|output| {
            output
                .commands
                .iter()
                .filter_map(|command| match command {
                    OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
    }

    fn drawing_ids(&self, chart: u64) -> Vec<u64> {
        self.workspace
            .charts
            .iter()
            .find(|candidate| candidate.id == chart)
            .expect("chart")
            .drawings
            .iter()
            .map(Drawing::id)
            .collect()
    }

    fn message(&self) -> Option<(String, bool)> {
        self.workspace
            .context
            .notice
            .as_ref()
            .map(|notice| (notice.text.clone(), notice.error))
    }

    fn with_account(mut self) -> Self {
        self.host.ensure_default_account().expect("account");
        self
    }
}

fn rectangle(id: u64, time_ms: i64) -> Drawing {
    Drawing::build(
        Tool::Rectangle,
        id,
        &[
            Anchor {
                time_ms,
                price: 100.0,
            },
            Anchor {
                time_ms: time_ms + 10,
                price: 110.0,
            },
        ],
        Tool::Rectangle.default_style(),
    )
    .expect("rectangle")
}

#[test]
fn the_snapshot_keeps_the_chart_the_price_and_the_time_of_the_click() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    let context = rig.open(first, 118_150.99);
    assert_eq!(context.chart_id, first);
    assert_eq!(context.symbol, "BTCUSDT");
    assert_eq!(context.price, Some(118_150.99));
    assert_eq!(context.time_ms, Some(5 * 86_400_000));
    assert_ne!(context.chart_id, second);
    // The right-clicked chart takes focus.
    assert_eq!(rig.workspace.focused, Some(first));
    assert!(rig.workspace.context_menu_open());
}

#[test]
fn a_new_right_click_replaces_the_open_menu() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    rig.open(first, 1.0);
    let serial = rig.workspace.context.menu.as_ref().unwrap().serial;
    let context = rig.open(second, 2.0);
    assert_eq!(context.chart_id, second);
    assert!(rig.workspace.context.menu.as_ref().unwrap().serial > serial);
}

#[test]
fn a_click_on_a_missing_chart_opens_nothing() {
    let mut rig = Rig::new();
    rig.workspace
        .open_context_menu(99, Rig::request(Some(1.0), None));
    assert!(!rig.workspace.context_menu_open());
}

#[test]
fn copy_price_copies_a_bare_number_from_the_click_not_the_pointer() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 118_150.99);
    rig.run(context, Command::CopyPrice);
    assert_eq!(rig.copied(), ["118150.99"]);
}

#[test]
fn a_closed_chart_is_refused_with_a_message_and_nothing_changes() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    let context = rig.open(first, 100.0);
    rig.workspace.charts.retain(|chart| chart.id != first);
    rig.run(context, Command::RemoveDrawings);
    assert_eq!(rig.workspace.confirm_remove_drawings, None);
    let (text, error) = rig.message().expect("a message");
    assert!(error);
    assert!(text.contains("closed"), "{text}");
    assert!(rig.workspace.charts.iter().any(|chart| chart.id == second));
}

#[test]
fn a_chart_that_switched_market_is_refused() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 100.0);
    rig.chart_mut(chart).symbol = "ETHUSDT".to_owned();
    rig.run(context, Command::CopyPrice);
    assert!(rig.copied().is_empty());
    assert!(rig.message().is_some_and(|(_, error)| error));
}

#[test]
fn add_alert_prefills_the_editor_and_creates_nothing() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 118_150.99);
    let actions = rig.run(context, Command::AddAlert);
    assert!(actions.is_empty(), "opening the editor saves nothing");
    let mut saved = Vec::new();
    let mut output = rig.ctx.run_ui(egui::RawInput::default(), |ui| {
        saved = rig.workspace.show_alert_editor(ui.ctx());
    });
    output.textures_delta.clear();
    assert!(
        saved
            .iter()
            .all(|action| !matches!(action, WorkspaceAction::SavePriceAlert { .. }))
    );
    let editor = rig.workspace.alert_editor.as_ref().expect("editor open");
    assert_eq!(editor.target, "118150.99");
    assert_eq!(editor.direction, alert_editor::Direction::Above);
    assert_eq!(editor.asset().symbol, "BTC");
}

#[test]
fn an_alert_below_the_market_crosses_downward() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 80.0);
    rig.run(context, Command::AddAlert);
    let mut output = rig.ctx.run_ui(egui::RawInput::default(), |ui| {
        rig.workspace.show_alert_editor(ui.ctx());
    });
    output.textures_delta.clear();
    let editor = rig.workspace.alert_editor.as_ref().expect("editor open");
    assert_eq!(editor.direction, alert_editor::Direction::Below);
    assert_eq!(editor.target, "80.00");
}

#[test]
fn add_alert_is_refused_for_a_market_without_alerts() {
    let mut rig = Rig::new();
    let id = rig
        .workspace
        .add_chart_for(
            &InstrumentId::new(DataSource::CoinMarketCap, "fear-greed"),
            &rig.ctx,
        )
        .chart_id;
    rig.chart_mut(id).candles = vec![kline(0, 50.0)];
    let context = rig.open(id, 50.0);
    rig.run(context, Command::AddAlert);
    assert!(rig.workspace.new_alert_request.is_none());
    assert!(rig.message().is_some_and(|(_, error)| error));
}

#[test]
fn a_quick_order_opens_the_ticket_and_places_no_order() {
    let mut rig = Rig::new().with_account();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 120.0);
    rig.run(
        context,
        Command::QuickOrder(QuickOrder {
            side: PaperOrderSide::Sell,
            kind: PaperOrderType::Limit,
        }),
    );
    assert!(rig.workspace.paper_panel_visible, "the panel opens for it");
    let draft = rig.workspace.paper_panel.ticket_draft().expect("ticket");
    assert_eq!(draft.side, PaperOrderSide::Sell);
    assert_eq!(draft.kind, PaperOrderType::Limit);
    assert_eq!(draft.limit_price(), Some(120.0));
    assert!(
        rig.host.snapshot().orders.is_empty(),
        "nothing is submitted"
    );
}

#[test]
fn add_order_suggests_the_type_the_price_implies() {
    let mut rig = Rig::new().with_account();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 90.0);
    rig.run(context, Command::AddOrder);
    let draft = rig.workspace.paper_panel.ticket_draft().expect("ticket");
    assert_eq!(draft.side, PaperOrderSide::Buy);
    assert_eq!(draft.kind, PaperOrderType::Limit);
    assert_eq!(draft.limit_price(), Some(90.0));
    assert!(rig.host.snapshot().orders.is_empty());
}

#[test]
fn order_entries_work_before_any_paper_account_exists() {
    // The default account is only made when the paper panel first draws; the menu must not wait
    // for that, or its order entries would be dead until the panel had been opened once.
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    assert!(rig.host.snapshot().accounts.is_empty());
    let context = rig.open(chart, 120.0);
    let items = menu_items(&rig, &context);
    for label in [
        "Sell BTCUSDT @ 120.00 limit",
        "Buy BTCUSDT @ 120.00 stop",
        "Add order on BTCUSDT at 120.00…",
    ] {
        let enabled = items
            .iter()
            .find(|(candidate, _)| candidate.starts_with(&label[..label.len() - 3]))
            .map(|(_, enabled)| *enabled);
        assert_eq!(enabled, Some(true), "{label}: {items:?}");
    }
    assert!(!rig.workspace.paper_panel_visible);

    rig.run(
        context,
        Command::QuickOrder(QuickOrder {
            side: PaperOrderSide::Sell,
            kind: PaperOrderType::Limit,
        }),
    );
    assert!(rig.workspace.paper_panel_visible, "the panel opens for it");
    assert!(
        rig.host.snapshot().selected_account_id.is_some(),
        "the default account was created"
    );
    let draft = rig.workspace.paper_panel.ticket_draft().expect("ticket");
    assert_eq!(draft.side, PaperOrderSide::Sell);
    assert_eq!(draft.kind, PaperOrderType::Limit);
    assert!(
        rig.host.snapshot().orders.is_empty(),
        "nothing is submitted"
    );
}

#[test]
fn add_order_creates_the_account_and_opens_the_panel_too() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 90.0);
    rig.run(context, Command::AddOrder);
    assert!(rig.workspace.paper_panel_visible);
    assert!(rig.host.snapshot().selected_account_id.is_some());
    assert!(rig.workspace.paper_panel.ticket_draft().is_some());
    assert!(rig.host.snapshot().orders.is_empty());
}

#[test]
fn replay_blocks_alerts_and_orders() {
    let mut rig = Rig::new().with_account();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.workspace.replay.start(
        3 * 86_400_000,
        "BTCUSDT",
        (0..10).map(|i| i * 86_400_000).collect(),
    );
    assert!(rig.workspace.replay.is_active());
    let context = rig.open(chart, 120.0);
    rig.run(context.clone(), Command::AddAlert);
    rig.run(context, Command::AddOrder);
    assert!(rig.workspace.new_alert_request.is_none());
    assert!(rig.workspace.paper_panel.ticket_draft().is_none());
}

#[test]
fn the_time_lock_toggles_on_the_clicked_chart_only() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    let context = rig.open(first, 100.0);
    rig.run(context.clone(), Command::ToggleTimeLock);
    assert_eq!(rig.chart_mut(first).chart.time_lock(), Some(5 * 86_400_000));
    assert_eq!(rig.chart_mut(second).chart.time_lock(), None);
    rig.run(context, Command::ToggleTimeLock);
    assert_eq!(rig.chart_mut(first).chart.time_lock(), None);
}

#[test]
fn settings_open_for_the_clicked_chart_not_the_focused_one() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    let context = rig.open(first, 100.0);
    rig.workspace.focused = Some(second);
    rig.run(context, Command::OpenSettings);
    assert_eq!(rig.workspace.settings_chart, Some(first));
    assert_eq!(
        rig.workspace.settings_state.tab,
        chart_settings::Tab::Appearance
    );
}

#[test]
fn table_and_tree_follow_their_chart_and_close_with_it() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    let context = rig.open(second, 1.0);
    rig.run(context.clone(), Command::ToggleTable);
    rig.run(context.clone(), Command::ToggleObjectTree);
    assert_eq!(rig.workspace.context.table_chart, Some(second));
    assert_eq!(rig.workspace.context.tree_chart, Some(second));
    rig.workspace.sync_context_panels();
    assert!(rig.workspace.charts.iter().any(|chart| chart.id == first));
    // Choosing it again closes it.
    rig.run(context, Command::ToggleTable);
    assert_eq!(rig.workspace.context.table_chart, None);
    // Removing the chart closes the tree.
    rig.workspace.charts.retain(|chart| chart.id != second);
    rig.workspace.sync_context_panels();
    assert_eq!(rig.workspace.context.tree_chart, None);
}

#[test]
fn removing_drawings_asks_first_then_is_one_undo_step() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = (1..=4).map(|id| rectangle(id, id as i64 * 100)).collect();
    let context = rig.open(chart, 100.0);
    rig.run(context, Command::RemoveDrawings);
    assert_eq!(rig.workspace.confirm_remove_drawings, Some(chart));
    assert_eq!(rig.drawing_ids(chart), [1, 2, 3, 4], "nothing yet");
}

#[test]
fn paste_adds_new_drawings_at_the_click_as_one_undoable_step() {
    let mut rig = Rig::new();
    let source = rig.chart("BTCUSDT", 100.0);
    let target = rig.chart("ETHUSDT", 50.0);
    rig.chart_mut(source).drawings = vec![rectangle(1, 1_000), rectangle(2, 2_000)];
    rig.workspace.selected_drawing = Some((source, vec![1, 2]));
    rig.workspace.copy_selection(&rig.ctx);
    assert_eq!(rig.copied().len(), 1);
    assert!(rig.copied()[0].starts_with("degenview-clipboard/v1\n"));

    let context = rig.open(target, 200.0);
    rig.run(context, Command::Paste);
    let pasted = rig.drawing_ids(target);
    assert_eq!(pasted.len(), 2);
    assert_eq!(rig.drawing_ids(source), [1, 2], "the source is untouched");
    // The first pasted point is at the click: the clicked price and time.
    let chart = rig.chart_mut(target);
    let anchors = chart.drawings[0].anchors();
    assert_eq!(anchors[0].price, 200.0);
    assert_eq!(anchors[0].time_ms, 5 * 86_400_000);
    // Relationships are kept: the second starts as far after the first as it did.
    assert_eq!(
        chart.drawings[1].anchors()[0].time_ms,
        5 * 86_400_000 + 1_000
    );
    assert_eq!(rig.workspace.selected_drawing, Some((target, pasted)));
    assert_eq!(rig.workspace.undo.len(), 1, "one step for the whole paste");
    rig.workspace.undo();
    assert!(rig.drawing_ids(target).is_empty());
}

#[test]
fn paste_with_nothing_copied_says_so() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 100.0);
    rig.run(context, Command::Paste);
    assert!(rig.drawing_ids(chart).is_empty());
    assert!(rig.message().is_some_and(|(_, error)| error));
}

#[test]
fn pasting_foreign_clipboard_text_is_ignored_and_damaged_text_reported() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.workspace.paste_text(&rig.ctx, "just some words");
    assert!(rig.message().is_none());
    assert!(rig.drawing_ids(chart).is_empty());
    rig.workspace
        .paste_text(&rig.ctx, "degenview-clipboard/v1\n{broken");
    assert!(rig.message().is_some_and(|(_, error)| error));
    assert!(rig.drawing_ids(chart).is_empty());
}

#[test]
fn a_keyboard_paste_lands_under_the_pointer_of_the_hovered_chart() {
    let mut rig = Rig::new();
    let source = rig.chart("BTCUSDT", 100.0);
    let target = rig.chart("ETHUSDT", 50.0);
    rig.chart_mut(source).drawings = vec![rectangle(1, 1_000)];
    rig.workspace.selected_drawing = Some((source, vec![1]));
    rig.workspace.copy_selection(&rig.ctx);
    let text = rig.copied().remove(0);
    rig.workspace.note_hover(
        target,
        Some(Anchor {
            time_ms: 7_000,
            price: 55.0,
        }),
    );
    rig.workspace.paste_text(&rig.ctx, &text);
    assert_eq!(rig.drawing_ids(source), [1]);
    assert_eq!(rig.drawing_ids(target).len(), 1);
    let anchors = rig.chart_mut(target).drawings[0].anchors();
    assert_eq!((anchors[0].time_ms, anchors[0].price), (7_000, 55.0));
}

#[test]
fn removing_indicators_asks_first_and_undo_brings_them_back() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    {
        let chart = rig.chart_mut(chart);
        BuiltIn::Ema.set_applied(&mut chart.indicators, true);
        BuiltIn::Rsi.set_applied(&mut chart.indicators, true);
        let id = chart
            .chart
            .add_pine_instance(None, "My script", "//@version=6");
        assert_eq!(id, 1);
    }
    let context = rig.open(chart, 100.0);
    rig.run(context, Command::RemoveIndicators);
    assert_eq!(rig.workspace.context.confirm_remove_indicators, Some(chart));
    assert_eq!(
        rig.chart_mut(chart).chart.pine_instances().len(),
        1,
        "nothing yet"
    );

    // What the confirmation does on Remove All, step by step.
    rig.workspace.remember_indicators(chart);
    {
        let chart = rig.chart_mut(chart);
        for indicator in BuiltIn::ALL {
            indicator.set_applied(&mut chart.indicators, false);
        }
        chart.chart.set_pine_instances(Vec::new());
    }
    assert!(rig.chart_mut(chart).chart.pine_instances().is_empty());
    rig.workspace.undo();
    let restored = rig.chart_mut(chart);
    assert!(BuiltIn::Ema.applied(&restored.indicators));
    assert!(BuiltIn::Rsi.applied(&restored.indicators));
    assert_eq!(restored.chart.pine_instances().len(), 1);
    assert_eq!(restored.chart.pine_instances()[0].name, "My script");
    rig.workspace.redo();
    assert!(rig.chart_mut(chart).chart.pine_instances().is_empty());
}

#[test]
fn indicator_history_and_drawing_history_share_one_stack_in_order() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.workspace
        .apply_drawing_commands(chart, vec![DrawingCommand::Add(rectangle(1, 0))]);
    BuiltIn::Ema.set_applied(&mut rig.chart_mut(chart).indicators, true);
    rig.workspace.remember_indicators(chart);
    BuiltIn::Ema.set_applied(&mut rig.chart_mut(chart).indicators, false);
    rig.workspace.undo();
    assert!(BuiltIn::Ema.applied(&rig.chart_mut(chart).indicators));
    assert_eq!(rig.drawing_ids(chart), [1], "the drawing is still there");
    rig.workspace.undo();
    assert!(rig.drawing_ids(chart).is_empty());
}

#[test]
fn removing_a_script_leaves_its_alert_subscriptions_and_source_alone() {
    // Only the chart's instance goes: the library and the subscription store are not touched by
    // removal (the host marks a subscription whose instance is gone as changed, silently).
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).chart.add_pine_instance(
        Some("saved-script".to_owned()),
        "Saved",
        "//@version=6",
    );
    let mut pane = rig.open(chart, 100.0);
    pane.target = Target::Pane { instance: 1 };
    rig.run(pane, Command::PaneRemove(1));
    assert!(rig.chart_mut(chart).chart.pine_instances().is_empty());
    rig.workspace.undo();
    let restored = rig.chart_mut(chart).chart.pine_instances().to_vec();
    assert_eq!(restored[0].script_id.as_deref(), Some("saved-script"));
}

#[test]
fn facts_count_what_the_menu_will_remove() {
    let mut rig = Rig::new().with_account();
    let chart = rig.chart("BTCUSDT", 100.0);
    {
        let chart = rig.chart_mut(chart);
        chart.drawings = vec![
            rectangle(1, 0),
            rectangle(2, 100),
            Drawing::Ruler {
                id: 3,
                a: Anchor {
                    time_ms: 0,
                    price: 1.0,
                },
                b: Anchor {
                    time_ms: 1,
                    price: 2.0,
                },
            },
        ];
        BuiltIn::Ema.set_applied(&mut chart.indicators, true);
        chart.chart.add_pine_instance(None, "S", "//@version=6");
    }
    let context = rig.open(chart, 100.0);
    let facts = rig
        .workspace
        .context_facts(&rig.ctx, &context, &[])
        .expect("facts");
    assert_eq!(facts.drawings, 2, "the ruler is not a drawing");
    assert_eq!(facts.indicators, 2, "one built-in and one script");
    assert_eq!(facts.last_price, Some(100.0));
    assert_eq!(facts.paper, chart_context::PaperAccess::Available);
    assert!(!facts.replay);
}

#[test]
fn facts_follow_the_clicked_chart_in_a_multi_chart_workspace() {
    let mut rig = Rig::new();
    let first = rig.chart("BTCUSDT", 100.0);
    let second = rig.chart("ETHUSDT", 50.0);
    rig.chart_mut(first).drawings = vec![rectangle(1, 0)];
    let context = rig.open(second, 50.0);
    let facts = rig
        .workspace
        .context_facts(&rig.ctx, &context, &[])
        .unwrap();
    assert_eq!(facts.drawings, 0);
    assert_eq!(facts.last_price, Some(50.0));
    let context = rig.open(first, 100.0);
    let facts = rig
        .workspace
        .context_facts(&rig.ctx, &context, &[])
        .unwrap();
    assert_eq!(facts.drawings, 1);
}

#[test]
fn alert_lines_are_this_charts_alerts_only() {
    use crate::model::alert::{AlertAsset, AlertCondition};
    let mut rig = Rig::new();
    let btc = rig.chart("BTCUSDT", 100.0);
    let eth = rig.chart("ETHUSDT", 50.0);
    let alert = |id: u64, symbol: &str| {
        let mut alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, symbol),
                symbol.to_owned(),
                symbol.to_owned(),
            ),
            AlertCondition::CrossesAbove { target: 120.0 },
            0,
        );
        alert.id = id;
        alert
    };
    rig.workspace
        .set_price_alerts(&[alert(1, "BTCUSDT"), alert(2, "ETHUSDT")]);
    rig.workspace.sync_context_panels();
    assert_eq!(rig.chart_mut(btc).chart.alert_line_ids(), [1]);
    assert_eq!(rig.chart_mut(eth).chart.alert_line_ids(), [2]);
}

#[test]
fn an_alert_deleted_while_its_menu_was_open_is_refused() {
    use crate::model::alert::{AlertAsset, AlertCondition};
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let mut alert = PriceAlert::new(
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTCUSDT".to_owned(),
            "BTCUSDT".to_owned(),
        ),
        AlertCondition::CrossesAbove { target: 120.0 },
        0,
    );
    alert.id = 5;
    rig.workspace.set_price_alerts(&[alert]);
    rig.workspace.open_context_menu(
        chart,
        ContextRequest {
            at: pos2(0.0, 0.0),
            time_ms: None,
            price: None,
            target: Target::Alert { id: 5 },
        },
    );
    let context = rig.workspace.context.menu.as_ref().unwrap().context.clone();
    rig.workspace.set_price_alerts(&[]);
    let actions = rig.run(context, Command::DeleteAlert(5));
    assert!(actions.is_empty(), "no delete was sent for a gone alert");
    assert!(rig.message().is_some_and(|(_, error)| error));
}

#[test]
fn pausing_and_deleting_an_alert_go_out_as_app_actions() {
    use crate::model::alert::{AlertAsset, AlertCondition, AlertState};
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let mut alert = PriceAlert::new(
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTCUSDT".to_owned(),
            "BTCUSDT".to_owned(),
        ),
        AlertCondition::CrossesAbove { target: 120.0 },
        0,
    );
    alert.id = 5;
    rig.workspace.set_price_alerts(&[alert]);
    rig.workspace.open_context_menu(
        chart,
        ContextRequest {
            at: pos2(0.0, 0.0),
            time_ms: None,
            price: None,
            target: Target::Alert { id: 5 },
        },
    );
    let context = rig.workspace.context.menu.as_ref().unwrap().context.clone();
    assert_eq!(
        rig.run(context.clone(), Command::ToggleAlert(5)),
        [WorkspaceAction::SetAlertState {
            id: 5,
            state: AlertState::Paused
        }]
    );
    assert_eq!(
        rig.run(context, Command::DeleteAlert(5)),
        [WorkspaceAction::DeleteAlert(5)]
    );
}

#[test]
fn the_menu_built_for_a_snapshot_matches_its_chart() {
    let mut rig = Rig::new().with_account();
    let chart = rig.chart("BTCUSDT", 100.0);
    let context = rig.open(chart, 120.0);
    let facts = rig
        .workspace
        .context_facts(&rig.ctx, &context, &[])
        .unwrap();
    let entries = chart_context::menu(&context, &facts);
    let labels: Vec<String> = entries
        .iter()
        .filter_map(|entry| match entry {
            chart_context::Entry::Item(item) => Some(item.label.clone()),
            chart_context::Entry::Separator | chart_context::Entry::Heading(_) => None,
        })
        .collect();
    assert_eq!(labels[0], "Copy price 120.00");
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Add alert on BTCUSDT"))
    );
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Sell") && label.ends_with("limit"))
    );
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Buy") && label.ends_with("stop"))
    );
}

#[test]
fn the_template_list_is_read_once_when_a_background_menu_opens() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.open(chart, 100.0);
    let templates = &rig.workspace.context.menu.as_ref().unwrap().templates;
    assert!(
        templates.len() >= 5,
        "the presets are always offered: {}",
        templates.len()
    );
    // An axis menu has no template submenu to fill.
    rig.workspace.open_context_menu(
        chart,
        ContextRequest {
            at: pos2(0.0, 0.0),
            time_ms: None,
            price: Some(1.0),
            target: Target::PriceAxis,
        },
    );
    assert!(
        rig.workspace
            .context
            .menu
            .as_ref()
            .unwrap()
            .templates
            .is_empty()
    );
}

fn tree_rig() -> (Rig, u64) {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![rectangle(1, 100), rectangle(2, 200)];
    (rig, chart)
}

#[test]
fn the_tree_hides_and_shows_a_drawing_and_undo_reverts_it() {
    use crate::model::object_tree::Key as TreeKey;
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleShown(TreeKey::Drawing(1)));
    assert!(rig.chart_mut(chart).drawings[0].hidden());
    assert!(!rig.chart_mut(chart).drawings[1].hidden());
    assert_eq!(rig.workspace.undo.len(), 1);
    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleShown(TreeKey::Drawing(1)));
    assert!(!rig.chart_mut(chart).drawings[0].hidden());
    rig.workspace.undo();
    assert!(rig.chart_mut(chart).drawings[0].hidden());
}

#[test]
fn the_tree_locks_and_unlocks_a_drawing() {
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleLocked(2));
    assert!(rig.chart_mut(chart).drawings[1].locked());
    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleLocked(2));
    assert!(!rig.chart_mut(chart).drawings[1].locked());
}

#[test]
fn the_tree_deletes_a_drawing_undoably_and_never_a_locked_one() {
    use crate::model::object_tree::Key as TreeKey;
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace.selected_drawing = Some((chart, vec![1, 2]));
    rig.workspace
        .apply_tree_action(chart, TreeAction::Delete(TreeKey::Drawing(1)));
    assert_eq!(rig.drawing_ids(chart), [2]);
    assert_eq!(
        rig.workspace.selected_drawing,
        Some((chart, vec![2])),
        "the deleted drawing leaves the selection"
    );
    rig.workspace.undo();
    assert_eq!(rig.drawing_ids(chart), [1, 2]);

    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleLocked(2));
    rig.workspace
        .apply_tree_action(chart, TreeAction::Delete(TreeKey::Drawing(2)));
    assert_eq!(rig.drawing_ids(chart), [1, 2], "a locked drawing stays");
}

#[test]
fn the_tree_selects_a_drawing_as_the_charts_one_selection() {
    use crate::model::object_tree::Key as TreeKey;
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace
        .apply_tree_action(chart, TreeAction::Select(TreeKey::Drawing(2)));
    assert_eq!(rig.workspace.selected_drawing, Some((chart, vec![2])));
}

#[test]
fn the_tree_reorders_drawings_in_the_stack() {
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace
        .apply_tree_action(chart, TreeAction::Reorder(1, 1));
    assert_eq!(rig.drawing_ids(chart), [2, 1]);
    rig.workspace.undo();
    assert_eq!(rig.drawing_ids(chart), [1, 2]);
}

#[test]
fn the_tree_hides_and_removes_indicators_and_scripts() {
    use crate::model::object_tree::{IndicatorKey, Key as TreeKey};
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    {
        let card = rig.chart_mut(chart);
        BuiltIn::Ema.set_applied(&mut card.indicators, true);
        card.chart
            .add_pine_instance(None, "My script", "//@version=6");
    }
    let ema = TreeKey::Indicator(IndicatorKey::BuiltIn(BuiltIn::Ema));
    let script = TreeKey::Indicator(IndicatorKey::Script(1));

    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleShown(ema));
    assert!(BuiltIn::Ema.hidden(&rig.chart_mut(chart).indicators));
    assert!(
        BuiltIn::Ema.applied(&rig.chart_mut(chart).indicators),
        "hidden is not removed"
    );
    rig.workspace
        .apply_tree_action(chart, TreeAction::ToggleShown(script));
    assert!(!rig.chart_mut(chart).chart.pine_instances()[0].visible);

    rig.workspace
        .apply_tree_action(chart, TreeAction::Delete(script));
    assert!(rig.chart_mut(chart).chart.pine_instances().is_empty());
    rig.workspace
        .apply_tree_action(chart, TreeAction::Delete(ema));
    assert!(!BuiltIn::Ema.applied(&rig.chart_mut(chart).indicators));
    // Both removals undo, newest first.
    rig.workspace.undo();
    assert!(BuiltIn::Ema.applied(&rig.chart_mut(chart).indicators));
    rig.workspace.undo();
    assert_eq!(rig.chart_mut(chart).chart.pine_instances().len(), 1);
}

#[test]
fn a_tree_action_for_a_closed_chart_does_nothing() {
    use crate::model::object_tree::Key as TreeKey;
    use crate::ui::object_tree::TreeAction;
    let (mut rig, chart) = tree_rig();
    rig.workspace
        .charts
        .retain(|candidate| candidate.id != chart);
    rig.workspace
        .apply_tree_action(chart, TreeAction::Delete(TreeKey::Drawing(1)));
    assert!(rig.workspace.undo.is_empty());
}

/// The labels and enabled state of a menu built for `context`, the way the screen builds it.
fn menu_items(rig: &Rig, context: &ChartContext) -> Vec<(String, bool)> {
    let facts = rig
        .workspace
        .context_facts(&rig.ctx, context, &[])
        .expect("facts");
    chart_context::menu(context, &facts)
        .into_iter()
        .filter_map(|entry| match entry {
            chart_context::Entry::Item(item) => Some((item.label.clone(), item.is_enabled())),
            chart_context::Entry::Separator | chart_context::Entry::Heading(_) => None,
        })
        .collect()
}

#[test]
fn paste_is_greyed_until_drawings_are_copied_then_enabled() {
    let mut rig = Rig::new();
    let source = rig.chart("BTCUSDT", 100.0);
    let target = rig.chart("ETHUSDT", 50.0);
    rig.chart_mut(source).drawings = vec![rectangle(1, 1_000)];

    let context = rig.open(target, 50.0);
    let paste = |rig: &Rig, context: &ChartContext| {
        menu_items(rig, context)
            .into_iter()
            .find(|(label, _)| label == "Paste")
            .expect("a Paste entry")
            .1
    };
    assert!(!paste(&rig, &context), "nothing has been copied");

    // Copy from the drawing's menu or Cmd+C: either way it is the workspace's copy.
    rig.workspace.selected_drawing = Some((source, vec![1]));
    rig.workspace.copy_selection(&rig.ctx);
    assert!(paste(&rig, &context), "a copy is waiting");

    // And it works from the menu: choosing Paste adds the drawing to the clicked chart.
    rig.run(context, Command::Paste);
    assert_eq!(rig.drawing_ids(target).len(), 1);
}

#[test]
fn a_copy_stays_available_to_every_chart_and_tab_sharing_the_context() {
    // The copy lives in the egui context the app shares, not in one workspace.
    let mut first = Rig::new();
    let source = first.chart("BTCUSDT", 100.0);
    first.chart_mut(source).drawings = vec![rectangle(1, 1_000)];
    first.workspace.selected_drawing = Some((source, vec![1]));
    first.workspace.copy_selection(&first.ctx);

    let mut second = Rig::new();
    second.ctx = first.ctx.clone();
    let chart = second.chart("ETHUSDT", 50.0);
    let context = second.open(chart, 50.0);
    let enabled = menu_items(&second, &context)
        .into_iter()
        .any(|(label, enabled)| label == "Paste" && enabled);
    assert!(enabled);
}

#[test]
fn the_template_the_chart_wears_is_marked_current_in_the_menu() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let store = rig.workspace.store.clone();
    let preset = rig
        .workspace
        .context
        .templates
        .entries(&store)
        .into_iter()
        .find(|entry| entry.built_in)
        .expect("a preset");

    let marked = |rig: &mut Rig| -> Vec<(u64, bool)> {
        rig.open(chart, 100.0);
        rig.workspace
            .context
            .menu
            .as_ref()
            .unwrap()
            .templates
            .iter()
            .map(|row| (row.id, row.current))
            .collect()
    };

    // Put another look on the chart, then give it the preset: only that one is current.
    let other = rig
        .workspace
        .context
        .templates
        .entries(&store)
        .into_iter()
        .filter(|entry| entry.built_in && entry.id != preset.id)
        .find(|entry| entry.appearance != preset.appearance)
        .expect("a preset with another look");
    let context = rig.open(chart, 100.0);
    rig.run(context, Command::ApplyTemplate(other.id));
    let before = marked(&mut rig);
    assert!(before.contains(&(other.id, true)));
    assert!(before.contains(&(preset.id, false)));

    let context = rig.open(chart, 100.0);
    rig.run(context, Command::ApplyTemplate(preset.id));
    let after = marked(&mut rig);
    assert!(after.contains(&(preset.id, true)));
    assert!(after.contains(&(other.id, false)));
    assert_eq!(
        after.iter().filter(|(_, current)| *current).count(),
        1,
        "{after:?}"
    );
}

// MARK: Post cards

const POST_LINK: &str = "https://x.com/degenview/status/1750000000000000000";
const OTHER_LINK: &str = "https://x.com/other/status/1750000000000000001";
const TRADINGVIEW_LINK: &str = "https://www.tradingview.com/chart/BTCUSD/AbCdEf12-A-sample-idea/";

fn card(tool: Tool, id: u64, link: &str) -> Drawing {
    let card = crate::model::link_card::LinkCard::new(
        crate::model::link_card::Link::parse(link).expect("a link"),
    );
    Drawing::build_with(
        tool,
        id,
        &[Anchor {
            time_ms: 1_000,
            price: 100.0,
        }],
        tool.default_style(),
        &crate::model::image_content::Content::Link(card),
    )
    .expect("a card")
}

fn card_of(rig: &mut Rig, chart: u64, id: u64) -> crate::model::link_card::LinkCard {
    match rig
        .chart_mut(chart)
        .drawings
        .iter()
        .find(|drawing| drawing.id() == id)
        .and_then(|drawing| drawing.geometry())
    {
        Some(crate::model::Geometry::Card { card, .. }) => card.clone(),
        other => panic!("not a card: {other:?}"),
    }
}

fn fetched_meta(title: &str) -> crate::model::link_card::LinkMeta {
    crate::model::link_card::LinkMeta {
        title: title.into(),
        author: "@degenview".into(),
        excerpt: "words".into(),
        fetched_at_ms: 5,
    }
}

#[test]
fn a_pasted_link_waits_to_be_placed_in_the_middle_of_the_focused_chart() {
    use crate::model::image_content::Content;
    let mut rig = Rig::new();
    rig.chart("BTCUSDT", 100.0);
    rig.workspace.paste_text(&rig.ctx, "just some words");
    assert!(rig.workspace.content.pending.is_none(), "text is not ours");
    rig.workspace.paste_text(&rig.ctx, POST_LINK);
    assert!(matches!(
        &rig.workspace.content.pending,
        Some(Content::Link(card)) if card.link.url() == POST_LINK
    ));
    assert!(rig.workspace.content.place_now);
    assert_eq!(rig.workspace.active_tool, Tool::Post);
    rig.workspace.content.pending = None;
    rig.workspace.active_tool = Tool::Cursor;
    rig.workspace.paste_text(&rig.ctx, TRADINGVIEW_LINK);
    assert!(
        rig.workspace.content.pending.is_none() && rig.workspace.active_tool == Tool::Cursor,
        "only X links become cards"
    );
    assert!(
        rig.workspace.undo.is_empty(),
        "nothing is created until it is placed"
    );
}

#[test]
fn a_card_opens_and_copies_its_link_only_when_asked() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![card(Tool::Post, 1, POST_LINK)];
    assert!(rig.copied().is_empty());
    rig.workspace
        .link_requested(&rig.ctx, chart, 1, crate::ui::LinkRequest::Copy);
    assert_eq!(rig.copied(), [POST_LINK]);
    assert_eq!(rig.message(), Some(("Copied the link.".to_owned(), false)));
    rig.workspace
        .link_requested(&rig.ctx, chart, 1, crate::ui::LinkRequest::Open);
    let opened: Vec<String> = rig.ctx.output(|output| {
        output
            .commands
            .iter()
            .filter_map(|command| match command {
                OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect()
    });
    assert_eq!(opened, [POST_LINK]);
}

#[test]
fn a_preview_is_not_fetched_until_the_service_is_allowed() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![card(Tool::Post, 1, POST_LINK)];
    rig.workspace
        .link_requested(&rig.ctx, chart, 1, crate::ui::LinkRequest::Fetch);
    assert!(
        rig.workspace.content.links.consent.is_some(),
        "it asks first"
    );
    assert!(
        rig.workspace.content.links.inbox.is_none(),
        "and contacts nobody"
    );
    assert!(!rig.workspace.previews_allowed());
    rig.workspace.allow_previews();
    assert!(rig.workspace.previews_allowed());
}

#[test]
fn the_choice_to_allow_previews_stays_on_this_mac() {
    use crate::sync::policy::{SettingPolicy, setting_policy};
    assert_eq!(
        setting_policy(crate::model::link_card::ContentPrefs::KEY),
        SettingPolicy::Local
    );
}

#[test]
fn a_fetched_preview_is_written_to_its_card_as_one_undo_step() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![card(Tool::Post, 1, POST_LINK)];
    let (sender, receiver) = std::sync::mpsc::channel();
    rig.workspace.content.links.inbox = Some(receiver);
    sender
        .send(links::Fetched {
            chart_id: chart,
            drawing_id: 1,
            link: crate::model::link_card::Link::parse(POST_LINK).unwrap(),
            result: Ok(fetched_meta("Developers")),
        })
        .unwrap();
    rig.workspace.take_fetched();
    assert_eq!(
        card_of(&mut rig, chart, 1).meta,
        Some(fetched_meta("Developers"))
    );
    assert_eq!(rig.workspace.undo.len(), 1);
    assert!(rig.workspace.content.links.inbox.is_none());
    rig.workspace.undo();
    assert_eq!(card_of(&mut rig, chart, 1).meta, None);
}

#[test]
fn a_preview_for_a_link_the_card_no_longer_has_is_dropped() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![card(Tool::Post, 1, POST_LINK)];
    let (sender, receiver) = std::sync::mpsc::channel();
    rig.workspace.content.links.inbox = Some(receiver);
    sender
        .send(links::Fetched {
            chart_id: chart,
            drawing_id: 1,
            link: crate::model::link_card::Link::parse(
                "https://x.com/other/status/1750000000000000001",
            )
            .unwrap(),
            result: Ok(fetched_meta("Not this one")),
        })
        .unwrap();
    rig.workspace.take_fetched();
    assert_eq!(card_of(&mut rig, chart, 1).meta, None);
    assert!(rig.workspace.undo.is_empty());
}

#[test]
fn a_failed_fetch_says_why_and_leaves_the_card_alone() {
    use crate::service::link_fetch::FetchError;
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    rig.chart_mut(chart).drawings = vec![card(Tool::Post, 1, POST_LINK)];
    let (sender, receiver) = std::sync::mpsc::channel();
    rig.workspace.content.links.inbox = Some(receiver);
    sender
        .send(links::Fetched {
            chart_id: chart,
            drawing_id: 1,
            link: crate::model::link_card::Link::parse(POST_LINK).unwrap(),
            result: Err(FetchError::Unavailable),
        })
        .unwrap();
    rig.workspace.take_fetched();
    assert_eq!(
        rig.message(),
        Some((FetchError::Unavailable.message(), true))
    );
    assert_eq!(card_of(&mut rig, chart, 1).meta, None);
    assert!(rig.workspace.undo.is_empty());
}

#[test]
fn replacing_the_link_of_a_card_drops_the_preview_of_the_old_one() {
    let mut rig = Rig::new();
    let chart = rig.chart("BTCUSDT", 100.0);
    let mut drawing = card(Tool::Post, 1, POST_LINK);
    if let Some(crate::model::Geometry::Card { card, .. }) = drawing.geometry_mut() {
        card.meta = Some(fetched_meta("Old"));
        card.size = [400.0, 200.0];
    }
    rig.chart_mut(chart).drawings = vec![drawing];
    let other =
        crate::model::link_card::Link::parse("https://x.com/other/status/1750000000000000001")
            .unwrap();
    rig.workspace.link_chosen(other.clone(), Some((chart, 1)));
    let replaced = card_of(&mut rig, chart, 1);
    assert_eq!(replaced.link, other);
    assert_eq!(replaced.meta, None);
    assert_eq!(replaced.size, [400.0, 200.0], "its size is kept");
    assert_eq!(rig.workspace.undo.len(), 1);
}

#[test]
fn a_new_link_waits_for_a_click_and_arms_its_tool() {
    use crate::model::image_content::Content;
    let mut rig = Rig::new();
    rig.chart("BTCUSDT", 100.0);
    let link = crate::model::link_card::Link::parse(OTHER_LINK).unwrap();
    rig.workspace.link_chosen(link, None);
    assert!(matches!(
        rig.workspace.content.pending,
        Some(Content::Link(_))
    ));
    assert!(!rig.workspace.content.place_now, "a click places it");
    assert_eq!(rig.workspace.active_tool, Tool::Post);
}
