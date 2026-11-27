use super::expressions::price_scale;
use super::*;
use crate::pine::DiagnosticCategory;
use crate::pine::Limits;
use crate::pine::SourceRange;
use crate::pine::builtins::array;
use crate::pine::compiler::program::{BarEvent, BarPhase};
use crate::pine::runtime::types::{BarFlags, RuntimeContext};

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
            Some(3_600_000),
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
fn a_live_bar_with_no_history_before_it_is_the_chart_s_first_and_last_bar() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(close)",
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
    session.load_history(&[], Some(7_200_000), None).unwrap();
    assert_eq!(session.last_bar_time, Some(7_200_000));
    assert_eq!(session.first_bar_time, Some(7_200_000));
    assert_eq!(session.last_bar_index, 0);
}

#[test]
fn timeframe_change_rolls_back_with_a_realtime_tick() {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(timeframe.change(\"120\") ? 1 : 0)",
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
        .load_history(&[hourly(0), hourly(3_600_000)], Some(7_200_000), None)
        .unwrap();
    let last_plotted = |session: &mut Session, hour: i64, phase: BarPhase| {
        session
            .execute(
                BarEvent {
                    candle: hourly(hour * 3_600_000),
                    phase,
                },
                true,
                None,
            )
            .unwrap();
        session.output().plots[0].values.last().copied().flatten()
    };
    // Hour 2 opens a new two-hour bucket, on every tick and on the close.
    for phase in [
        BarPhase::RealtimeTick { is_new: true },
        BarPhase::RealtimeTick { is_new: false },
        BarPhase::RealtimeClose { is_new: false },
    ] {
        assert_eq!(last_plotted(&mut session, 2, phase), Some(1.0));
    }
    // Hour 3 shares it: the call compares with hour 2, the bar the closed tick committed.
    assert_eq!(
        last_plotted(&mut session, 3, BarPhase::RealtimeTick { is_new: true }),
        Some(0.0)
    );
}

#[test]
fn timeframe_change_ignores_a_call_only_an_unconfirmed_tick_made() {
    // The call runs on every bar but the close of hour 2. Hour 2's tick sees a new two-hour bucket
    // against hour 1, the last committed call, and is thrown away with the tick: hour 3 compares
    // with hour 1 again (a new bucket), not with the tick's hour 2 (the same one).
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\n\
         skipped = barstate.isconfirmed and hour == 2\n\
         plot((skipped ? false : timeframe.change(\"120\")) ? 1 : 0)",
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
        .load_history(&[hourly(0), hourly(3_600_000)], Some(7_200_000), None)
        .unwrap();
    let last_plotted = |session: &mut Session, hour: i64, phase: BarPhase| {
        session
            .execute(
                BarEvent {
                    candle: hourly(hour * 3_600_000),
                    phase,
                },
                true,
                None,
            )
            .unwrap();
        session.output().plots[0].values.last().copied().flatten()
    };
    assert_eq!(
        last_plotted(&mut session, 2, BarPhase::RealtimeTick { is_new: true }),
        Some(1.0)
    );
    assert_eq!(
        last_plotted(&mut session, 2, BarPhase::RealtimeClose { is_new: false }),
        Some(0.0),
        "the close skips the call"
    );
    assert_eq!(
        last_plotted(&mut session, 3, BarPhase::RealtimeTick { is_new: true }),
        Some(1.0),
        "compared with hour 1, the last bar the call was committed on"
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
    session
        .load_history(&[hourly(0)], Some(3_600_000), None)
        .unwrap();
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

/// A method call evaluates its receiver first, so a limit can be reached exactly there. It has to
/// stop the script as that limit, not read as a function the engine does not know.
#[test]
fn a_limit_reached_at_a_method_receiver_keeps_its_own_error() {
    let source = "//@version=6\nindicator(\"T\")\n\
        levels = array.new_int(0)\n\
        levels.push(1)\n\
        line gone = na\n\
        gone.delete()\n\
        plot(levels.size())";
    let program = crate::pine::compiler::compile(
        source,
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let mut stopped = 0;
    for instructions_per_bar in 0..100 {
        let mut session = Session::new(
            program.clone(),
            HashMap::new(),
            Limits {
                instructions_per_bar,
                ..Limits::default()
            },
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        match session.evaluate(&[hourly(0)]) {
            Ok(_) => {}
            Err(RunError::Script(error)) => {
                assert_eq!(error.code, "PINE8004", "{instructions_per_bar}: {error:?}");
                stopped += 1;
            }
            Err(RunError::Rejection) => panic!("not a rejection"),
        }
    }
    assert!(stopped > 0, "no limit in the sweep stopped the script");
}

/// An empty script's session, and a bar to call one of its builtins on directly. Nothing has
/// been charged yet, so the first read a call makes is also the first check of the limits.
fn idle_session() -> (Session, RuntimeContext) {
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")",
        Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    (session, RuntimeContext::new(bar(), BarFlags::default()))
}

fn call_without_arguments(
    session: &mut Session,
    name: &str,
    context: &mut RuntimeContext,
) -> Result<RuntimeValue, Diagnostic> {
    session.call(name, &[], None, 0, SourceRange::ZERO, context)
}

#[test]
fn a_deadline_or_cancellation_at_a_method_receiver_keeps_its_own_error() {
    let (mut late, mut context) = idle_session();
    let levels = array::new_array(&mut late.working, Vec::new());
    late.working.variables.insert("levels".into(), levels);
    late.deadline_at = Some(Instant::now());
    let deadline = call_without_arguments(&mut late, "levels.size", &mut context).unwrap_err();
    assert_eq!(deadline.code, "PINE8007", "{deadline:?}");

    let (mut cancelled, mut context) = idle_session();
    let levels = array::new_array(&mut cancelled.working, Vec::new());
    cancelled.working.variables.insert("levels".into(), levels);
    cancelled.set_cancellation(Arc::new(AtomicBool::new(true)));
    let error = call_without_arguments(&mut cancelled, "levels.size", &mut context).unwrap_err();
    assert_eq!(error.code, "PINE8008", "{error:?}");
}

/// `line.all` builds a new array each time it is read.
#[test]
fn a_dotted_method_call_reads_its_receiver_once() {
    let (mut session, mut context) = idle_session();
    let before = session.working.next_reference;
    let size = call_without_arguments(&mut session, "line.all.size", &mut context).unwrap();
    assert_eq!(size, RuntimeValue::Int(0));
    assert_eq!(session.working.next_reference, before + 1, "arrays built");
    assert_eq!(session.working.instructions, 1, "instructions charged");
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

#[test]
fn timeframe_change_with_a_varying_timeframe_rolls_back_with_realtime_ticks() {
    // The live bar names an hour or two by its own close, which moves between ticks: every tick
    // compares with what the last committed bar found, never with an earlier tick of its own bar.
    let program = crate::pine::compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(timeframe.change(close > 10 ? \"60\" : \"120\") ? 1 : 0)",
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
    let priced = |hour: i64, close: f64| Kline {
        close,
        ..hourly(hour * 3_600_000)
    };
    // Hour 2 commits the two-hour period that opens at hour 2 (close below ten).
    session
        .load_history(&[priced(1, 5.0), priced(2, 5.0)], Some(3 * 3_600_000), None)
        .unwrap();
    let run = |session: &mut Session, hour: i64, close: f64, phase: BarPhase| {
        session
            .execute(
                BarEvent {
                    candle: priced(hour, close),
                    phase,
                },
                true,
                None,
            )
            .unwrap();
        session.output().plots[0].values.last().copied().flatten()
    };
    // Hour 3: "120" finds the period hour 2 committed, "60" finds hour 3's own.
    let tick = BarPhase::RealtimeTick { is_new: false };
    assert_eq!(
        run(
            &mut session,
            3,
            5.0,
            BarPhase::RealtimeTick { is_new: true }
        ),
        Some(0.0)
    );
    assert_eq!(run(&mut session, 3, 20.0, tick), Some(1.0));
    assert_eq!(
        run(&mut session, 3, 5.0, tick),
        Some(0.0),
        "the tick before left nothing behind"
    );
    assert_eq!(
        run(
            &mut session,
            3,
            20.0,
            BarPhase::RealtimeClose { is_new: false }
        ),
        Some(1.0)
    );
    // Hour 4 compares with hour 3, which closed naming "60": hour 4's two-hour period differs.
    assert_eq!(
        run(
            &mut session,
            4,
            5.0,
            BarPhase::RealtimeTick { is_new: true }
        ),
        Some(1.0)
    );
}
