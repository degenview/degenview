use super::*;
use crate::pine::outputs::{LabelStyle, Size, TablePosition};
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

fn named(name: &str, value: RuntimeValue) -> Arg {
    Arg {
        name: Some(name.to_owned()),
        value,
    }
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

#[test]
fn force_overlay_is_recorded_on_every_drawing_that_takes_it() {
    let mut state = RuntimeState::default();
    let forced = || named("force_overlay", RuntimeValue::Bool(true));
    let corners = || vec![arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))];
    let with = |mut args: Vec<Arg>, extra: Arg| {
        args.push(extra);
        args
    };

    let line = run(&mut state, "line.new", &with(corners(), forced())).unwrap();
    let plain_line = run(&mut state, "line.new", &corners()).unwrap();
    let (RuntimeValue::Ref(_, line_id), RuntimeValue::Ref(_, plain_line_id)) = (line, plain_line)
    else {
        panic!()
    };
    assert!(state.lines[&line_id].force_overlay);
    assert!(!state.lines[&plain_line_id].force_overlay);

    let label_args = || vec![arg(Int(1)), arg(Float(10.0)), arg(Str("hi".into()))];
    let label = run(&mut state, "label.new", &with(label_args(), forced())).unwrap();
    let plain_label = run(&mut state, "label.new", &label_args()).unwrap();
    let (RuntimeValue::Ref(_, label_id), RuntimeValue::Ref(_, plain_label_id)) =
        (label, plain_label)
    else {
        panic!()
    };
    assert!(state.labels[&label_id].force_overlay);
    assert!(!state.labels[&plain_label_id].force_overlay);

    let boxed = run(&mut state, "box.new", &with(corners(), forced())).unwrap();
    let plain_box = run(&mut state, "box.new", &corners()).unwrap();
    let (RuntimeValue::Ref(_, box_id), RuntimeValue::Ref(_, plain_box_id)) = (boxed, plain_box)
    else {
        panic!()
    };
    assert!(state.boxes[&box_id].force_overlay);
    assert!(!state.boxes[&plain_box_id].force_overlay);

    let grid = || vec![arg(Int(0)), arg(Int(1)), arg(Int(1))];
    let table = run(&mut state, "table.new", &with(grid(), forced())).unwrap();
    let plain_table = run(&mut state, "table.new", &grid()).unwrap();
    let (RuntimeValue::Ref(_, table_id), RuntimeValue::Ref(_, plain_table_id)) =
        (table, plain_table)
    else {
        panic!()
    };
    assert!(state.tables[&table_id].force_overlay);
    assert!(!state.tables[&plain_table_id].force_overlay);

    let a = point(&mut state, 1, 10.0);
    let b = point(&mut state, 2, 20.0);
    let array = crate::pine::builtins::array::new_array(&mut state, vec![a, b]);
    let polyline = run(&mut state, "polyline.new", &[arg(array.clone()), forced()]).unwrap();
    let plain_polyline = run(&mut state, "polyline.new", &[arg(array)]).unwrap();
    let (RuntimeValue::Ref(_, polyline_id), RuntimeValue::Ref(_, plain_polyline_id)) =
        (polyline, plain_polyline)
    else {
        panic!()
    };
    assert!(state.polylines[&polyline_id].force_overlay);
    assert!(!state.polylines[&plain_polyline_id].force_overlay);
}

#[test]
fn force_overlay_binds_by_position_and_in_the_point_forms() {
    let mut state = RuntimeState::default();
    let yes = || arg(RuntimeValue::Bool(true));
    let line = run(
        &mut state,
        "line.new",
        &[
            arg(Int(1)),
            arg(Float(10.0)),
            arg(Int(3)),
            arg(Float(20.0)),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            yes(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Line, line_id) = line else {
        panic!()
    };
    assert!(state.lines[&line_id].force_overlay);

    let table = run(
        &mut state,
        "table.new",
        &[
            arg(Int(0)),
            arg(Int(1)),
            arg(Int(1)),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            yes(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Table, table_id) = table else {
        panic!()
    };
    assert!(state.tables[&table_id].force_overlay);

    let first = point(&mut state, 5, 50.0);
    let second = point(&mut state, 9, 90.0);
    let point_line = run(
        &mut state,
        "line.new",
        &[
            arg(first.clone()),
            arg(second.clone()),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            yes(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Line, point_line_id) = point_line else {
        panic!()
    };
    assert!(state.lines[&point_line_id].force_overlay);

    let point_label = run(
        &mut state,
        "label.new",
        &[
            arg(first.clone()),
            arg(Str("hi".into())),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            yes(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Label, point_label_id) = point_label else {
        panic!()
    };
    assert!(state.labels[&point_label_id].force_overlay);

    let point_box = run(
        &mut state,
        "box.new",
        &[
            arg(first),
            arg(second),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            arg(Na),
            yes(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Box, point_box_id) = point_box else {
        panic!()
    };
    assert!(state.boxes[&point_box_id].force_overlay);
}

#[test]
fn copies_keep_force_overlay() {
    let mut state = RuntimeState::default();
    let forced = || named("force_overlay", RuntimeValue::Bool(true));
    let line = run(
        &mut state,
        "line.new",
        &[
            arg(Int(1)),
            arg(Float(10.0)),
            arg(Int(3)),
            arg(Float(20.0)),
            forced(),
        ],
    )
    .unwrap();
    let label = run(
        &mut state,
        "label.new",
        &[arg(Int(1)), arg(Float(10.0)), forced()],
    )
    .unwrap();
    let boxed = run(
        &mut state,
        "box.new",
        &[
            arg(Int(1)),
            arg(Float(10.0)),
            arg(Int(3)),
            arg(Float(20.0)),
            forced(),
        ],
    )
    .unwrap();
    let line_copy = run(&mut state, "line.copy", &[arg(line)]).unwrap();
    let label_copy = run(&mut state, "label.copy", &[arg(label)]).unwrap();
    let box_copy = run(&mut state, "box.copy", &[arg(boxed)]).unwrap();
    let (
        RuntimeValue::Ref(_, line_id),
        RuntimeValue::Ref(_, label_id),
        RuntimeValue::Ref(_, box_id),
    ) = (line_copy, label_copy, box_copy)
    else {
        panic!()
    };
    assert!(state.lines[&line_id].force_overlay);
    assert!(state.labels[&label_id].force_overlay);
    assert!(state.boxes[&box_id].force_overlay);
}

#[test]
fn only_the_setters_that_place_a_point_read_their_first_argument_as_a_time() {
    let mut state = RuntimeState::default();
    let bar_time = || named("xloc", Str("xloc.bar_time".into()));
    // The test bar opens at 0 and lasts a minute, so a time of `n` minutes is bar index `n`.
    let line = run(
        &mut state,
        "line.new",
        &[
            arg(Int(60_000)),
            arg(Float(10.0)),
            arg(Int(120_000)),
            arg(Float(20.0)),
            bar_time(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Line, line_id) = line.clone() else {
        panic!("{line:?}")
    };
    assert!(state.lines[&line_id].time_anchored);
    // A price, a width and a size given as integers are what the script wrote, whatever the
    // drawing is anchored to.
    run(
        &mut state,
        "line.set_y1",
        &[arg(line.clone()), arg(Int(3_000))],
    )
    .unwrap();
    run(
        &mut state,
        "line.set_width",
        &[arg(line.clone()), arg(Int(3))],
    )
    .unwrap();
    assert_eq!(state.lines[&line_id].y1, 3_000.0);
    assert_eq!(state.lines[&line_id].width, 3);
    // A time is still a time.
    run(
        &mut state,
        "line.set_x2",
        &[arg(line.clone()), arg(Int(600_000))],
    )
    .unwrap();
    assert_eq!(state.lines[&line_id].x2, 10);
    // One call that sets both: the x is converted and the y is kept as given.
    run(
        &mut state,
        "line.set_xy1",
        &[arg(line.clone()), arg(Int(180_000)), arg(Int(77))],
    )
    .unwrap();
    assert_eq!(state.lines[&line_id].x1, 3);
    assert_eq!(state.lines[&line_id].y1, 77.0);
    // A chart.point already holds a bar index: it is not read as a time a second time.
    let placed = point(&mut state, 7, 55.0);
    run(
        &mut state,
        "line.set_first_point",
        &[arg(line), arg(placed)],
    )
    .unwrap();
    assert_eq!(state.lines[&line_id].x1, 7);
    assert_eq!(state.lines[&line_id].y1, 55.0);

    let label = run(
        &mut state,
        "label.new",
        &[
            arg(Int(60_000)),
            arg(Float(10.0)),
            arg(Str("hi".into())),
            bar_time(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Label, label_id) = label.clone() else {
        panic!("{label:?}")
    };
    assert!(state.labels[&label_id].time_anchored);
    run(
        &mut state,
        "label.set_size",
        &[arg(label.clone()), arg(Int(12))],
    )
    .unwrap();
    assert_eq!(state.labels[&label_id].size, Size::Points(12));
    run(
        &mut state,
        "label.set_xy",
        &[arg(label), arg(Int(300_000)), arg(Int(55))],
    )
    .unwrap();
    assert_eq!(state.labels[&label_id].x, 5);
    assert_eq!(state.labels[&label_id].y, 55.0);

    let boxed = run(
        &mut state,
        "box.new",
        &[
            arg(Int(60_000)),
            arg(Float(10.0)),
            arg(Int(120_000)),
            arg(Float(20.0)),
            bar_time(),
        ],
    )
    .unwrap();
    let RuntimeValue::Ref(RefKind::Box, box_id) = boxed.clone() else {
        panic!("{boxed:?}")
    };
    run(
        &mut state,
        "box.set_top",
        &[arg(boxed.clone()), arg(Int(4_000))],
    )
    .unwrap();
    run(
        &mut state,
        "box.set_border_width",
        &[arg(boxed), arg(Int(2))],
    )
    .unwrap();
    assert_eq!(state.boxes[&box_id].top, 4_000.0);
    assert_eq!(state.boxes[&box_id].border_width, 2);
}
