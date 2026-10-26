//! End-to-end checks of language features that real scripts lean on: each test compiles a small
//! original script, runs it over a few bars and reads the variables the last bar left behind.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;
use crate::pine::token::Limits;

fn bars(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let price = 100.0 + index as f64;
            Kline {
                open_time: index as i64 * 60_000,
                close_time: (index as i64 + 1) * 60_000,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 1.0,
                volume: 10.0,
            }
        })
        .collect()
}

/// Compiles and runs `body` (after the version and declaration lines) over `count` bars.
fn run(body: &str, count: usize) -> Session {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
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
    if let Err(error) = session.evaluate(&bars(count)) {
        panic!("{error:?}");
    }
    session
}

fn value(session: &Session, name: &str) -> RuntimeValue {
    session
        .working
        .variables
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("no variable {name}"))
}

#[test]
fn a_bool_field_drives_a_condition() {
    let session = run(
        "type Zone\n    float top\n    bool active\n\
         z = Zone.new(5.0, true)\n\
         a = z.active ? 1 : 0\n\
         b = 0\n\
         if z.active\n    b := 2\n\
         c = z.top",
        2,
    );
    assert_eq!(value(&session, "a"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "b"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "c"), RuntimeValue::Float(5.0));
}

#[test]
fn a_dotted_field_can_be_assigned_and_updated() {
    let session = run(
        "type Counter\n    int hits\n    bool on\n\
         var c = Counter.new(0, true)\n\
         c.hits += 1\n\
         c.on := c.hits < 3\n\
         total = c.hits\n\
         still = c.on",
        5,
    );
    assert_eq!(value(&session, "total"), RuntimeValue::Int(5));
    assert_eq!(value(&session, "still"), RuntimeValue::Bool(false));
}

#[test]
fn fields_of_nested_objects_read_and_write_through_the_chain() {
    let session = run(
        "type Inner\n    float level\n\
         type Outer\n    Inner inner\n\
         var o = Outer.new(Inner.new(1.0))\n\
         o.inner.level := o.inner.level + 2.0\n\
         seen = o.inner.level",
        1,
    );
    assert_eq!(value(&session, "seen"), RuntimeValue::Float(3.0));
}

#[test]
fn a_method_runs_on_its_object_in_both_call_styles() {
    let session = run(
        "type Box2\n    float w\n    float h\n\
         method area(Box2 this) =>\n    this.w * this.h\n\
         method grow(Box2 this, float by) =>\n    this.w += by\n    this\n\
         b = Box2.new(2.0, 3.0)\n\
         first = b.area()\n\
         grown = b.grow(1.0)\n\
         again = area(b)\n\
         width = b.w",
        1,
    );
    assert_eq!(value(&session, "first"), RuntimeValue::Float(6.0));
    assert_eq!(value(&session, "again"), RuntimeValue::Float(9.0));
    assert_eq!(value(&session, "width"), RuntimeValue::Float(3.0));
}

#[test]
fn method_overloads_are_chosen_by_the_receiver_type() {
    let session = run(
        "type Dog\n    int legs\n\
         type Bird\n    int legs\n\
         method describe(Dog this) =>\n    \"dog\"\n\
         method describe(Bird this) =>\n    \"bird\"\n\
         method describe(int this) =>\n    \"int\"\n\
         method describe(string this) =>\n    \"string\"\n\
         d = Dog.new(4).describe()\n\
         b = describe(Bird.new(2))\n\
         i = (3).describe()\n\
         s = \"x\".describe()",
        1,
    );
    assert_eq!(value(&session, "d"), RuntimeValue::String("dog".into()));
    assert_eq!(value(&session, "b"), RuntimeValue::String("bird".into()));
    assert_eq!(value(&session, "i"), RuntimeValue::String("int".into()));
    assert_eq!(value(&session, "s"), RuntimeValue::String("string".into()));
}

#[test]
fn a_method_on_a_variable_of_a_builtin_type() {
    let session = run(
        "method twice(float this) =>\n    this * 2.0\n\
         x = close\n\
         y = x.twice()",
        3,
    );
    assert_eq!(value(&session, "y"), RuntimeValue::Float(2.0 * 103.0));
}

fn close_to(session: &Session, name: &str, expected: f64) {
    let RuntimeValue::Float(actual) = value(session, name) else {
        panic!("{name} is {:?}", value(session, name));
    };
    assert!(
        (actual - expected).abs() < 1e-9,
        "{name}: {actual} != {expected}"
    );
}

#[test]
fn windowed_statistics_follow_the_reference_formulas() {
    // Five bars: close 101…105, high close+1, low close-3, volume 10.
    let session = run(
        "alma = ta.alma(close, 3, 0.85, 6)\n\
         nearest = ta.percentile_nearest_rank(close, 5, 90)\n\
         linear = ta.percentile_linear_interpolation(close, 5, 90)\n\
         median = ta.percentile_linear_interpolation(close, 5, 50)\n\
         common = ta.mode(close % 2 == 0 ? 1.0 : 2.0, 5)\n\
         gravity = ta.cog(close, 2)\n\
         weighted = ta.vwma(close, 3)\n\
         every = ta.valuewhen(bar_index % 2 == 0, close, 0)\n\
         before = ta.valuewhen(bar_index % 2 == 0, close, 1)",
        5,
    );
    close_to(&session, "alma", 103.314_326_399_761_74);
    close_to(&session, "nearest", 105.0);
    close_to(&session, "linear", 104.6);
    close_to(&session, "median", 103.0);
    // close 101…105: odd values 101, 103, 105 map to 2.0 (three of them), even ones to 1.0.
    close_to(&session, "common", 2.0);
    // -(105 * 1 + 104 * 2) / (105 + 104)
    close_to(&session, "gravity", -(105.0 + 208.0) / 209.0);
    close_to(&session, "weighted", 104.0);
    close_to(&session, "every", 105.0);
    close_to(&session, "before", 103.0);
}

#[test]
fn range_indicators_read_the_bar_range() {
    let session = run(
        "stoch = ta.stoch(close, high, low, 3)\n\
         wpr = ta.wpr(3)\n\
         [basis, upper, lower] = ta.kc(close, 3, 2)\n\
         width = ta.kcw(close, 3, 2)\n\
         strength = ta.tsi(close, 2, 3)\n\
         flow = ta.mfi(close, 3)",
        5,
    );
    // Highs 104…106 → highest 106; lows 98+2…: lowest 100; close 105.
    close_to(&session, "stoch", 100.0 * 5.0 / 6.0);
    close_to(&session, "wpr", -100.0 / 6.0);
    // EMA(3) of close 101…105 seeds at 102 and ends at 104; the true range is 4 on every bar.
    close_to(&session, "basis", 104.0);
    close_to(&session, "upper", 112.0);
    close_to(&session, "lower", 96.0);
    close_to(&session, "width", 16.0 / 104.0);
    // Every change is +1, so the doubly smoothed momentum equals the smoothed magnitude.
    close_to(&session, "strength", 1.0);
    // Only rising closes: no money flows out.
    close_to(&session, "flow", 100.0);
}

#[test]
fn cumulative_volume_variables_catch_up_on_skipped_bars() {
    // Read only on the last bar: the state must still fold in the bars before it.
    let session = run(
        "var float seen_obv = na\n\
         var float seen_acc = na\n\
         var float seen_wvad = na\n\
         if barstate.islast\n    seen_obv := ta.obv\n    seen_acc := ta.accdist\n    seen_wvad := ta.wvad",
        5,
    );
    close_to(&session, "seen_obv", 40.0);
    // (2 * close - low - high) / (high - low) * volume is 2 / 4 * 10 = 5 on every bar, five bars.
    close_to(&session, "seen_acc", 25.0);
    // (close - open) / (high - low) * volume = 1 / 4 * 10 on the last bar.
    close_to(&session, "seen_wvad", 2.5);
}

/// The codes of every diagnostic the compiler reports for `body`, errors and warnings alike.
fn diagnostic_codes(body: &str) -> Vec<String> {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    compiler::compile(&source, Limits::default(), &NoLibraries)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn stateless_builtins_return_what_the_app_can_know() {
    let session = run(
        "max_bars_back(close, 500)\n\
         pair = ticker.new(\"BINANCE\", \"BTCUSDT\")\n\
         plain = ticker.standard(\"BINANCE:ETHUSDT\")\n\
         kept = ticker.inherit(\"A:B\", \"C:D\")\n\
         var box slot = box(na)\n\
         empty = na(slot)\n\
         same = request.currency_rate(\"USD\", \"USD\")\n\
         other = request.currency_rate(\"USD\", \"EUR\")\n\
         seed = request.seed(\"repo\", \"SYM\", close)\n\
         log.info(\"hello {0}\", close)",
        2,
    );
    assert_eq!(
        value(&session, "pair"),
        RuntimeValue::String("BINANCE:BTCUSDT".into())
    );
    assert_eq!(
        value(&session, "plain"),
        RuntimeValue::String("BINANCE:ETHUSDT".into())
    );
    assert_eq!(value(&session, "kept"), RuntimeValue::String("C:D".into()));
    assert_eq!(value(&session, "empty"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "same"), RuntimeValue::Float(1.0));
    assert_eq!(value(&session, "other"), RuntimeValue::Na);
    assert_eq!(value(&session, "seed"), RuntimeValue::Na);
}

#[test]
fn what_cannot_be_honoured_is_warned_about() {
    let codes = diagnostic_codes(
        "a = request.seed(\"repo\", \"SYM\", close)\n\
         b = request.economic(\"US\", \"GDP\")\n\
         log.error(\"x\")\n\
         plot(close)",
    );
    assert_eq!(
        codes.iter().filter(|c| *c == "PINE7005").count(),
        2,
        "{codes:?}"
    );
    // `log.*()` writes messages the Pine logs pane shows, so nothing about it is ignored.
    assert!(!codes.contains(&"PINE7002".to_owned()), "{codes:?}");
    assert!(!codes.contains(&"PINE9003".to_owned()), "{codes:?}");
    // A request the app has no feed for and no stand-in stays a compile error.
    let unknown = diagnostic_codes("f = request.volume_flow(1)\nplot(close)");
    assert!(unknown.contains(&"PINE9003".to_owned()), "{unknown:?}");
}

#[test]
fn a_parameter_may_reuse_a_builtin_function_name_with_a_warning() {
    let codes = diagnostic_codes("f(color fill) =>\n    fill\nplot(close)");
    assert_eq!(codes, ["PINE7006"]);
    // A parameter named like a builtin variable is a warning too; a builtin function's name as a variable is not allowed.
    let variable = diagnostic_codes("f(float close) =>\n    close\nplot(f(1.0))");
    assert_eq!(variable, ["PINE7006"]);
    let reserved = diagnostic_codes("plot = 1\nplot(close)");
    assert!(reserved.contains(&"CE10190".to_owned()), "{reserved:?}");
}

#[test]
fn int_truncates_toward_zero() {
    let session = run(
        "up = int(2.9)\n\
         down = int(-2.9)\n\
         whole = int(7)\n\
         gone = int(na)\n\
         bucket = math.min(5, int((close - 100.0) / 2.0))",
        4,
    );
    assert_eq!(value(&session, "up"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "down"), RuntimeValue::Int(-2));
    assert_eq!(value(&session, "whole"), RuntimeValue::Int(7));
    assert_eq!(value(&session, "gone"), RuntimeValue::Na);
    // close is 104 on the last bar: (104 - 100) / 2 = 2.
    assert_eq!(value(&session, "bucket"), RuntimeValue::Int(2));
}

#[test]
fn a_string_may_run_over_several_lines() {
    let session = run(
        "tip = \"first line\n second line\"\n\
         after = 1 + 2\n\
         plot(close, title = tip)",
        1,
    );
    assert_eq!(
        value(&session, "tip"),
        RuntimeValue::String("first line\n second line".to_owned())
    );
    assert_eq!(value(&session, "after"), RuntimeValue::Int(3));
    // A quote that is never closed still reports where it started.
    let codes = diagnostic_codes("broken = \"never closed\nplot(close)\nx = 1\ny = 2");
    assert!(codes.contains(&"PINE1003".to_owned()), "{codes:?}");
}

#[test]
fn a_function_declared_twice_is_picked_by_its_argument_types() {
    let session = run(
        "label_of(float value) =>\n    \"number\"\n\
         label_of(string text, bool loud = false) =>\n    loud ? \"SHOUT\" : \"text\"\n\
         label_of(float lower, float upper) =>\n    \"range\"\n\
         a = label_of(1.5)\n\
         b = label_of(\"hi\")\n\
         c = label_of(\"hi\", true)\n\
         d = label_of(1.0, 2.0)\n\
         e = label_of(text = \"named\")",
        1,
    );
    let text = |name: &str| value(&session, name);
    assert_eq!(text("a"), RuntimeValue::String("number".into()));
    assert_eq!(text("b"), RuntimeValue::String("text".into()));
    assert_eq!(text("c"), RuntimeValue::String("SHOUT".into()));
    assert_eq!(text("d"), RuntimeValue::String("range".into()));
    assert_eq!(text("e"), RuntimeValue::String("text".into()));
    // The same signature twice is still an error.
    let codes = diagnostic_codes("f(float x) =>\n    1\nf(float y) =>\n    2\nplot(close)");
    assert!(codes.contains(&"PINE3024".to_owned()), "{codes:?}");
}

#[test]
fn var_inside_a_function_initialises_once_per_call_site() {
    let session = run(
        "counter() =>\n    var int n = 10\n    n += 1\n    n\n\
         first = counter()\n\
         again = counter()",
        3,
    );
    // Each call site keeps its own `n`: 10 + one increment per bar.
    assert_eq!(value(&session, "first"), RuntimeValue::Int(13));
    assert_eq!(value(&session, "again"), RuntimeValue::Int(13));
}

#[test]
fn history_offsets_that_reach_no_bar_are_na() {
    let session = run(
        "gone = close[na]\n\
         far = close[100]\n\
         back = close[1]\n\
         offset = bar_index > 1 ? 2 : na\n\
         dynamic = close[offset]",
        4,
    );
    assert_eq!(value(&session, "gone"), RuntimeValue::Na);
    assert_eq!(value(&session, "far"), RuntimeValue::Na);
    assert_eq!(value(&session, "back"), RuntimeValue::Float(103.0));
    assert_eq!(value(&session, "dynamic"), RuntimeValue::Float(102.0));
}

#[test]
fn drawing_collections_list_what_is_alive() {
    let session = run(
        "var box first = box.new(0, 2.0, 1, 1.0)\n\
         if bar_index == 2\n    box.new(2, 3.0, 3, 2.0)\n\
         if bar_index == 3\n    box.delete(first)\n\
         count = array.size(box.all)\n\
         none = array.size(table.all)",
        4,
    );
    assert_eq!(value(&session, "count"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "none"), RuntimeValue::Int(0));
}

#[test]
fn an_unknown_member_of_a_variable_namespace_is_an_error_not_a_string() {
    let source = "//@version=6\nindicator(\"T\")\nx = ta.nonexistent\nplot(close)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    let Err(crate::pine::session::RunError::Script(diagnostic)) = session.evaluate(&bars(2)) else {
        panic!("expected a script error");
    };
    assert_eq!(diagnostic.code, "PINE4008");
    assert!(diagnostic.message.contains("ta.nonexistent"));
    // Named constants of the other namespaces still read as themselves.
    let session = run("style = label.style_label_up\nplot(close)", 1);
    assert_eq!(
        value(&session, "style"),
        RuntimeValue::String("label.style_label_up".into())
    );
}

#[test]
fn a_method_can_take_an_enum_receiver() {
    let session = run(
        "enum Side\n    long\n    short\n\
         method sign(Side this) =>\n    this == Side.long ? 1 : -1\n\
         up = Side.long.sign()\n\
         down = sign(Side.short)",
        1,
    );
    assert_eq!(value(&session, "up"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "down"), RuntimeValue::Int(-1));
}

#[test]
fn named_time_zones_follow_daylight_saving_time() {
    // The first bar opens at the epoch, 00:00 UTC: 19:00 the day before in New York (standard time).
    let session = run(
        "ny = hour(time, \"America/New_York\")\n\
         tokyo = hour(time, \"Asia/Tokyo\")\n\
         plain = hour(time, \"UTC-5\")\n\
         open930 = timestamp(\"America/New_York\", 2024, 7, 15, 9, 30)\n\
         shown = str.format_time(open930, \"HH:mm\", \"America/New_York\")",
        1,
    );
    assert_eq!(value(&session, "ny"), RuntimeValue::Int(19));
    assert_eq!(value(&session, "tokyo"), RuntimeValue::Int(9));
    assert_eq!(value(&session, "plain"), RuntimeValue::Int(19));
    // 09:30 on a July day in New York is 13:30 UTC.
    assert_eq!(
        value(&session, "open930"),
        RuntimeValue::Int(1_721_050_200_000)
    );
    assert_eq!(
        value(&session, "shown"),
        RuntimeValue::String("09:30".into())
    );
}

#[test]
fn a_zone_the_table_lacks_is_warned_about() {
    let codes = diagnostic_codes(
        "z = input.string(\"Asia/Jerusalem\", \"zone\")\nok = hour(time, \"Europe/Paris\")\nplot(close)",
    );
    assert_eq!(codes, ["PINE7007"]);
}

#[test]
fn ratios_with_nothing_to_divide_are_na_not_nan() {
    // A flat market: no money flow in either direction, and a zero-width range.
    let session = run(
        "flat = 100.0\n\
         flow = ta.mfi(flat, 3)\n\
         pos = ta.stoch(flat, flat, flat, 3)\n\
         plain = ta.wpr(3)",
        5,
    );
    assert_eq!(value(&session, "flow"), RuntimeValue::Na);
    assert_eq!(value(&session, "pos"), RuntimeValue::Na);
    // The bars themselves still span 4 points, so %R is defined.
    assert!(matches!(value(&session, "plain"), RuntimeValue::Float(_)));
}

#[test]
fn strategy_settings_the_engine_cannot_honour_are_warnings_not_errors() {
    let source = "//@version=6\nstrategy(\"S\", overlay = true, use_bar_magnifier = true, fill_orders_on_standard_ohlc = true, backtest_fill_limits_assumption = 2, risk_free_rate = 1.5)\nplot(close)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut codes: Vec<&str> = program
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    codes.sort_unstable();
    // magnifier, fill limits and risk-free rate are warned about; standard-OHLC fills are a no-op here.
    assert_eq!(codes, ["PINE7001", "PINE7001", "PINE7001"]);
    let session = run(
        "amount = strategy.convert_to_account(12.5)\nweek = weekofyear(time)",
        1,
    );
    assert_eq!(value(&session, "amount"), RuntimeValue::Float(12.5));
    assert_eq!(value(&session, "week"), RuntimeValue::Na);
    let codes =
        diagnostic_codes("a = strategy.convert_to_symbol(1)\nb = weekofyear(time)\nplot(close)");
    assert_eq!(
        codes.iter().filter(|c| *c == "PINE7005").count(),
        2,
        "{codes:?}"
    );
}

#[test]
fn a_block_can_declare_a_name_an_outer_scope_already_has() {
    let session = run(
        "x = 1\n\
         y = 10\n\
         if bar_index >= 0\n    x = 2\n    y := y + x\n\
         for i = 0 to 1\n    x = 100 + i\n\
         i = 7\n\
         for i = 0 to 2\n    y += 1\n",
        2,
    );
    // `x = 2` inside the branch is a new local: the outer `x` is untouched, while `y := …` writes
    // through. The counter `i` is the loop's own, and the outer `i` survives it.
    assert_eq!(value(&session, "x"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "i"), RuntimeValue::Int(7));
    // `y` restarts at 10 on every bar: +2 in the branch (through the local x = 2), +3 in the second loop.
    assert_eq!(value(&session, "y"), RuntimeValue::Int(15));
    let codes = diagnostic_codes("x = 1\nif true\n    x = 2\nplot(close)");
    assert!(codes.is_empty(), "{codes:?}");
    // The same scope still may not declare a name twice, and a parameter is in the function's scope.
    let codes = diagnostic_codes("x = 1\nx = 2\nplot(close)");
    assert!(codes.contains(&"PINE3020".to_owned()), "{codes:?}");
    let codes = diagnostic_codes("f(a) =>\n    a = 2\n    a\nplot(f(1))");
    assert!(codes.contains(&"PINE3020".to_owned()), "{codes:?}");
}

#[test]
fn a_loop_variable_may_carry_its_type() {
    let session = run(
        "type Zone\n    float top\n\
         zones = array.from(Zone.new(1.0), Zone.new(2.0))\n\
         total = 0\n\
         for int i = 0 to 3\n    total += i\n\
         sum = 0.0\n\
         for Zone z in zones\n    sum += z.top\n\
         last = 0.0\n\
         for [int index, Zone z] in zones\n    last := index + z.top\n\
         plain = 0.0\n\
         for z in zones\n    plain += z.top",
        1,
    );
    assert_eq!(value(&session, "total"), RuntimeValue::Int(6));
    assert_eq!(value(&session, "sum"), RuntimeValue::Float(3.0));
    assert_eq!(value(&session, "last"), RuntimeValue::Float(3.0));
    assert_eq!(value(&session, "plain"), RuntimeValue::Float(3.0));
}

#[test]
fn copy_makes_an_independent_object() {
    let session = run(
        "type Point\n    float x\n    float y\n\
         a = Point.new(1.0, 2.0)\n\
         b = a.copy()\n\
         b.x := 10.0\n\
         ax = a.x\n\
         bx = b.x\n\
         by = b.y",
        1,
    );
    assert_eq!(value(&session, "ax"), RuntimeValue::Float(1.0));
    assert_eq!(value(&session, "bx"), RuntimeValue::Float(10.0));
    assert_eq!(value(&session, "by"), RuntimeValue::Float(2.0));
}

#[test]
fn a_request_may_sit_inside_another_requests_expression() {
    let session = run(
        "inner() =>\n    request.security(syminfo.tickerid, timeframe.period, close)\n\
         outer = request.security(syminfo.tickerid, timeframe.period, inner() + 1)",
        3,
    );
    // The last bar closes at 103 (the helper's bars close at 101 + index).
    assert_eq!(value(&session, "outer"), RuntimeValue::Float(104.0));
    // A request that requests itself is cut off, not run away.
    let source = "//@version=6\nindicator(\"T\")\nf() =>\n    request.security(syminfo.tickerid, timeframe.period, f())\nx = f()";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    let Err(crate::pine::session::RunError::Script(diagnostic)) = session.evaluate(&bars(3)) else {
        panic!("expected the nesting limit");
    };
    assert_eq!(diagnostic.code, "PINE4023");
}

#[test]
fn the_first_row_added_to_an_empty_matrix_sets_its_width() {
    let session = run(
        "var matrix<float> hist = matrix.new<float>()\n\
         if bar_index < 2\n    hist.add_row(hist.rows(), array.from(1.0 + bar_index, 2.0, 3.0))\n\
         rows = hist.rows()\n\
         columns = hist.columns()\n\
         float corner = bar_index == 2 ? hist.get(1, 0) : na",
        3,
    );
    assert_eq!(value(&session, "rows"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "columns"), RuntimeValue::Int(3));
    assert_eq!(value(&session, "corner"), RuntimeValue::Float(2.0));
}

#[test]
fn calls_on_a_drawing_variable_that_is_still_na_do_nothing() {
    let session = run(
        "var line l = na\n\
         var polyline p = na\n\
         l.delete()\n\
         l.set_x1(3)\n\
         p.delete()\n\
         x = l.get_x1()\n\
         done = true",
        2,
    );
    assert_eq!(value(&session, "x"), RuntimeValue::Na);
    assert_eq!(value(&session, "done"), RuntimeValue::Bool(true));
}

#[test]
fn a_script_may_declare_a_name_a_builtin_variable_has() {
    // `low` is the script's from the declaration on, on every bar.
    let session = run(
        "dummy = 1\n\
         low = high - 1000.0\n\
         after = low\n\
         lagged = low[1]",
        3,
    );
    // The bars are low = price - 2 (98 + index): the third bar's builtin low is 100.
    assert_eq!(
        value(&session, "after"),
        RuntimeValue::Float(104.0 - 1000.0)
    );
    // `low[1]` keeps reading the builtin's history (a documented approximation).
    assert_eq!(value(&session, "lagged"), RuntimeValue::Float(99.0));
    let codes = diagnostic_codes("low = high - 1\nplot(low)");
    assert_eq!(codes, ["PINE7006"]);
}

#[test]
fn a_builtin_variable_read_above_its_declaration_cannot_be_shadowed() {
    // As in TradingView: once the script has used the builtin, the name cannot also be its own.
    let source = "//@version=6\nindicator(\"T\")\nx = open\nopen = 1\nplot(x + open)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let error = program
        .diagnostics
        .iter()
        .find(|d| d.code == "CE10190")
        .unwrap_or_else(|| panic!("{:?}", program.diagnostics));
    assert_eq!(
        error.message,
        "Cannot shadow the built-in variable \"open\" because it has already been used as a built-in."
    );
    assert_eq!(
        error.severity,
        crate::pine::token::DiagnosticSeverity::Error
    );
    assert_eq!(error.range.start.line, 4, "on the declaration");
    // Declared before any read, it stays the warning.
    assert_eq!(diagnostic_codes("open = 1\nplot(open)"), ["PINE7006"]);
}

#[test]
fn a_plot_offset_shifts_the_values_and_says_what_it_cannot_do() {
    let source =
        "//@version=6\nindicator(\"T\")\nplot(close, offset = 2)\nplot(close, offset = -1)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let warnings: Vec<_> = program
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].contains("only partly supported"),
        "{warnings:?}"
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
    let Ok(result) = session.evaluate(&bars(4)) else {
        panic!("the script runs");
    };
    let output = result.output;
    // close is 101, 102, 103, 104. Two bars later: the first two bars are empty, the last two hold
    // 101 and 102 (103 and 104 land past the last bar).
    assert_eq!(
        output.plots[0].values,
        [None, None, Some(101.0), Some(102.0)]
    );
    // One bar earlier: bar 0's value is gone, the rest move left.
    assert_eq!(
        output.plots[1].values,
        [Some(102.0), Some(103.0), Some(104.0), None]
    );
}

#[test]
fn a_variable_may_have_the_name_of_a_function() {
    let session = run(
        "twice(x) =>\n    x * 2\n\
         twice = twice(close)\n\
         plain = twice + 1",
        2,
    );
    assert_eq!(value(&session, "twice"), RuntimeValue::Float(2.0 * 102.0));
    assert_eq!(
        value(&session, "plain"),
        RuntimeValue::Float(2.0 * 102.0 + 1.0)
    );
    let codes = diagnostic_codes("f(float a) =>\n    a\nf(float b) =>\n    b\nplot(f(1.0))");
    assert!(codes.contains(&"PINE3024".to_owned()), "{codes:?}");
}

#[test]
fn every_builtin_variable_the_catalog_lists_can_be_read() {
    let mut unreadable = Vec::new();
    for name in crate::pine::symbol_catalog::VARIABLES {
        let source = format!("//@version=6\nindicator(\"T\")\nvalue = {name}\nplot(close)");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        if let Err(crate::pine::session::RunError::Script(diagnostic)) = session.evaluate(&bars(3))
            && diagnostic.code == "PINE4008"
        {
            unreadable.push(*name);
        }
    }
    assert!(unreadable.is_empty(), "{unreadable:?}");
}

#[test]
fn a_function_local_keeps_a_history_per_call_site() {
    let session = run(
        "step() =>\n    v = bar_index * 2\n    v[1]\n\
         grow(float src) =>\n    total = nz(total[1]) + src\n    total\n\
         edge(int period) =>\n    stamp = time(\"60\")\n    not na(stamp[1]) and stamp != stamp[1]\n\
         reads_chart() =>\n    close[1] + high[2]\n\
         a = step()\n\
         b = grow(1.0)\n\
         c = grow(10.0)\n\
         d = reads_chart()",
        5,
    );
    // v[1] is the value the previous bar's call left: bar_index 3 * 2 on the last bar (index 4).
    assert_eq!(value(&session, "a"), RuntimeValue::Int(6));
    // Each call site accumulates on its own: 1 + 1 + 1 + 1 + 1 and 10 * 5.
    assert_eq!(value(&session, "b"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "c"), RuntimeValue::Float(50.0));
    // A series that is not the function's own is still the chart's: close[1] = 104, high[2] = 104.
    assert_eq!(value(&session, "d"), RuntimeValue::Float(104.0 + 104.0));
}

#[test]
fn trade_properties_read_the_closed_and_open_trades() {
    let source = "//@version=6\nstrategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 2)\n\
         if bar_index == 1\n    strategy.entry(\"L\", strategy.long)\n\
         if bar_index == 3\n    strategy.close(\"L\")\n\
         if bar_index == 5\n    strategy.entry(\"S\", strategy.short)\n\
         closedCount = strategy.closedtrades\n\
         exitPrice = strategy.closedtrades.exit_price(0)\n\
         entryId = strategy.closedtrades.entry_id(0)\n\
         size = strategy.closedtrades.size(0)\n\
         profit = strategy.closedtrades.profit(0)\n\
         missing = strategy.closedtrades.exit_price(7)\n\
         openSize = strategy.opentrades.size(0)\n\
         openProfit = strategy.opentrades.profit(0)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
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
    assert!(session.evaluate(&bars(8)).is_ok());
    assert_eq!(value(&session, "closedCount"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "entryId"), RuntimeValue::String("L".into()));
    assert_eq!(value(&session, "size"), RuntimeValue::Float(2.0));
    assert_eq!(value(&session, "missing"), RuntimeValue::Na);
    // Entered at bar 2's open (102), closed at bar 4's open (104): two units, +2 each.
    assert_eq!(value(&session, "exitPrice"), RuntimeValue::Float(104.0));
    assert_eq!(value(&session, "profit"), RuntimeValue::Float(4.0));
    // The short opened on bar 6 at 106 and the last close is 108: 2 units losing 2 each.
    assert_eq!(value(&session, "openSize"), RuntimeValue::Float(-2.0));
    assert_eq!(value(&session, "openProfit"), RuntimeValue::Float(-4.0));
}

#[test]
fn the_chart_colours_are_the_themes() {
    let source = "//@version=6\nindicator(\"T\")\nbg = chart.bg_color\nfg = chart.fg_color\n\
                  light = color.r(chart.bg_color) > 128\nplot(close)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    assert!(session.evaluate(&bars(1)).is_ok());
    assert_eq!(value(&session, "light"), RuntimeValue::Bool(false));
    session.theme = crate::pine::program::ChartTheme::LIGHT;
    assert!(session.evaluate(&bars(1)).is_ok());
    assert_eq!(value(&session, "bg"), RuntimeValue::Color(0xffff_ffff));
    assert_eq!(value(&session, "fg"), RuntimeValue::Color(0x1317_22ff));
    assert_eq!(value(&session, "light"), RuntimeValue::Bool(true));
}

#[test]
fn names_a_block_declares_do_not_leak_out_of_it() {
    let session = run(
        "i = 100\nx = 5.0\ntotal = 0.0\n\
         for i = 0 to 3\n    x = i * 2.0\n    total += x\n\
         if total > 0\n    x = 99.0\n    i = 7\n\
         y = x\nz = i",
        2,
    );
    assert_eq!(value(&session, "total"), RuntimeValue::Float(12.0));
    assert_eq!(value(&session, "x"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "y"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "i"), RuntimeValue::Int(100));
    assert_eq!(value(&session, "z"), RuntimeValue::Int(100));
}

#[test]
fn a_block_that_breaks_or_continues_still_puts_its_names_back() {
    let session = run(
        "n = 1.0\nm = 2.0\nseen = 0\n\
         for k = 0 to 5\n    n = k * 1.0\n    if k == 1\n        continue\n    m = 50.0\n    seen += 1\n    if k == 2\n        break\n\
         r = n\ns = m",
        2,
    );
    assert_eq!(value(&session, "seen"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "r"), RuntimeValue::Float(1.0));
    assert_eq!(value(&session, "s"), RuntimeValue::Float(2.0));
}

#[test]
fn a_block_that_shadows_more_names_than_fit_inline_puts_them_all_back() {
    let session = run(
        "a = 1.0\nb = 2.0\nc = 3.0\nd = 4.0\ne = 5.0\nf = 6.0\n\
         for q = 0 to 1\n    a = 10.0\n    b = 11.0\n    c = 12.0\n    d = 13.0\n    e = 14.0\n    f = 15.0\n\
         sum = a + b + c + d + e + f",
        2,
    );
    assert_eq!(value(&session, "sum"), RuntimeValue::Float(21.0));
}

#[test]
fn a_nested_loop_restores_its_own_counter_each_time_it_runs() {
    let session = run(
        "j = 40\nacc = 0\n\
         for i = 0 to 2\n    for j = 0 to 1\n        acc += 1\n    acc += j\n\
         after = j",
        2,
    );
    // The inner counter is the outer loop's own `j` again after each inner loop: 40 per outer pass.
    assert_eq!(value(&session, "acc"), RuntimeValue::Int(3 * 2 + 3 * 40));
    assert_eq!(value(&session, "after"), RuntimeValue::Int(40));
}

#[test]
fn percent_assign_takes_the_remainder_like_modulo() {
    let session = run(
        "whole = 17\nwhole %= 5\nfraction = 7.5\nfraction %= 2\nwrapped = -7\nwrapped %= 3",
        2,
    );
    assert_eq!(value(&session, "whole"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "fraction"), RuntimeValue::Float(1.5));
    assert_eq!(value(&session, "wrapped"), RuntimeValue::Int(-1));
}

#[test]
fn once_runs_its_block_the_first_time_the_condition_holds() {
    let session = run(
        "var int hits = 0\nvar int at = -1\n\
         once bar_index >= 2\n    hits += 1\n    at := bar_index",
        6,
    );
    assert_eq!(value(&session, "hits"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "at"), RuntimeValue::Int(2));
}

#[test]
fn once_without_a_condition_runs_on_the_first_bar_only() {
    let session = run("var int hits = 0\nonce\n    hits += 1", 5);
    assert_eq!(value(&session, "hits"), RuntimeValue::Int(1));
}

#[test]
fn once_never_runs_when_its_condition_never_holds() {
    let session = run("var int hits = 0\nonce bar_index > 100\n    hits += 1", 5);
    assert_eq!(value(&session, "hits"), RuntimeValue::Int(0));
}

#[test]
fn each_once_structure_keeps_its_own_state() {
    let session = run(
        "var int early = 0\nvar int late = 0\n\
         once bar_index >= 1\n    early += 1\n\
         once bar_index >= 3\n    late += 1",
        6,
    );
    assert_eq!(value(&session, "early"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "late"), RuntimeValue::Int(1));
}

#[test]
fn a_loop_control_inside_once_still_reaches_the_loop() {
    let session = run(
        "var int hits = 0\nvar int seen = 0\n\
         for i = 0 to 4\n    once\n        hits += 1\n        continue\n    seen += 1",
        3,
    );
    // The block runs once in the whole run, so `continue` skips `seen` on one pass of 15.
    assert_eq!(value(&session, "hits"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "seen"), RuntimeValue::Int(14));
}

#[test]
fn a_numeric_once_condition_is_rejected() {
    assert!(!diagnostic_codes("once 1\n    x = 1").is_empty());
}

#[test]
fn once_cannot_be_assigned_to_a_variable() {
    assert!(!diagnostic_codes("x = once\n    1").is_empty());
}

#[test]
fn linefill_and_polyline_annotations_are_typed_handles() {
    let session = run(
        "var line a = na\nvar line b = na\nvar linefill f = na\nvar polyline p = na\n\
         if barstate.isfirst\n    a := line.new(0, 1.0, 2, 3.0)\n    b := line.new(0, 2.0, 2, 4.0)\n    f := linefill.new(a, b, color.red)\n\
         first = f.get_line1() == a\n\
         same = array.size(linefill.all)",
        2,
    );
    assert_eq!(value(&session, "first"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "same"), RuntimeValue::Int(1));
}

#[test]
fn a_function_parameter_may_be_a_linefill() {
    let session = run(
        "var line a = na\nvar line b = na\n\
         if barstate.isfirst\n    a := line.new(0, 1.0, 2, 3.0)\n    b := line.new(0, 2.0, 2, 4.0)\n\
         tint(linefill f, color c) =>\n    f.set_color(c)\n    f.get_line2() == b\n\
         ok = tint(linefill.new(a, b, color.blue), color.green)",
        2,
    );
    assert_eq!(value(&session, "ok"), RuntimeValue::Bool(true));
}

#[test]
fn a_linefill_variable_rejects_a_value_of_another_type() {
    assert!(diagnostic_codes("linefill f = 1.5").contains(&"PINE3030".to_owned()));
    assert!(diagnostic_codes("polyline p = \"x\"").contains(&"PINE3030".to_owned()));
    assert!(
        diagnostic_codes("linefill f = line.new(0, 1.0, 1, 2.0)").contains(&"PINE3030".to_owned())
    );
}

#[test]
fn linefill_and_polyline_names_are_spelled_in_diagnostics() {
    let message = compiler::compile(
        "//@version=6\nindicator(\"T\")\nlinefill f = 1.5",
        Limits::default(),
        &NoLibraries,
    )
    .diagnostics
    .into_iter()
    .find(|d| d.code == "PINE3030")
    .map(|d| d.message)
    .unwrap_or_default();
    assert!(message.contains("linefill"), "{message}");
}

#[test]
fn array_percentrank_every_and_some_work_as_functions_and_methods() {
    let session = run(
        "a = array.from(10, 20, 30, 40)\n\
         flags = array.from(true, false)\n\
         rank = array.percentrank(a, 1)\n\
         method_rank = a.percentrank(3)\n\
         all_flags = flags.every()\n\
         any_flags = array.some(flags)",
        2,
    );
    assert_eq!(value(&session, "rank"), RuntimeValue::Float(50.0));
    assert_eq!(value(&session, "method_rank"), RuntimeValue::Float(100.0));
    assert_eq!(value(&session, "all_flags"), RuntimeValue::Bool(false));
    assert_eq!(value(&session, "any_flags"), RuntimeValue::Bool(true));
}
