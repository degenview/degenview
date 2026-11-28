use super::*;
use crate::pine::compiler::program::InputValue;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::inputs::points::PointAxes;
use crate::pine::syntax::token::Limits;

fn schema(body: &str) -> InputSchema {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}\nplot(1)");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    program.input_schema
}

fn asked(setup: &InputSetup) -> Option<String> {
    match setup.step() {
        Step::Pick(point) => Some(point.name.clone()),
        _ => None,
    }
}

const MIXED: &str = "
start = input.time(0, \"Start time\", inline = \"a\", confirm = true)
from = input.price(0, \"Start price\", inline = \"a\", confirm = true)
until = input.time(0, \"End time\", confirm = true)
level = input.price(0, \"Stop level\", confirm = true)
len = input.int(14, \"Length\", confirm = true)
later = input.time(0, \"NoConfirm\")
";

#[test]
fn adding_asks_the_confirm_points_in_order_and_then_the_dialog() {
    let mut setup = InputSetup::adding(&schema(MIXED)).unwrap();
    assert_eq!(asked(&setup).as_deref(), Some("a"));
    setup.pick(1_000, 10.5);
    assert_eq!(asked(&setup).as_deref(), Some("End time"));
    assert_eq!(
        setup.picked_points(),
        [PointValue {
            time_ms: Some(1_000),
            price: Some(10.5)
        }]
    );
    setup.pick(2_000, 11.0);
    assert_eq!(asked(&setup).as_deref(), Some("Stop level"));
    setup.pick(3_000, 12.0);
    assert_eq!(setup.step(), Step::Confirm);
    setup.edit(InputEdit::Set("len".into(), InputValue::Int(5)));
    setup.confirm();
    assert_eq!(setup.step(), Step::Done);

    let values = setup.values_on(&InputValues::new());
    assert_eq!(values["start"], InputValue::Int(1_000));
    assert_eq!(values["from"], InputValue::Float(10.5));
    // A time-only point takes the click's time, a price-only point its price.
    assert_eq!(values["until"], InputValue::Int(2_000));
    assert_eq!(values["level"], InputValue::Float(12.0));
    assert_eq!(values["len"], InputValue::Int(5));
    assert!(!values.contains_key("later"), "not asked when adding");
}

#[test]
fn only_confirm_points_means_no_dialog() {
    let mut setup = InputSetup::adding(&schema(
        "t = input.time(0, \"T\", confirm = true)\nn = input.int(1, \"N\")",
    ))
    .unwrap();
    setup.pick(5, 1.0);
    assert_eq!(setup.step(), Step::Done);
}

#[test]
fn only_other_confirm_inputs_go_straight_to_the_dialog() {
    let setup = InputSetup::adding(&schema("n = input.int(1, \"N\", confirm = true)")).unwrap();
    assert_eq!(setup.step(), Step::Confirm);
}

#[test]
fn nothing_marked_confirm_asks_nothing() {
    assert!(InputSetup::adding(&schema("t = input.time(0, \"T\")")).is_none());
    assert!(InputSetup::repicking(&schema("n = input.int(1, \"N\", confirm = true)")).is_none());
}

#[test]
fn repicking_asks_every_point_confirm_or_not_and_never_the_dialog() {
    let mut setup = InputSetup::repicking(&schema(MIXED)).unwrap();
    for time in 1..=4 {
        assert!(asked(&setup).is_some());
        setup.pick(time, 1.0);
    }
    assert_eq!(setup.step(), Step::Done);
    assert_eq!(
        setup.values_on(&InputValues::new())["later"],
        InputValue::Int(4)
    );
}

#[test]
fn an_input_switched_off_by_active_is_still_asked() {
    let setup = InputSetup::adding(&schema(
        "on = input.bool(false, \"On\")\n\
         t = input.time(0, \"T\", confirm = true, active = on)",
    ))
    .unwrap();
    assert!(matches!(
        setup.step(),
        Step::Pick(point) if point.axes == PointAxes::Time("t".into())
    ));
}

#[test]
fn what_is_given_lands_on_the_values_the_script_already_has() {
    let mut setup =
        InputSetup::repicking(&schema("t = input.time(7, \"T\")\nn = input.int(1, \"N\")"))
            .unwrap();
    let held = InputValues::from([("n".to_owned(), InputValue::Int(9))]);
    // A click on the default is no override.
    setup.pick(7, 0.0);
    assert_eq!(setup.values_on(&held), held);
}
