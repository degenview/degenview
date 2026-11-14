use super::*;
use crate::pine::outputs::{LabelStyle, TablePosition};
use crate::pine::runtime::containers::Object;
use crate::pine::runtime::value::RefKind;
use RuntimeValue::{Float, Int, Na, String as Str};

fn bar(open_time: i64, close: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + 60_000,
        open: close,
        high: close,
        low: close,
        close,
        volume: 0.0,
    }
}

fn arg(value: RuntimeValue) -> Arg {
    Arg { name: None, value }
}

fn run(state: &mut RuntimeState, name: &str, args: &[Arg]) -> Result<RuntimeValue, Diagnostic> {
    call(
        name,
        args,
        state,
        &bar(0, 0.0),
        60.0,
        &DrawingLimits::default(),
        SourceRange::ZERO,
    )
}

fn point(state: &mut RuntimeState, index: i64, price: f64) -> RuntimeValue {
    let id = allocate(state);
    let mut fields = HashMap::new();
    fields.insert("index".to_owned(), Int(index));
    fields.insert("time".to_owned(), Na);
    fields.insert("price".to_owned(), Float(price));
    state.instances.insert(
        id,
        Object {
            type_name: CHART_POINT_TYPE.to_owned(),
            fields,
        },
    );
    RuntimeValue::Ref(RefKind::Object, id)
}

#[test]
fn line_new_plain_and_point_forms() {
    let mut state = RuntimeState::default();
    let line = run(
        &mut state,
        "line.new",
        &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Line, id) = line else {
        panic!("{line:?}")
    };
    assert_eq!(state.lines[&id].x1, 1);
    assert_eq!(state.lines[&id].y2, 20.0);
    assert_eq!(state.lines[&id].color, DEFAULT_COLOR);

    let first = point(&mut state, 5, 50.0);
    let second = point(&mut state, 9, 90.0);
    let line2 = run(&mut state, "line.new", &[arg(first), arg(second)]).unwrap();
    let RuntimeValue::Ref(RefKind::Line, id2) = line2 else {
        panic!("{line2:?}")
    };
    assert_eq!(state.lines[&id2].x1, 5);
    assert_eq!(state.lines[&id2].y2, 90.0);
}

#[test]
fn line_setters_and_getters() {
    let mut state = RuntimeState::default();
    let line = run(
        &mut state,
        "line.new",
        &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
    )
    .unwrap();
    run(&mut state, "line.set_x2", &[arg(line.clone()), arg(Int(7))]).unwrap();
    assert_eq!(
        run(&mut state, "line.get_x2", &[arg(line.clone())]).unwrap(),
        Int(7)
    );
    assert_eq!(
        run(&mut state, "line.get_y1", &[arg(line.clone())]).unwrap(),
        Float(10.0)
    );
    run(
        &mut state,
        "line.set_xy1",
        &[arg(line.clone()), arg(Int(2)), arg(Float(15.0))],
    )
    .unwrap();
    assert_eq!(
        run(&mut state, "line.get_x1", &[arg(line.clone())]).unwrap(),
        Int(2)
    );
    run(&mut state, "line.delete", &[arg(line.clone())]).unwrap();
    assert_eq!(run(&mut state, "line.get_x1", &[arg(line)]).unwrap(), Na);
}

#[test]
fn label_style_defaults_and_keeps_existing_on_unknown() {
    let mut state = RuntimeState::default();
    let label = run(
        &mut state,
        "label.new",
        &[arg(Int(1)), arg(Float(10.0)), arg(Str("hi".into()))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Label, id) = label.clone() else {
        panic!("{label:?}")
    };
    assert_eq!(state.labels[&id].style, LabelStyle::LabelDown);
    assert_eq!(state.labels[&id].text, "hi");
    // An unknown style name falls back to labelCenter, matching Pine.
    run(
        &mut state,
        "label.set_style",
        &[arg(label.clone()), arg(Str("label.style_nope".into()))],
    )
    .unwrap();
    assert_eq!(state.labels[&id].style, LabelStyle::LabelCenter);
    // `na` leaves the current style alone.
    run(&mut state, "label.set_style", &[arg(label), arg(Na)]).unwrap();
    assert_eq!(state.labels[&id].style, LabelStyle::LabelCenter);
}

#[test]
fn box_and_table_round_trip() {
    let mut state = RuntimeState::default();
    let boxed = run(
        &mut state,
        "box.new",
        &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Box, box_id) = boxed else {
        panic!("{boxed:?}")
    };
    assert_eq!(state.boxes[&box_id].border_color, Some(DEFAULT_COLOR));
    assert_eq!(state.boxes[&box_id].background_color, Some(DEFAULT_COLOR));

    let table = run(
        &mut state,
        "table.new",
        &[arg(Int(0)), arg(Int(2)), arg(Int(2))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Table, table_id) = table else {
        panic!("{table:?}")
    };
    assert_eq!(state.tables[&table_id].columns, 2);
    assert_eq!(state.tables[&table_id].rows, 2);
    assert_eq!(state.tables[&table_id].position, TablePosition::TopRight);
    run(
        &mut state,
        "table.cell",
        &[
            arg(RuntimeValue::Ref(RefKind::Table, table_id)),
            arg(Int(0)),
            arg(Int(0)),
            arg(Str("x".into())),
        ],
    )
    .unwrap();
    assert_eq!(state.tables[&table_id].cells.len(), 1);
    assert_eq!(state.tables[&table_id].cells[0].text, "x");
}

#[test]
fn table_cell_out_of_bounds_is_pine4013() {
    let mut state = RuntimeState::default();
    let table = run(
        &mut state,
        "table.new",
        &[arg(Int(0)), arg(Int(1)), arg(Int(1))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Table, table_id) = table else {
        panic!()
    };
    let error = run(
        &mut state,
        "table.cell",
        &[
            arg(RuntimeValue::Ref(RefKind::Table, table_id)),
            arg(Int(3)),
            arg(Int(0)),
            arg(Str("x".into())),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code, "PINE4013");
}

#[test]
fn chart_point_from_index_and_now() {
    let mut state = RuntimeState {
        bar_index: 4,
        ..RuntimeState::default()
    };
    let p = run(
        &mut state,
        "chart.point.from_index",
        &[arg(Int(2)), arg(Float(5.0))],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Object, id) = p else {
        panic!("{p:?}")
    };
    assert_eq!(state.instances[&id].fields["index"], Int(2));
    assert_eq!(state.instances[&id].fields["price"], Float(5.0));

    let now = run(&mut state, "chart.point.now", &[]).unwrap();
    let RuntimeValue::Ref(RefKind::Object, now_id) = now else {
        panic!("{now:?}")
    };
    assert_eq!(state.instances[&now_id].fields["index"], Int(4));
}

#[test]
fn polyline_collects_known_points() {
    let mut state = RuntimeState::default();
    let a = point(&mut state, 1, 10.0);
    let b = point(&mut state, 2, 20.0);
    let array = crate::pine::builtins::array::new_array(&mut state, vec![a, b, Na]);
    let polyline = run(&mut state, "polyline.new", &[arg(array)]).unwrap();
    let RuntimeValue::Ref(RefKind::Polyline, id) = polyline else {
        panic!("{polyline:?}")
    };
    assert_eq!(state.polylines[&id].points.len(), 2);
    assert_eq!(state.polylines[&id].line_color, Some(DEFAULT_COLOR));
    assert_eq!(state.polylines[&id].fill_color, None);
}

#[test]
fn handles_covers_namespaces() {
    assert!(
        handles("line.new")
            && handles("line.set_x1")
            && handles("line.get_x1")
            && handles("line.delete")
    );
    assert!(
        handles("label.new") && handles("box.new") && handles("table.new") && handles("table.cell")
    );
    assert!(handles("linefill.new") && handles("polyline.new") && handles("polyline.delete"));
    assert!(handles("chart.point") && handles("chart.point.now"));
    assert!(!handles("array.get") && !handles("plot") && !handles("line.nope_member"));
}
