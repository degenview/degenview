use super::*;
use crate::model::DataSource;
use crate::pine::compiler::program::InputValue;
use crate::workspace::test_support::empty_workspace;

const ASKS: &str = "//@version=6
indicator(\"Range\", overlay = true)
from = input.time(0, \"From\", confirm = true)
level = input.price(0, \"Level\", confirm = true)
plot(close)";

const ASKS_MORE: &str = "//@version=6
indicator(\"More\")
from = input.time(0, \"From\", confirm = true)
len = input.int(14, \"Length\", confirm = true)
plot(close)";

fn workspace() -> (ChartWorkspace, u64, egui::Context) {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(11);
    let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
    let chart = workspace.add_chart_for(&btc, &ctx).chart_id;
    (workspace, chart, ctx)
}

fn add(workspace: &mut ChartWorkspace, chart: u64, source: &str) -> u64 {
    match workspace.add_script(chart, NewScript::new(None, "Script", source)) {
        AddOutcome::AwaitingInputs(id) => id,
        other => panic!("{other:?}"),
    }
}

/// A click on the chart, as the card reports it.
fn pick(workspace: &mut ChartWorkspace, time_ms: i64, price: f64) {
    let session = workspace.script_setups.queue.front_mut().expect("a setup");
    session.setup.pick(time_ms, price);
    workspace.finish_setups();
}

fn scripts(workspace: &ChartWorkspace) -> &[PineInstance] {
    workspace.charts[0].chart.pine_instances()
}

#[test]
fn a_script_that_asks_goes_on_the_chart_only_when_everything_is_given() {
    let (mut workspace, chart, ctx) = workspace();
    let steps = workspace.history.steps();
    let id = add(&mut workspace, chart, ASKS);
    assert!(scripts(&workspace).is_empty());
    assert!(
        workspace.persisted().charts[0].pine_instances.is_empty(),
        "a waiting script is never saved"
    );
    assert_eq!(workspace.script_setups.asking_chart(), Some(chart));
    let view = workspace.script_setups.view(chart);
    assert_eq!(view.pending[0].name, "Script");
    let prompt = view.pick.unwrap().prompt.unwrap();
    assert_eq!(prompt, "Set the “From” time for “Range”");

    pick(&mut workspace, 5_000, 1.0);
    assert!(scripts(&workspace).is_empty(), "one point to go");
    pick(&mut workspace, 6_000, 42.5);

    let applied = &scripts(&workspace)[0];
    assert_eq!(applied.id, id);
    assert_eq!(applied.input_values()["from"], InputValue::Int(5_000));
    assert_eq!(applied.input_values()["level"], InputValue::Float(42.5));
    assert_eq!(workspace.history.steps(), steps + 1, "one step in all");
    assert_eq!(workspace.script_setups.asking_chart(), None);
    workspace.undo(&ctx);
    assert!(scripts(&workspace).is_empty());
}

#[test]
fn giving_up_leaves_no_trace() {
    let (mut workspace, chart, _) = workspace();
    let steps = workspace.history.steps();
    add(&mut workspace, chart, ASKS);
    pick(&mut workspace, 5_000, 1.0);
    assert!(workspace.cancel_setup());
    assert!(scripts(&workspace).is_empty());
    assert_eq!(workspace.history.steps(), steps);
    assert!(!workspace.cancel_setup());
}

#[test]
fn other_confirm_inputs_wait_for_the_dialog() {
    let (mut workspace, chart, _) = workspace();
    add(&mut workspace, chart, ASKS_MORE);
    pick(&mut workspace, 5_000, 1.0);
    assert!(workspace.script_setups.dialog_open());
    assert!(scripts(&workspace).is_empty());
    workspace
        .script_setups
        .queue
        .front_mut()
        .unwrap()
        .setup
        .confirm();
    workspace.finish_setups();
    assert_eq!(scripts(&workspace).len(), 1);
}

#[test]
fn a_script_with_nothing_to_confirm_is_added_at_once() {
    let (mut workspace, chart, _) = workspace();
    let plain = "//@version=6\nindicator(\"P\")\nt = input.time(0, \"T\")\nplot(close)";
    for source in [plain, "not a script"] {
        assert!(matches!(
            workspace.add_script(chart, NewScript::new(None, "P", source)),
            AddOutcome::Added(_)
        ));
    }
}

#[test]
fn an_agents_add_asks_nothing() {
    let (mut workspace, chart, _) = workspace();
    assert!(
        workspace
            .insert_script(chart, NewScript::new(None, "Script", ASKS))
            .is_some()
    );
    assert_eq!(scripts(&workspace).len(), 1);
    assert_eq!(workspace.script_setups.asking_chart(), None);
}

#[test]
fn waiting_scripts_keep_their_ids_apart_from_the_ones_added_meanwhile() {
    let (mut workspace, chart, _) = workspace();
    let waiting = add(&mut workspace, chart, ASKS);
    let direct = workspace
        .insert_script(chart, NewScript::new(None, "P", "plot(close)"))
        .unwrap();
    assert_ne!(waiting, direct);
    let second = add(&mut workspace, chart, ASKS);
    assert!(second != waiting && second != direct);
    // The first asked is the first answered.
    assert_eq!(workspace.script_setups.view(chart).pending.len(), 2);
    pick(&mut workspace, 1, 1.0);
    pick(&mut workspace, 2, 2.0);
    assert!(scripts(&workspace).iter().any(|s| s.id == waiting));
    assert!(!scripts(&workspace).iter().any(|s| s.id == second));
    assert!(workspace.cancel_pending_script(chart, second));
    assert_eq!(workspace.script_setups.asking_chart(), None);
}

#[test]
fn reset_points_changes_the_script_once_and_giving_up_keeps_it() {
    let (mut workspace, chart, _) = workspace();
    let id = workspace
        .insert_script(chart, NewScript::new(None, "Script", ASKS))
        .unwrap();
    let steps = workspace.history.steps();
    workspace.reset_points(chart, id);
    workspace.reset_points(chart, id);
    assert_eq!(workspace.script_setups.queue.len(), 1);
    pick(&mut workspace, 5_000, 1.0);
    assert!(
        scripts(&workspace)[0].inputs.is_empty(),
        "not until the end"
    );
    assert!(workspace.cancel_setup());
    assert!(scripts(&workspace)[0].inputs.is_empty());
    assert_eq!(workspace.history.steps(), steps);

    workspace.reset_points(chart, id);
    pick(&mut workspace, 5_000, 1.0);
    pick(&mut workspace, 6_000, 2.0);
    assert_eq!(
        scripts(&workspace)[0].input_values()["level"],
        InputValue::Float(2.0)
    );
    assert_eq!(workspace.history.steps(), steps + 1);
}

#[test]
fn a_setup_goes_with_its_chart_and_starts_over_on_another_market() {
    let (mut workspace, chart, ctx) = workspace();
    let id = workspace
        .insert_script(chart, NewScript::new(None, "Script", ASKS))
        .unwrap();
    add(&mut workspace, chart, ASKS);
    pick(&mut workspace, 5_000, 1.0);
    workspace.charts[0].symbol = "ETHUSDT".into();
    workspace.reconcile_setups();
    assert!(
        workspace
            .script_setups
            .view(chart)
            .pick
            .unwrap()
            .picked
            .is_empty()
    );
    workspace.cancel_setup();

    workspace.reset_points(chart, id);
    workspace.charts[0].symbol = "SOLUSDT".into();
    workspace.reconcile_setups();
    assert_eq!(workspace.script_setups.asking_chart(), None);

    add(&mut workspace, chart, ASKS);
    workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "XRPUSDT"), &ctx);
    workspace.remove_chart(chart);
    assert!(workspace.script_setups.queue.is_empty());
}

#[test]
fn esc_gives_up_the_setup_only_after_what_lies_on_top() {
    let (mut workspace, chart, ctx) = workspace();
    add(&mut workspace, chart, ASKS);
    workspace.indicator_picker.open();
    workspace.escape_over_charts(&ctx);
    assert_eq!(workspace.script_setups.asking_chart(), Some(chart));
    workspace.indicator_picker.close();
    workspace.escape_over_charts(&ctx);
    assert_eq!(workspace.script_setups.asking_chart(), None);
}
