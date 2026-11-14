use super::expressions::price_scale;
use super::*;
use crate::pine::DiagnosticCategory;
use crate::pine::Limits;
use crate::pine::compiler::program::{BarEvent, BarPhase};

fn bar() -> Kline {
    Kline {
        open_time: 1_000,
        close_time: 2_000,
        open: 100.0,
        high: 110.0,
        low: 90.0,
        close: 105.0,
        volume: 1.0,
    }
}

#[test]
fn evaluates_a_declaration() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nx = 1 + 2\n",
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    let result = session.evaluate(std::slice::from_ref(&bar())).unwrap();
    assert_eq!(result.bar_states.len(), 1);
    assert_eq!(
        session.working.variables.get("x"),
        Some(&RuntimeValue::Int(3))
    );
}

#[test]
fn evaluates_control_flow() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nx = 0\nif close > open\n    x := 1\n",
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    session.evaluate(std::slice::from_ref(&bar())).unwrap();
    assert_eq!(
        session.working.variables.get("x"),
        Some(&RuntimeValue::Int(1))
    );
}

/// `source` run over `bars`, one confirmed bar at a time, leaving the session to inspect.
fn run_over(source: &str, bars: &[Kline]) -> Session {
    let program = crate::pine::compiler::compile(
        source,
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    session.evaluate(bars).unwrap();
    session
}

fn hourly(open_time: i64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + 3_600_000,
        ..bar()
    }
}

#[test]
fn time_call_buckets_to_the_timeframe_and_gates_on_a_session() {
    const DAY_MS: i64 = 86_400_000;
    // 2023-11-14 12:00 UTC (a Tuesday) and the bar before it.
    let noon = 1_699_963_200_000;
    let source = "//@version=6\nindicator(\"T\")\n\
        day = time(\"D\")\n\
        own = time(timeframe.period)\n\
        inside = time(timeframe.period, \"0930-1600\")\n\
        outside = time(timeframe.period, \"1300-1600\")\n\
        shifted = time(timeframe.period, \"1300-1600\", \"UTC+2\")";
    let session = run_over(source, &[hourly(noon - 3_600_000), hourly(noon)]);
    let variable = |name: &str| session.working.variables.get(name).cloned();
    assert_eq!(
        variable("day"),
        Some(RuntimeValue::Int(noon.div_euclid(DAY_MS) * DAY_MS))
    );
    assert_eq!(variable("own"), Some(RuntimeValue::Int(noon)));
    assert_eq!(variable("inside"), Some(RuntimeValue::Int(noon)));
    assert_eq!(variable("outside"), Some(RuntimeValue::Na));
    // 12:00 UTC is 14:00 in UTC+2, inside 13:00-16:00 there.
    assert_eq!(variable("shifted"), Some(RuntimeValue::Int(noon)));
}

#[test]
fn calendar_parts_take_a_time_and_a_zone() {
    let noon = 1_699_963_200_000;
    let source = "//@version=6\nindicator(\"T\")\n\
        own = hour\n\
        utc = hour(time)\n\
        shifted = hour(time, \"UTC+3\")";
    let session = run_over(source, &[hourly(noon)]);
    let variable = |name: &str| session.working.variables.get(name).cloned();
    assert_eq!(variable("own"), Some(RuntimeValue::Int(12)));
    assert_eq!(variable("utc"), Some(RuntimeValue::Int(12)));
    assert_eq!(variable("shifted"), Some(RuntimeValue::Int(15)));
}

#[test]
fn drawing_methods_work_on_variables_and_call_results() {
    let source = "//@version=6\nindicator(\"T\", overlay = true)\n\
        make() =>\n    line.new(bar_index, close, bar_index + 1, close)\n\
        ln = line.new(bar_index, open, bar_index + 1, open)\n\
        ln.set_extend(extend.right)\n\
        make().set_extend(extend.both)\n\
        line.new(bar_index, low, bar_index + 1, low).set_extend(extend.left)";
    let session = run_over(source, &[hourly(1_000_000)]);
    let mut extends: Vec<_> = session
        .output()
        .lines
        .iter()
        .map(|line| format!("{:?}", line.extend))
        .collect();
    extends.sort();
    assert_eq!(extends, ["Both", "Left", "Right"]);
}

#[test]
fn drawing_calls_honor_the_limits_from_the_script_declaration() {
    let source = "//@version=6\nindicator(\"T\", overlay = true, max_lines_count = 2)\n\
        line.new(bar_index, close, bar_index + 1, close)";
    let session = run_over(source, &[hourly(0), hourly(3_600_000), hourly(7_200_000)]);

    assert_eq!(session.output().lines.len(), 2);
    assert_eq!(session.output().lines[0].x1, 1);
    assert_eq!(session.output().lines[1].x1, 2);
}

#[test]
fn array_call_arguments_are_evaluated_once() {
    let source = "//@version=6\nindicator(\"T\")\n\
        var side = array.new_int()\n\
        var target = array.new_int()\n\
        value() =>\n    array.push(side, 1)\n    7\n\
        array.push(target, value())\n\
        plot(array.size(side))";
    let session = run_over(source, &[hourly(0)]);

    assert_eq!(session.output().plots[0].values, [Some(1.0)]);
}

#[test]
fn canonical_math_and_string_builtins_are_reachable_from_scripts() {
    let source = "//@version=6\nindicator(\"T\")\n\
        matched = str.match(\"price=123\", \"[0-9]+\")\n\
        parts = str.split(\"a,b,c\", \",\")\n\
        plot(str.length(matched))\n\
        plot(array.size(parts))\n\
        plot(math.sum(close, 2))";
    let session = run_over(source, &[hourly(0), hourly(3_600_000)]);
    let plots = session.output().plots;

    assert_eq!(plots[0].values.last(), Some(&Some(3.0)));
    assert_eq!(plots[1].values.last(), Some(&Some(3.0)));
    assert_eq!(plots[2].values.last(), Some(&Some(210.0)));
}

#[test]
fn ta_overloads_use_default_sources_and_linreg_offset() {
    let source = "//@version=6\nindicator(\"T\")\n\
        plot(ta.highest(2))\n\
        plot(ta.lowest(length = 2))\n\
        plot(ta.highestbars(2))\n\
        plot(ta.linreg(close, 4, 1))";
    let bars = [
        Kline {
            high: 2.0,
            low: 1.0,
            close: 2.0,
            ..hourly(0)
        },
        Kline {
            high: 4.0,
            low: 3.0,
            close: 4.0,
            ..hourly(3_600_000)
        },
        Kline {
            high: 7.0,
            low: 2.0,
            close: 6.0,
            ..hourly(7_200_000)
        },
        Kline {
            high: 5.0,
            low: 4.0,
            close: 8.0,
            ..hourly(10_800_000)
        },
    ];
    let plots = run_over(source, &bars).output().plots;

    assert_eq!(plots[0].values.last(), Some(&Some(7.0)));
    assert_eq!(plots[1].values.last(), Some(&Some(2.0)));
    assert_eq!(plots[2].values.last(), Some(&Some(-1.0)));
    assert_eq!(plots[3].values.last(), Some(&Some(6.0)));
}

#[test]
fn the_price_scale_makes_the_tick_a_whole_number_of_units() {
    assert_eq!(price_scale(0.01), Some((1, 100)));
    assert_eq!(price_scale(0.25), Some((25, 100)));
    assert_eq!(price_scale(0.05), Some((5, 100)));
    assert_eq!(price_scale(0.0001), Some((1, 10_000)));
    assert_eq!(price_scale(5.0), Some((5, 1)));
    assert_eq!(price_scale(1.0), Some((1, 1)));
    assert_eq!(price_scale(0.0), None);
    assert_eq!(price_scale(f64::NAN), None);
    assert_eq!(price_scale(-1.0), None);
    assert_eq!(price_scale(1e-10), Some((1, 10_000_000_000)));
    assert_eq!(price_scale(1e-13), None, "finer than twelve decimals");
}

#[test]
fn realtime_ticks_do_not_advance_committed_ta_history() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(ta.sma(close, 2))",
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    session
        .load_history(
            &[Kline {
                close: 10.0,
                ..hourly(0)
            }],
            true,
            None,
        )
        .unwrap();
    for close in [20.0, 30.0] {
        session
            .execute(
                BarEvent {
                    candle: Kline {
                        close,
                        ..hourly(3_600_000)
                    },
                    phase: BarPhase::RealtimeTick {
                        is_new: close == 20.0,
                    },
                },
                true,
                None,
            )
            .unwrap();
    }
    session
        .execute(
            BarEvent {
                candle: Kline {
                    close: 40.0,
                    ..hourly(3_600_000)
                },
                phase: BarPhase::RealtimeClose { is_new: false },
            },
            true,
            None,
        )
        .unwrap();

    assert_eq!(session.output().plots[0].values.last(), Some(&Some(25.0)));
    assert!(
        session
            .working
            .ta_states
            .values()
            .all(|state| state.inputs.len() == 2),
        "only the historical bar and confirmed realtime bar should be committed"
    );
}

#[test]
fn once_stays_active_through_realtime_ticks_and_finishes_when_the_bar_closes() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nvarip int runs = 0\n\
         once barstate.isrealtime\n    runs += 1\nplot(runs)",
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    session.load_history(&[hourly(0)], true, None).unwrap();
    let runs_after = |session: &mut Session, phase: BarPhase, open_time: i64| {
        session
            .execute(
                BarEvent {
                    candle: hourly(open_time),
                    phase,
                },
                true,
                None,
            )
            .unwrap();
        session.output().plots[0].values.last().copied().flatten()
    };
    // Two ticks of the open bar each run the block: rollback keeps the structure active.
    assert_eq!(
        runs_after(
            &mut session,
            BarPhase::RealtimeTick { is_new: true },
            3_600_000
        ),
        Some(1.0)
    );
    assert_eq!(
        runs_after(
            &mut session,
            BarPhase::RealtimeTick { is_new: false },
            3_600_000
        ),
        Some(2.0)
    );
    // It runs on the closing tick and is then finished for good.
    assert_eq!(
        runs_after(
            &mut session,
            BarPhase::RealtimeClose { is_new: false },
            3_600_000
        ),
        Some(3.0)
    );
    assert_eq!(
        runs_after(
            &mut session,
            BarPhase::RealtimeTick { is_new: true },
            7_200_000
        ),
        Some(3.0)
    );
}

#[test]
fn evaluation_enforces_deadlines_and_cooperative_cancellation() {
    let source = "//@version=6\nindicator(\"T\")\nplot(close)";
    let program = crate::pine::compiler::compile(
        source,
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut deadline_session = Session::new(
        program.clone(),
        HashMap::new(),
        Limits {
            deadline_seconds: 0.0,
            ..Limits::default()
        },
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    let Err(RunError::Script(deadline)) = deadline_session.evaluate(&[hourly(0)]) else {
        panic!("a zero deadline must stop evaluation");
    };
    assert_eq!(deadline.code, "PINE8007");

    let mut cancelled_session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    cancelled_session.set_cancellation(Arc::new(AtomicBool::new(true)));
    let Err(RunError::Script(cancelled)) = cancelled_session.evaluate(&[hourly(0)]) else {
        panic!("a raised cancellation flag must stop evaluation");
    };
    assert_eq!(cancelled.code, "PINE8008");
    assert_eq!(cancelled.category, DiagnosticCategory::Cancellation);
}

#[test]
fn strategy_readings_follow_the_broker_and_keep_a_history() {
    let source = "//@version=6\nstrategy(\"T\")\n\
        if bar_index == 1\n    strategy.entry(\"L\", strategy.long)\n\
        size = strategy.position_size\n\
        before = strategy.position_size[1]\n\
        avg = strategy.position_avg_price";
    let session = run_over(
        source,
        &[
            hourly(0),
            hourly(3_600_000),
            hourly(7_200_000),
            hourly(10_800_000),
        ],
    );
    let variable = |name: &str| session.working.variables.get(name).cloned();
    // Entered on bar 1, filled at bar 2's open; bar 3 sees the position and its history.
    assert!(matches!(variable("size"), Some(RuntimeValue::Float(size)) if size > 0.0));
    assert!(matches!(variable("before"), Some(RuntimeValue::Float(size)) if size > 0.0));
    assert!(matches!(variable("avg"), Some(RuntimeValue::Float(_))));
}
