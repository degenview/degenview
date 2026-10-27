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
        "amount = strategy.convert_to_account(12.5)\nsame = strategy.convert_to_symbol(3)",
        1,
    );
    assert_eq!(value(&session, "amount"), RuntimeValue::Float(12.5));
    assert_eq!(value(&session, "same"), RuntimeValue::Int(3));
    let codes = diagnostic_codes(
        "a = strategy.convert_to_symbol(1)\nb = strategy.convert_to_account(1)\nplot(close)",
    );
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

#[test]
fn matrix_arithmetic_and_structure_work_as_functions_and_methods() {
    let session = run(
        "a = matrix.new<int>(2, 2, 0)\n\
         a.set(0, 0, 1)\na.set(0, 1, 2)\na.set(1, 0, 3)\na.set(1, 1, 4)\n\
         b = a.mult(a)\n\
         c = matrix.sum(a, b)\n\
         sq = matrix.pow(a, 2)\n\
         tr = a.trace()\n\
         top = c.get(0, 0)\n\
         same = sq.get(1, 1)\n\
         a.concat(matrix.copy(a))\n\
         rows = a.rows()\n\
         a.reshape(2, 4)\n\
         wide = a.columns()\n\
         part = a.submatrix(0, 1, 0, 2)\n\
         first = part.get(0, 1)\n\
         a.sort(0, order.descending)\n\
         lead = a.get(0, 0)",
        2,
    );
    assert_eq!(value(&session, "top"), RuntimeValue::Int(8));
    assert_eq!(value(&session, "same"), RuntimeValue::Float(22.0));
    assert_eq!(value(&session, "tr"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "rows"), RuntimeValue::Int(4));
    assert_eq!(value(&session, "wide"), RuntimeValue::Int(4));
    assert_eq!(value(&session, "first"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "lead"), RuntimeValue::Int(1));
}

#[test]
fn a_matrix_runtime_error_names_the_function() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\")\na = matrix.new<int>(2, 2, 0)\nb = matrix.new<int>(3, 3, 0)\nc = matrix.sum(a, b)",
        Limits::default(),
        &NoLibraries,
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
    let Err(error) = session.evaluate(&bars(2)) else {
        panic!("a mismatched sum must fail");
    };
    assert!(format!("{error:?}").contains("PINE4037"), "{error:?}");
}

#[test]
fn copies_of_a_line_label_and_box_are_independent_objects() {
    let session = run(
        "var line l = na\nvar label t = na\nvar box b = na\n\
         var line lc = na\nvar label tc = na\nvar box bc = na\n\
         if barstate.islast\n\
         \x20   l := line.new(0, 10.0, 4, 18.0)\n\
         \x20   lc := line.copy(l)\n\
         \x20   lc.set_x2(8)\n\
         \x20   t := label.new(1, 5.0, \"hi\")\n\
         \x20   tc := label.copy(t)\n\
         \x20   tc.set_text(\"copy\")\n\
         \x20   b := box.new(0, 9.0, 3, 2.0)\n\
         \x20   bc := box.copy(b)\n\
         \x20   bc.set_right(7)\n\
         original = l.get_x2()\ncopied = lc.get_x2()\n\
         texts = t.get_text() + \"/\" + tc.get_text()\n\
         right_original = b.get_right()\nright_copied = bc.get_right()\n\
         lines = array.size(line.all)\nlabels = array.size(label.all)\nboxes = array.size(box.all)\n\
         gone = line.copy(na)",
        2,
    );
    assert_eq!(value(&session, "original"), RuntimeValue::Int(4));
    assert_eq!(value(&session, "copied"), RuntimeValue::Int(8));
    assert_eq!(
        value(&session, "texts"),
        RuntimeValue::String("hi/copy".into())
    );
    assert_eq!(value(&session, "right_original"), RuntimeValue::Int(3));
    assert_eq!(value(&session, "right_copied"), RuntimeValue::Int(7));
    assert_eq!(value(&session, "lines"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "labels"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "boxes"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "gone"), RuntimeValue::Na);
}

#[test]
fn copying_respects_the_declared_cap_on_drawings() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\", max_lines_count = 2)\n\
         l = line.new(0, 1.0, 1, 2.0)\n\
         line.copy(l)\nline.copy(l)\n\
         n = array.size(line.all)",
        Limits::default(),
        &NoLibraries,
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
    session.evaluate(&bars(1)).expect("runs");
    assert_eq!(value(&session, "n"), RuntimeValue::Int(2));
}

#[test]
fn a_chart_point_copy_is_a_new_point_with_the_same_fields() {
    let session = run(
        "p = chart.point.from_index(3, 12.5)\n\
         q = chart.point.copy(p)\n\
         same_price = q.price == 12.5\n\
         same_index = q.index == 3\n\
         p.price := 99.0\n\
         kept = q.price",
        2,
    );
    assert_eq!(value(&session, "same_price"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "same_index"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "kept"), RuntimeValue::Float(12.5));
}

#[test]
fn line_get_price_extends_the_line_both_ways() {
    let session = run(
        "l = line.new(0, 10.0, 4, 18.0)\n\
         inside = l.get_price(2)\n\
         ahead = line.get_price(l, 10)\n\
         behind = l.get_price(-2)\n\
         vertical = line.get_price(line.new(3, 1.0, 3, 5.0), 3)",
        2,
    );
    assert_eq!(value(&session, "inside"), RuntimeValue::Float(14.0));
    assert_eq!(value(&session, "ahead"), RuntimeValue::Float(30.0));
    assert_eq!(value(&session, "behind"), RuntimeValue::Float(6.0));
    assert_eq!(value(&session, "vertical"), RuntimeValue::Na);
}

#[test]
fn line_get_price_is_an_error_for_a_line_placed_by_time() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\")\n\
         l = line.new(0, 1.0, 60000, 2.0, xloc = xloc.bar_time)\n\
         p = l.get_price(1)",
        Limits::default(),
        &NoLibraries,
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
    let Err(error) = session.evaluate(&bars(3)) else {
        panic!("a time-placed line has no price by bar index");
    };
    assert!(format!("{error:?}").contains("PINE4038"), "{error:?}");
}

#[test]
fn set_xloc_changes_how_the_coordinates_are_read() {
    let session = run(
        "var line l = na\nvar box b = na\nvar label t = na\n\
         if barstate.islast\n\
         \x20   l := line.new(0, 1.0, 1, 2.0)\n\
         \x20   b := box.new(0, 9.0, 1, 2.0)\n\
         \x20   t := label.new(0, 5.0, \"x\")\n\
         \x20   l.set_xloc(120000, 240000, xloc.bar_time)\n\
         \x20   b.set_xloc(60000, 180000, xloc.bar_time)\n\
         \x20   t.set_xloc(300000, xloc.bar_time)\n\
         l_from = l.get_x1()\nl_to = l.get_x2()\n\
         b_left = b.get_left()\nb_right = b.get_right()\n\
         t_at = t.get_x()\n\
         if barstate.islast\n\
         \x20   l.set_xloc(1, 3, xloc.bar_index)\n\
         \x20   l.set_x2(4)\n\
         back = l.get_x1()\nend = l.get_x2()",
        6,
    );
    assert_eq!(value(&session, "l_from"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "l_to"), RuntimeValue::Int(4));
    assert_eq!(value(&session, "b_left"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "b_right"), RuntimeValue::Int(3));
    assert_eq!(value(&session, "t_at"), RuntimeValue::Int(5));
    assert_eq!(value(&session, "back"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "end"), RuntimeValue::Int(4));
}

#[test]
fn a_labels_yloc_is_kept_and_can_be_changed() {
    let session = run(
        "var label a = na\nvar label b = na\n\
         if barstate.islast\n\
         \x20   a := label.new(bar_index, na, \"up\", yloc = yloc.abovebar)\n\
         \x20   b := label.new(bar_index, 7.0, \"price\")\n\
         \x20   b.set_yloc(yloc.belowbar)",
        3,
    );
    let labels = session.output().labels;
    let yloc = |text: &str| labels.iter().find(|l| l.text == text).map(|l| l.yloc);
    assert_eq!(
        yloc("up"),
        Some(crate::pine::outputs::YLocation::AboveBar),
        "a price of na is fine above the bar"
    );
    assert_eq!(
        yloc("price"),
        Some(crate::pine::outputs::YLocation::BelowBar)
    );
    assert!(labels.iter().all(|label| label.is_complete()));
}

#[test]
fn the_cell_setters_change_one_property_of_a_cell() {
    let session = run(
        "var table t = na\n\
         if barstate.islast\n\
         \x20   t := table.new(position.top_right, 2, 2)\n\
         \x20   table.cell(t, 0, 0, \"one\", text_color = color.red, bgcolor = color.blue)\n\
         \x20   table.cell_set_text(t, 0, 0, \"first\")\n\
         \x20   table.cell_set_text_color(t, 0, 0, color.green)\n\
         \x20   table.cell_set_bgcolor(t, 0, 0, color.yellow)\n\
         \x20   table.cell_set_width(t, 0, 0, 25)\n\
         \x20   table.cell_set_height(t, 0, 0, 10.5)\n\
         \x20   table.cell_set_text_halign(t, 0, 0, text.align_left)\n\
         \x20   table.cell_set_text_valign(t, 0, 0, text.align_top)\n\
         \x20   table.cell_set_text_size(t, 0, 0, size.large)\n\
         \x20   t.cell_set_text(1, 1, \"made by the setter\")\n\
         \x20   table.cell_set_text_size(t, 1, 1, 20)",
        2,
    );
    let tables = session.output().tables;
    let cells = &tables[0].cells;
    let first = cells.iter().find(|c| (c.column, c.row) == (0, 0)).unwrap();
    assert_eq!(first.text, "first");
    let color = |name: &str| crate::pine::builtins::color(name).unwrap();
    assert_eq!(first.text_color, color("color.green"), "set over red");
    assert_eq!(
        first.background_color,
        Some(color("color.yellow")),
        "set over blue"
    );
    assert_eq!((first.width, first.height), (25.0, 10.5));
    assert_eq!(
        first.text_horizontal_align,
        crate::pine::outputs::TextAlign::Left
    );
    assert_eq!(
        first.text_vertical_align,
        crate::pine::outputs::TextAlign::Top
    );
    assert_eq!(first.text_size, crate::pine::outputs::Size::Large);
    let made = cells.iter().find(|c| (c.column, c.row) == (1, 1)).unwrap();
    assert_eq!(made.text, "made by the setter");
    assert_eq!(made.text_size, crate::pine::outputs::Size::Points(20));
    assert_eq!(cells.len(), 2, "no other cell appears");
}

#[test]
fn a_cell_setter_outside_the_grid_is_an_error() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\")\nt = table.new(position.top_left, 2, 2)\ntable.cell_set_text(t, 2, 0, \"x\")",
        Limits::default(),
        &NoLibraries,
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
    let Err(error) = session.evaluate(&bars(2)) else {
        panic!("a cell outside the grid must fail");
    };
    assert!(format!("{error:?}").contains("PINE4013"), "{error:?}");
}

#[test]
fn text_sizes_may_be_numbers_of_points() {
    let session = run(
        "var label a = na\nvar label b = na\nvar box c = na\n\
         if barstate.islast\n\
         \x20   a := label.new(bar_index, 1.0, \"a\", size = 20)\n\
         \x20   b := label.new(bar_index, 1.0, \"b\", size = size.huge)\n\
         \x20   b.set_size(0)\n\
         \x20   c := box.new(0, 2.0, 1, 1.0, text = \"c\", text_size = 8)\n\
         \x20   c.set_text_size(14)",
        2,
    );
    use crate::pine::outputs::Size;
    let output = session.output();
    let size = |text: &str| output.labels.iter().find(|l| l.text == text).unwrap().size;
    assert_eq!(size("a"), Size::Points(20));
    assert_eq!(size("b"), Size::Auto, "0 is size.auto");
    assert_eq!(output.boxes[0].text_size, Size::Points(14));
}

#[test]
fn the_cell_properties_the_chart_cannot_draw_are_warned_about() {
    let codes = diagnostic_codes(
        "t = table.new(position.top_left, 1, 1)\n\
         table.cell(t, 0, 0, \"x\", tooltip = \"tip\", text_font_family = font.family_monospace)\n\
         table.cell_set_tooltip(t, 0, 0, \"tip\")\n\
         table.cell_set_text_formatting(t, 0, 0, text.format_bold)\n\
         table.cell_set_text_font_family(t, 0, 0, font.family_monospace)",
    );
    assert_eq!(codes.iter().filter(|c| *c == "PINE7001").count(), 2);
    assert_eq!(codes.iter().filter(|c| *c == "PINE7005").count(), 3);
}

#[test]
fn plotarrow_records_a_value_and_both_colors_for_each_bar() {
    let session = run(
        "plotarrow(bar_index % 3 - 1, \"arrows\", color.green, color.maroon, minheight = 8, maxheight = 40)",
        6,
    );
    let output = session.output();
    let arrow = &output.arrows[0];
    assert_eq!(arrow.title.as_deref(), Some("arrows"));
    assert_eq!(
        arrow.values,
        [-1, 0, 1, -1, 0, 1].map(|v| Some(f64::from(v)))
    );
    let color = |name: &str| crate::pine::builtins::color(name);
    assert!(arrow.up_colors.iter().all(|c| *c == color("color.green")));
    assert!(
        arrow
            .down_colors
            .iter()
            .all(|c| *c == color("color.maroon"))
    );
    assert_eq!((arrow.min_height, arrow.max_height), (8.0, 40.0));
}

#[test]
fn plotarrow_without_options_has_the_defaults_and_na_draws_nothing() {
    let session = run("plotarrow(bar_index == 1 ? na : 2.5)", 3);
    let output = session.output();
    let arrow = &output.arrows[0];
    assert_eq!(arrow.values, [Some(2.5), None, Some(2.5)]);
    assert_eq!((arrow.min_height, arrow.max_height), (5.0, 100.0));
    assert_eq!(
        arrow.up_colors[0],
        Some(crate::pine::drawings::DEFAULT_COLOR)
    );
    assert_eq!(
        arrow.down_colors[0],
        crate::pine::builtins::color("color.red")
    );
    assert!(!arrow.force_overlay);
}

#[test]
fn plotarrow_honours_offset_show_last_and_force_overlay() {
    let session = run(
        "plotarrow(bar_index + 1.0, offset = 1, show_last = 2, force_overlay = true)",
        5,
    );
    let output = session.output();
    // Bar i's value i+1 is shown on bar i+1, and only the last two bars keep their arrow.
    assert_eq!(
        output.arrows[0].values,
        [None, None, None, Some(3.0), Some(4.0)]
    );
    assert!(output.has_forced_overlay());
    let (pane, forced) = output.split_forced_overlay();
    assert_eq!(forced.arrows.len(), 1);
    assert_eq!(
        pane.arrows[0].display & crate::pine::outputs::DISPLAY_PANE,
        0,
        "the pane no longer draws it"
    );
}

#[test]
fn plotarrow_is_no_longer_reserved_and_its_ignored_arguments_are_warned_about() {
    assert!(
        !diagnostic_codes("plotarrow(close)")
            .iter()
            .any(|c| c.starts_with("PINE3") || c.starts_with("PINE4"))
    );
    let codes = diagnostic_codes("plotarrow(close, editable = true, format = format.price)");
    assert_eq!(codes.iter().filter(|c| *c == "PINE7001").count(), 2);
    assert!(diagnostic_codes("plotarrow(close, offset = 1)").contains(&"PINE7001".to_owned()));
}

#[test]
fn the_stepline_diamond_style_is_a_style_of_its_own() {
    let session = run("plot(close, style = plot.style_stepline_diamond)", 3);
    assert_eq!(
        session.output().plots[0].style,
        crate::pine::outputs::PlotStyle::SteplineDiamond
    );
    assert!(
        !diagnostic_codes("plot(close, style = plot.style_stepline_diamond)")
            .contains(&"PINE7004".to_owned()),
        "it is no longer drawn as something else"
    );
}

#[test]
fn pivot_point_levels_come_back_as_an_array_in_the_documented_order() {
    // Bars 0 and 1 are the period (O 100, H 103, L 98, C 102); bar 2 is the anchor.
    let session = run(
        "levels = ta.pivot_point_levels(\"Traditional\", bar_index == 2)\n\
         p = array.get(levels, 0)\nr1 = array.get(levels, 1)\ns1 = array.get(levels, 2)\n\
         r5 = array.get(levels, 9)\ns5 = array.get(levels, 10)\n\
         size = array.size(levels)\n\
         dm = ta.pivot_point_levels(\"DM\", bar_index == 2)\n\
         dm_r2 = array.get(dm, 3)\n\
         developing = ta.pivot_point_levels(\"Traditional\", bar_index == 2, true)\n\
         running = array.get(developing, 0)",
        3,
    );
    // P = (103 + 98 + 102) / 3 = 101.
    assert_eq!(value(&session, "p"), RuntimeValue::Float(101.0));
    assert_eq!(value(&session, "r1"), RuntimeValue::Float(104.0));
    assert_eq!(value(&session, "s1"), RuntimeValue::Float(99.0));
    assert_eq!(value(&session, "r5"), RuntimeValue::Float(115.0));
    assert_eq!(value(&session, "s5"), RuntimeValue::Float(90.0));
    assert_eq!(value(&session, "size"), RuntimeValue::Int(11));
    assert_eq!(
        value(&session, "dm_r2"),
        RuntimeValue::Na,
        "DM stops at R1 and S1"
    );
    // The anchor bar alone: H 104, L 100, C 103.
    close_to(&session, "running", (104.0 + 100.0 + 103.0) / 3.0);
}

#[test]
fn an_unknown_pivot_type_stops_the_script() {
    let program = compiler::compile(
        "//@version=6\nindicator(\"T\")\nlevels = ta.pivot_point_levels(\"Nope\", true)",
        Limits::default(),
        &NoLibraries,
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
    let Err(error) = session.evaluate(&bars(2)) else {
        panic!("an unknown type must fail");
    };
    assert!(format!("{error:?}").contains("Traditional"), "{error:?}");
}

#[test]
fn the_rank_correlation_index_follows_a_trend() {
    let session = run(
        "rising = ta.rci(close, 4)\nfalling = ta.rci(-close, 4)\nshort = ta.rci(close, 9)",
        6,
    );
    close_to(&session, "rising", 100.0);
    close_to(&session, "falling", -100.0);
    assert_eq!(value(&session, "short"), RuntimeValue::Na);
}

/// Compiles `body` and runs it over `data` for a symbol of `kind`.
fn run_for(kind: &str, data: &[Kline], body: &str, alert: bool) -> Session {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let symbol = SymbolInfo {
        kind: kind.to_owned(),
        ..SymbolInfo::default()
    };
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol,
        None,
        None,
    );
    session.alert_mode = alert;
    if let Err(error) = session.evaluate(data) {
        panic!("{error:?}");
    }
    session
}

/// Half-hour bars of Tuesday 2024-03-05 from 04:00 to 19:30 New York time.
fn new_york_day() -> Vec<Kline> {
    let start = crate::pine::calendar::make(2024, 3, 5, 9, 0, 0, 0).unwrap();
    (0..32)
        .map(|index| {
            let open = start + index * 1_800_000;
            Kline {
                open_time: open,
                close_time: open + 1_799_999,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 1.0,
            }
        })
        .collect()
}

#[test]
fn session_flags_follow_a_us_stocks_hours_in_new_york() {
    let session = run_for(
        "stock",
        &new_york_day(),
        "var int market = 0\nvar int pre = 0\nvar int post = 0\nvar int firsts = 0\nvar int lasts = 0\n\
         var int first_at = -1\nvar int last_at = -1\n\
         if session.ismarket\n    market += 1\n\
         if session.ispremarket\n    pre += 1\n\
         if session.ispostmarket\n    post += 1\n\
         if session.isfirstbar\n    firsts += 1\n    first_at := bar_index\n\
         if session.islastbar\n    lasts += 1\n    last_at := bar_index\n\
         regular_first = session.isfirstbar_regular\nregular_last = session.islastbar_regular",
        false,
    );
    // 04:00 … 09:00 are 11 premarket bars, 09:30 … 15:30 are 13 regular ones, 16:00 … 19:30 are 8.
    assert_eq!(value(&session, "pre"), RuntimeValue::Int(11));
    assert_eq!(value(&session, "market"), RuntimeValue::Int(13));
    assert_eq!(value(&session, "post"), RuntimeValue::Int(8));
    assert_eq!(value(&session, "firsts"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "first_at"), RuntimeValue::Int(11));
    assert_eq!(value(&session, "lasts"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "last_at"), RuntimeValue::Int(23));
    // The regular-session forms agree on the last bar (19:30 is after the close).
    assert_eq!(value(&session, "regular_first"), RuntimeValue::Bool(false));
    assert_eq!(value(&session, "regular_last"), RuntimeValue::Bool(false));
}

#[test]
fn a_crypto_session_is_a_utc_day_that_is_always_open() {
    // One-minute bars from 1970-01-01 00:00 UTC for a day and a quarter.
    let session = run(
        "var int firsts = 0\nvar int lasts = 0\nvar int closed = 0\n\
         if session.isfirstbar\n    firsts += 1\n\
         if session.islastbar\n    lasts += 1\n\
         if not session.ismarket or session.ispremarket or session.ispostmarket\n    closed += 1",
        1_800,
    );
    assert_eq!(value(&session, "firsts"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "lasts"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "closed"), RuntimeValue::Int(0));
}

#[test]
fn every_bar_of_a_daily_chart_is_a_whole_session() {
    let daily: Vec<Kline> = (0..3)
        .map(|index| Kline {
            open_time: index * 86_400_000,
            close_time: (index + 1) * 86_400_000 - 1,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 1.0,
        })
        .collect();
    let session = run_for(
        "stock",
        &daily,
        "var int firsts = 0\nvar int lasts = 0\nvar int pre = 0\n\
         if session.isfirstbar\n    firsts += 1\n\
         if session.islastbar\n    lasts += 1\n\
         if session.ispremarket\n    pre += 1\n\
         market = session.ismarket",
        false,
    );
    assert_eq!(value(&session, "firsts"), RuntimeValue::Int(3));
    assert_eq!(value(&session, "lasts"), RuntimeValue::Int(3));
    assert_eq!(value(&session, "pre"), RuntimeValue::Int(0));
    assert_eq!(value(&session, "market"), RuntimeValue::Bool(true));
}

#[test]
fn syminfo_names_the_session_the_zone_and_the_contract() {
    let session = run_for(
        "stock",
        &new_york_day(),
        "zone = syminfo.timezone\nmode = syminfo.session\nmain = syminfo.main_tickerid\n\
         contract = syminfo.current_contract\nexpires = syminfo.expiration_date\n\
         scale = syminfo.pricescale\nmove = syminfo.minmove",
        false,
    );
    assert_eq!(
        value(&session, "zone"),
        RuntimeValue::String("America/New_York".into())
    );
    assert_eq!(
        value(&session, "mode"),
        RuntimeValue::String("session.regular".into())
    );
    assert_eq!(value(&session, "main"), RuntimeValue::String(String::new()));
    assert_eq!(value(&session, "contract"), RuntimeValue::Na);
    assert_eq!(value(&session, "expires"), RuntimeValue::Na);
    assert_eq!(
        value(&session, "move").number().unwrap() / value(&session, "scale").number().unwrap(),
        session.mintick,
        "minmove / pricescale is the tick"
    );
    let crypto = run("zone = syminfo.timezone", 2);
    assert_eq!(
        value(&crypto, "zone"),
        RuntimeValue::String("Etc/UTC".into())
    );
}

#[test]
fn weekofyear_counts_sunday_to_saturday_weeks_with_the_first_holding_january_1() {
    let session = run(
        "w1 = weekofyear(timestamp(2024, 1, 1))\n\
         w1_sat = weekofyear(timestamp(2024, 1, 6))\n\
         w2 = weekofyear(timestamp(2024, 1, 7))\n\
         last_2023 = weekofyear(timestamp(2023, 12, 31))\n\
         last_2024 = weekofyear(timestamp(2024, 12, 31))\n\
         shifted = weekofyear(timestamp(2024, 1, 6, 23, 0, 0), \"UTC+2\")\n\
         from_bar = weekofyear\n\
         missing = weekofyear(na)",
        2,
    );
    assert_eq!(value(&session, "w1"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "w1_sat"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "w2"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "last_2023"), RuntimeValue::Int(53));
    assert_eq!(value(&session, "last_2024"), RuntimeValue::Int(53));
    assert_eq!(
        value(&session, "shifted"),
        RuntimeValue::Int(2),
        "Sunday 01:00 in UTC+2"
    );
    assert_eq!(
        value(&session, "from_bar"),
        RuntimeValue::Int(1),
        "1970-01-01 is week 1"
    );
    assert_eq!(value(&session, "missing"), RuntimeValue::Na);
}

#[test]
fn runtime_calculation_mode_tells_a_chart_from_an_alert() {
    let chart = run_for("crypto", &bars(2), "mode = runtime.calculation.mode", false);
    assert_eq!(
        value(&chart, "mode"),
        RuntimeValue::String("runtime.calculation.mode.chart".into())
    );
    let alert = run_for("crypto", &bars(2), "mode = runtime.calculation.mode", true);
    assert_eq!(
        value(&alert, "mode"),
        RuntimeValue::String("runtime.calculation.mode.alert".into())
    );
    let compared = run_for(
        "crypto",
        &bars(2),
        "in_alert = runtime.calculation.mode == runtime.calculation.mode.alert",
        true,
    );
    assert_eq!(value(&compared, "in_alert"), RuntimeValue::Bool(true));
}

#[test]
fn the_constants_that_belong_to_the_data_requests_are_known_names() {
    let session = run(
        "a = dividends.net\nb = earnings.estimate\nc = splits.denominator\nd = currency.RSD\ne = currency.VES",
        1,
    );
    assert_eq!(
        value(&session, "a"),
        RuntimeValue::String("dividends.net".into())
    );
    assert_eq!(
        value(&session, "d"),
        RuntimeValue::String("currency.RSD".into())
    );
}

#[test]
fn the_variables_that_need_data_the_app_lacks_read_na_with_a_warning() {
    let body = "a = dividends.future_amount\nb = earnings.future_eps\nc = syminfo.isin\nd = ask\nplot(close)";
    let session = run(body, 1);
    for name in ["a", "b", "c", "d"] {
        assert_eq!(value(&session, name), RuntimeValue::Na, "{name}");
    }
    assert_eq!(
        diagnostic_codes(body)
            .iter()
            .filter(|code| *code == "PINE7003")
            .count(),
        4
    );
}

/// One-minute bars that open and close at `prices` (high and low one off), for strategies whose
/// fills must be easy to follow: an order placed on a bar fills at the next bar's open.
fn flat_bars(prices: &[f64]) -> Vec<Kline> {
    prices
        .iter()
        .enumerate()
        .map(|(index, price)| Kline {
            open_time: index as i64 * 60_000,
            close_time: (index as i64 + 1) * 60_000 - 1,
            open: *price,
            high: price + 1.0,
            low: price - 1.0,
            close: *price,
            volume: 1.0,
        })
        .collect()
}

const STRATEGY_PRICES: [f64; 15] = [
    100.0, 100.0, 110.0, 110.0, 100.0, 100.0, 90.0, 90.0, 100.0, 100.0, 100.0, 100.0, 100.0, 80.0,
    80.0,
];

/// Four round trips of two units: a long for +20, a short for +20, a long for 0 and a long for -40.
const STRATEGY_BODY: &str = "//@version=6\n\
    strategy(\"S\", initial_capital = 1000, default_qty_type = strategy.fixed, default_qty_value = 2)\n\
    if bar_index == 0\n    strategy.entry(\"L\", strategy.long)\n\
    if bar_index == 2\n    strategy.close(\"L\")\n\
    if bar_index == 4\n    strategy.entry(\"S\", strategy.short)\n\
    if bar_index == 6\n    strategy.close(\"S\")\n\
    if bar_index == 8\n    strategy.entry(\"L2\", strategy.long)\n\
    if bar_index == 10\n    strategy.close(\"L2\")\n\
    if bar_index == 11\n    strategy.entry(\"L3\", strategy.long)\n\
    if bar_index == 12\n    strategy.close(\"L3\")\n";

fn run_strategy(extra: &str, prices: &[f64]) -> Session {
    let source = format!("{STRATEGY_BODY}{extra}");
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
    if let Err(error) = session.evaluate(&flat_bars(prices)) {
        panic!("{error:?}");
    }
    session
}

#[test]
fn the_strategy_statistics_summarise_the_closed_trades() {
    let session = run_strategy(
        "closed = strategy.closedtrades\nnet = strategy.netprofit\n\
         avg = strategy.avg_trade\navg_pct = strategy.avg_trade_percent\n\
         win = strategy.avg_winning_trade\nwin_pct = strategy.avg_winning_trade_percent\n\
         loss = strategy.avg_losing_trade\nloss_pct = strategy.avg_losing_trade_percent\n\
         even = strategy.eventrades\nwins = strategy.wintrades\nlosses = strategy.losstrades\n\
         gross_pct = strategy.grossprofit_percent\nnet_pct = strategy.netprofit_percent\n\
         first = strategy.closedtrades.first_index",
        &STRATEGY_PRICES,
    );
    let number = |name: &str| value(&session, name).number().unwrap();
    assert_eq!(value(&session, "closed"), RuntimeValue::Int(4));
    assert_eq!(number("net"), 0.0);
    // +20, +20, 0, -40.
    assert_eq!(number("avg"), 0.0);
    assert_eq!(number("win"), 20.0);
    assert_eq!(number("loss"), -40.0);
    // 10 %, 10 %, 0 %, -20 % of what each put in.
    assert!((number("avg_pct") - 0.0).abs() < 1e-9);
    assert!((number("win_pct") - 10.0).abs() < 1e-9);
    assert!((number("loss_pct") + 20.0).abs() < 1e-9);
    assert_eq!(value(&session, "even"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "wins"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "losses"), RuntimeValue::Int(1));
    // 40 of profit on 1000 of capital.
    assert!((number("gross_pct") - 4.0).abs() < 1e-9);
    assert_eq!(number("net_pct"), 0.0);
    assert_eq!(value(&session, "first"), RuntimeValue::Int(0));
}

#[test]
fn the_averages_are_na_until_there_is_a_trade_to_average() {
    let session = run_strategy(
        "avg = strategy.avg_trade\nwin = strategy.avg_winning_trade\nloss = strategy.avg_losing_trade_percent",
        &STRATEGY_PRICES[..2],
    );
    for name in ["avg", "win", "loss"] {
        assert_eq!(value(&session, name), RuntimeValue::Na, "{name}");
    }
}

#[test]
fn drawdown_and_run_up_follow_the_equity_curve() {
    let session = run_strategy(
        "drawdown = strategy.max_drawdown\ndrawdown_pct = strategy.max_drawdown_percent\n\
         runup = strategy.max_runup\nrunup_pct = strategy.max_runup_percent",
        &STRATEGY_PRICES,
    );
    let number = |name: &str| value(&session, name).number().unwrap();
    // Equity runs 1000 → 1040 and falls back to 1000 when the last trade loses 40.
    assert_eq!(number("drawdown"), 40.0);
    assert!((number("drawdown_pct") - 40.0 / 1040.0 * 100.0).abs() < 1e-9);
    assert_eq!(number("runup"), 40.0);
    assert!((number("runup_pct") - 4.0).abs() < 1e-9);
}

#[test]
fn the_extremes_include_the_bar_being_run() {
    // On bar 6 the short is 20 in profit and not yet closed: equity 1040 is already the peak, and
    // the bar-2 close of the long had put it at 1020.
    let session = run_strategy(
        "runup = strategy.max_runup\ndrawdown = strategy.max_drawdown",
        &STRATEGY_PRICES[..7],
    );
    assert_eq!(value(&session, "runup"), RuntimeValue::Float(40.0));
    assert_eq!(value(&session, "drawdown"), RuntimeValue::Float(0.0));
}

#[test]
fn contracts_held_the_open_position_and_the_capital_it_holds() {
    let session = run_strategy(
        "all = strategy.max_contracts_held_all\nlong = strategy.max_contracts_held_long\n\
         short = strategy.max_contracts_held_short\nentry = strategy.position_entry_name\n\
         held = strategy.opentrades.capital_held\nopen_pct = strategy.openprofit_percent",
        &STRATEGY_PRICES[..7],
    );
    assert_eq!(value(&session, "all"), RuntimeValue::Float(2.0));
    assert_eq!(value(&session, "long"), RuntimeValue::Float(2.0));
    assert_eq!(value(&session, "short"), RuntimeValue::Float(2.0));
    // The short opened on bar 5 at 100 is still open on bar 6, where the close is 90.
    assert_eq!(value(&session, "entry"), RuntimeValue::String("S".into()));
    assert_eq!(value(&session, "held"), RuntimeValue::Float(200.0));
    // 20 of open profit on the 1020 the first trade realized.
    assert!((value(&session, "open_pct").number().unwrap() - 20.0 / 1020.0 * 100.0).abs() < 1e-9);
    let flat = run_strategy(
        "entry = strategy.position_entry_name\nheld = strategy.opentrades.capital_held\nnone_long = strategy.max_contracts_held_long",
        &STRATEGY_PRICES[..1],
    );
    assert_eq!(value(&flat, "entry"), RuntimeValue::String(String::new()));
    assert_eq!(value(&flat, "held"), RuntimeValue::Float(0.0));
    assert_eq!(value(&flat, "none_long"), RuntimeValue::Float(0.0));
}

#[test]
fn the_account_currency_is_the_strategys_or_the_symbols() {
    let symbols = run_strategy(
        "currency = strategy.account_currency",
        &STRATEGY_PRICES[..2],
    );
    assert_eq!(
        value(&symbols, "currency"),
        RuntimeValue::String("USD".into())
    );
    let program = compiler::compile(
        "//@version=6\nstrategy(\"S\", currency = currency.EUR)\ncode = strategy.account_currency",
        Limits::default(),
        &NoLibraries,
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
    assert!(session.evaluate(&bars(2)).is_ok());
    assert_eq!(value(&session, "code"), RuntimeValue::String("EUR".into()));
}

#[test]
fn the_margin_liquidation_price_is_na_with_a_warning() {
    let body = "//@version=6\nstrategy(\"S\")\nprice = strategy.margin_liquidation_price";
    let codes: Vec<String> = compiler::compile(body, Limits::default(), &NoLibraries)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert!(codes.contains(&"PINE7003".to_owned()), "{codes:?}");
    let session = run_strategy(
        "price = strategy.margin_liquidation_price",
        &STRATEGY_PRICES[..2],
    );
    assert_eq!(value(&session, "price"), RuntimeValue::Na);
}

#[test]
fn order_comments_and_trade_excursions_are_readable_per_trade() {
    let source = "//@version=6\n\
        strategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 1)\n\
        if bar_index == 0\n    strategy.entry(\"L\", strategy.long, comment = \"go\")\n\
        if bar_index == 2\n    strategy.close(\"L\", comment = \"done\")\n\
        if bar_index == 4\n    strategy.entry(\"S\", strategy.short, comment = \"fade\")\n\
        entry_comment = strategy.closedtrades.entry_comment(0)\n\
        exit_comment = strategy.closedtrades.exit_comment(0)\n\
        runup = strategy.closedtrades.max_runup(0)\n\
        runup_pct = strategy.closedtrades.max_runup_percent(0)\n\
        drawdown = strategy.closedtrades.max_drawdown(0)\n\
        drawdown_pct = strategy.closedtrades.max_drawdown_percent(0)\n\
        missing_comment = strategy.closedtrades.entry_comment(9)\n\
        open_comment = strategy.opentrades.entry_comment(0)\n\
        open_runup = strategy.opentrades.max_runup(0)\n\
        open_drawdown_pct = strategy.opentrades.max_drawdown_percent(0)\n\
        no_open = strategy.opentrades.max_runup(3)";
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
    session
        .evaluate(&flat_bars(&STRATEGY_PRICES[..7]))
        .expect("runs");
    let text = |name: &str| value(&session, name);
    assert_eq!(text("entry_comment"), RuntimeValue::String("go".into()));
    assert_eq!(text("exit_comment"), RuntimeValue::String("done".into()));
    // Long from 100, the bars reached 111 and 99 before it closed at the open of the next bar.
    assert_eq!(text("runup"), RuntimeValue::Float(11.0));
    assert_eq!(text("drawdown"), RuntimeValue::Float(1.0));
    assert!((text("runup_pct").number().unwrap() - 11.0).abs() < 1e-9);
    assert!((text("drawdown_pct").number().unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(text("missing_comment"), RuntimeValue::Na);
    // The short from 100 is open: it fell to 89 and rose to 101.
    assert_eq!(text("open_comment"), RuntimeValue::String("fade".into()));
    assert_eq!(text("open_runup"), RuntimeValue::Float(11.0));
    assert!((text("open_drawdown_pct").number().unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(text("no_open"), RuntimeValue::Na);
}

#[test]
fn an_exit_picks_its_comment_by_the_level_that_filled_it() {
    let source = "//@version=6\n\
        strategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 1)\n\
        if bar_index == 0\n    strategy.entry(\"L\", strategy.long)\n\
        if bar_index == 1\n    strategy.exit(\"X\", \"L\", limit = 105, stop = 95, comment = \"plain\", comment_profit = \"target\", comment_loss = \"stopped\")\n\
        text = strategy.closedtrades.exit_comment(0)";
    let session = {
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
        // Bar 2 reaches 106 (the target) before it falls anywhere near the stop.
        let mut bars = flat_bars(&[100.0, 100.0, 100.0]);
        bars[2].high = 106.0;
        bars[2].open = 100.0;
        session.evaluate(&bars).expect("runs");
        session
    };
    assert_eq!(
        value(&session, "text"),
        RuntimeValue::String("target".into())
    );
}

#[test]
fn gross_loss_percent_is_the_losses_over_the_initial_capital() {
    let session = run_strategy("pct = strategy.grossloss_percent", &STRATEGY_PRICES);
    // 40 of loss on 1000.
    assert!((value(&session, "pct").number().unwrap() - 4.0).abs() < 1e-9);
}

/// Runs a strategy `body` (after its declaration, 1 unit per entry) over bars at `prices`.
fn run_priced_strategy(declaration: &str, body: &str, prices: &[f64]) -> Session {
    let source = format!("//@version=6\n{declaration}\n{body}");
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
    if let Err(error) = session.evaluate(&flat_bars(prices)) {
        panic!("{error:?}");
    }
    session
}

const ONE_UNIT: &str = "strategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 1)";

#[test]
fn default_entry_qty_is_what_an_entry_would_get_at_a_price() {
    let cash = run_priced_strategy(
        "strategy(\"S\", default_qty_type = strategy.cash, default_qty_value = 500)",
        "qty = strategy.default_entry_qty(50)\nnone = strategy.default_entry_qty(na)",
        &[100.0, 100.0],
    );
    assert_eq!(value(&cash, "qty"), RuntimeValue::Float(10.0));
    assert_eq!(value(&cash, "none"), RuntimeValue::Na);
    let fixed = run_priced_strategy(
        "strategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 3)",
        "qty = strategy.default_entry_qty(close)",
        &[100.0, 100.0],
    );
    assert_eq!(value(&fixed, "qty"), RuntimeValue::Float(3.0));
}

#[test]
fn a_drawdown_rule_closes_the_position_and_ends_the_orders() {
    let session = run_priced_strategy(
        ONE_UNIT,
        "strategy.risk.max_drawdown(30, strategy.cash)\n\
         strategy.entry(\"L\", strategy.long)\n\
         size = strategy.position_size\ncount = strategy.closedtrades",
        &[100.0, 100.0, 100.0, 60.0, 60.0, 60.0, 60.0],
    );
    // The entry fills at 100 on bar 1; bar 3 closes 40 lower, past the 30 limit, so the position
    // is closed there and the entries of the later bars are ignored.
    assert_eq!(value(&session, "size"), RuntimeValue::Float(0.0));
    assert_eq!(value(&session, "count"), RuntimeValue::Int(1));
    assert_eq!(session.working.broker.closed_trades[0].exit_price, 60.0);
}

#[test]
fn the_direction_rule_keeps_a_strategy_long_only() {
    let session = run_priced_strategy(
        ONE_UNIT,
        "strategy.risk.allow_entry_in(strategy.direction.long)\n\
         if bar_index == 0\n    strategy.entry(\"S\", strategy.short)\n\
         if bar_index == 2\n    strategy.entry(\"L\", strategy.long)\n\
         size = strategy.position_size",
        &[100.0; 5],
    );
    assert_eq!(value(&session, "size"), RuntimeValue::Float(1.0));
    assert_eq!(session.working.broker.open_trades[0].entry_id, "L");
}

#[test]
fn a_position_size_rule_trims_the_entries() {
    let session = run_priced_strategy(
        "strategy(\"S\", default_qty_type = strategy.fixed, default_qty_value = 4, pyramiding = 3)",
        "strategy.risk.max_position_size(5)\n\
         if bar_index == 0\n    strategy.entry(\"A\", strategy.long)\n\
         if bar_index == 1\n    strategy.entry(\"B\", strategy.long)\n\
         size = strategy.position_size",
        &[100.0; 5],
    );
    assert_eq!(value(&session, "size"), RuntimeValue::Float(5.0));
}

#[test]
fn an_oca_group_cancels_the_orders_that_did_not_fill() {
    let session = run_priced_strategy(
        ONE_UNIT,
        "if bar_index == 0\n\
         \x20   strategy.entry(\"A\", strategy.long, limit = 95, oca_name = \"g\", oca_type = strategy.oca.cancel)\n\
         \x20   strategy.entry(\"B\", strategy.long, limit = 80, oca_name = \"g\", oca_type = strategy.oca.cancel)\n\
         size = strategy.position_size",
        // Bar 2 reaches A's limit; bar 4 would reach B's if it were still there.
        &[100.0, 100.0, 94.0, 94.0, 79.0, 79.0],
    );
    assert_eq!(value(&session, "size"), RuntimeValue::Float(1.0));
    assert_eq!(session.working.broker.open_trades.len(), 1);
}

#[test]
fn the_risk_and_oca_arguments_the_chart_cannot_honour_are_warned_about() {
    let codes = diagnostic_codes(
        "strategy.risk.max_drawdown(10, strategy.percent_of_equity, alert_message = \"stop\")\n\
         strategy.risk.allow_entry_in(strategy.direction.long)\n\
         strategy.entry(\"L\", strategy.long, oca_name = \"g\", oca_type = strategy.oca.reduce)",
    );
    assert!(
        !codes.contains(&"PINE7002".to_owned()),
        "risk rules are enforced now: {codes:?}"
    );
    assert_eq!(
        codes.iter().filter(|c| *c == "PINE7001").count(),
        1,
        "only the alert message: {codes:?}"
    );
}

fn input_schema(body: &str) -> crate::pine::program::InputSchema {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    program.input_schema
}

#[test]
fn a_positional_input_argument_is_read_in_its_place() {
    use crate::pine::program::InputValue;
    let schema = input_schema(
        "length = input.int(14, \"Length\", 1, 200, 2, \"How many bars\", \"row\", \"Settings\", true)\n\
         plot(length)",
    );
    let input = &schema.inputs[0];
    assert_eq!(input.default_value, InputValue::Int(14));
    assert_eq!(input.title.as_deref(), Some("Length"));
    assert_eq!(
        (input.min_value, input.max_value, input.step),
        (Some(1.0), Some(200.0), Some(2.0))
    );
    assert_eq!(input.tooltip.as_deref(), Some("How many bars"));
    assert_eq!(input.inline.as_deref(), Some("row"));
    assert_eq!(input.group.as_deref(), Some("Settings"));
    assert!(input.confirm);
}

#[test]
fn options_in_the_third_place_make_a_choice() {
    use crate::pine::program::InputValue;
    let schema = input_schema(
        "mode = input.int(2, \"Mode\", [1, 2, 3], \"which\")\n\
         kind = input.string(\"b\", \"Kind\", [\"a\", \"b\"])\nplot(mode)",
    );
    assert_eq!(
        schema.inputs[0].options,
        Some(vec![
            InputValue::Int(1),
            InputValue::Int(2),
            InputValue::Int(3)
        ])
    );
    assert_eq!(schema.inputs[0].min_value, None);
    assert_eq!(schema.inputs[0].tooltip.as_deref(), Some("which"));
    assert_eq!(schema.inputs[1].options.as_ref().map(Vec::len), Some(2));
}

#[test]
fn the_input_kinds_that_were_plain_strings_and_floats_have_their_own_control() {
    use crate::pine::program::{InputControl, InputValue};
    let schema = input_schema(
        "level = input.price(100.5, \"Level\")\n\
         pair = input.symbol(\"BINANCE:BTCUSDT\", \"Symbol\")\n\
         frame = input.timeframe(\"60\", \"Timeframe\")\n\
         picked = input.timeframe(\"D\", \"Options\", [\"D\", \"W\"])\n\
         hours = input.session(\"0930-1600\", \"Session\")\n\
         notes = input.text_area(\"hello\", \"Notes\", \"free text\", \"Group\")\n\
         generic = input(3.5, \"Generic\")\n\
         plot(level)",
    );
    let found = |id: &str| schema.inputs.iter().find(|i| i.id == id).unwrap();
    assert_eq!(found("level").control, InputControl::Price);
    assert_eq!(found("level").kind, crate::pine::token::ValueType::Float);
    assert_eq!(found("level").default_value, InputValue::Float(100.5));
    assert_eq!(found("pair").control, InputControl::Symbol);
    assert_eq!(found("frame").control, InputControl::Timeframe);
    assert_eq!(found("picked").options.as_ref().map(Vec::len), Some(2));
    assert_eq!(found("hours").control, InputControl::Session);
    assert_eq!(found("notes").control, InputControl::TextArea);
    assert_eq!(found("notes").tooltip.as_deref(), Some("free text"));
    assert_eq!(found("notes").group.as_deref(), Some("Group"));
    assert_eq!(found("generic").control, InputControl::Standard);
    assert_eq!(
        found("generic").kind,
        crate::pine::token::ValueType::Float,
        "from its default"
    );
}

#[test]
fn the_new_input_functions_return_their_defaults_typed() {
    let session = run(
        "level = input.price(100.5, \"Level\")\n\
         pair = input.symbol(\"BINANCE:BTCUSDT\", \"Symbol\")\n\
         frame = input.timeframe(\"60\", \"Timeframe\")\n\
         hours = input.session(\"0930-1600\", \"Session\")\n\
         notes = input.text_area(\"hello\\nthere\", \"Notes\")\n\
         doubled = level * 2",
        2,
    );
    assert_eq!(value(&session, "level"), RuntimeValue::Float(100.5));
    assert_eq!(value(&session, "doubled"), RuntimeValue::Float(201.0));
    assert_eq!(
        value(&session, "pair"),
        RuntimeValue::String("BINANCE:BTCUSDT".into())
    );
    assert_eq!(value(&session, "frame"), RuntimeValue::String("60".into()));
    assert_eq!(
        value(&session, "hours"),
        RuntimeValue::String("0930-1600".into())
    );
    assert_eq!(
        value(&session, "notes"),
        RuntimeValue::String("hello\nthere".into())
    );
}

#[test]
fn the_input_arguments_that_change_nothing_are_warned_about() {
    let codes = diagnostic_codes(
        "a = input.int(1, \"A\", display = display.none)\n\
         b = input.text_area(\"x\", \"B\", active = close > open)\nplot(a)",
    );
    assert_eq!(
        codes.iter().filter(|c| *c == "PINE7001").count(),
        2,
        "{codes:?}"
    );
}

fn active_inputs(body: &str) -> crate::pine::program::InputSchema {
    input_schema(&format!("{body}\nplot(1)"))
}

#[test]
fn active_greys_an_input_out_by_other_inputs_and_constants() {
    use crate::pine::program::InputValue;
    use std::collections::HashMap;
    let schema = active_inputs(
        "const bool debug = false\n\
         custom = input.bool(false, \"Custom\")\n\
         mode = input.string(\"Fast\", \"Mode\", options = [\"Fast\", \"Slow\"])\n\
         always = input.int(1, \"Always\", active = true)\n\
         never = input.int(1, \"Never\", active = false)\n\
         by_switch = input.int(2, \"By switch\", active = custom)\n\
         by_not = input.int(3, \"By not\", active = not custom)\n\
         by_mode = input.int(4, \"By mode\", active = mode == \"Slow\" and not debug)\n\
         by_constant = input.int(5, \"By constant\", active = debug)\n\
         by_ternary = input.int(6, \"By ternary\", active = custom ? mode == \"Slow\" : true)",
    );
    let active = |id: &str, values: &HashMap<String, InputValue>| {
        let input = schema.inputs.iter().find(|i| i.id == id).unwrap();
        schema.is_active(input, values)
    };
    let none = HashMap::new();
    assert!(active("always", &none) && active("custom", &none) && active("mode", &none));
    assert!(!active("never", &none) && !active("by_constant", &none));
    assert!(!active("by_switch", &none) && active("by_not", &none));
    assert!(!active("by_mode", &none) && active("by_ternary", &none));

    let edited = HashMap::from([
        ("custom".to_owned(), InputValue::Bool(true)),
        ("mode".to_owned(), InputValue::String("Slow".into())),
    ]);
    assert!(active("by_switch", &edited) && !active("by_not", &edited));
    assert!(active("by_mode", &edited), "`not debug` is a constant true");
    assert!(
        !active("by_constant", &edited),
        "`debug` is a constant and stays false"
    );
    assert!(active("by_ternary", &edited));
    assert!(
        !active("never", &edited),
        "nothing re-enables a constant false"
    );
}

#[test]
fn active_compares_numbers_and_follows_a_chain_of_inputs() {
    use crate::pine::program::InputValue;
    use std::collections::HashMap;
    let schema = active_inputs(
        "length = input.int(10, \"Length\")\n\
         smooth = input.bool(true, \"Smooth\", active = length > 5)\n\
         factor = input.float(2.0, \"Factor\", active = smooth)",
    );
    let factor = &schema.inputs[2];
    assert!(schema.is_active(factor, &HashMap::new()));
    let short = HashMap::from([("length".to_owned(), InputValue::Int(3))]);
    assert!(
        schema.is_active(factor, &short),
        "`smooth` still holds true: a greyed-out input keeps its value"
    );
    let switched_off = HashMap::from([("smooth".to_owned(), InputValue::Bool(false))]);
    assert!(!schema.is_active(factor, &switched_off));
    assert!(!schema.is_active(&schema.inputs[1], &short));
}

#[test]
fn active_is_found_by_position_and_on_enum_inputs() {
    let schema = active_inputs(
        "enum Side\n    left\n    right\n\
         on = input.bool(false, \"On\")\n\
         length = input.int(14, \"Length\", 1, 100, 1, \"tip\", \"row\", \"Group\", false, display.all, on)\n\
         side = input.enum(Side.left, \"Side\", active = on)",
    );
    let values = std::collections::HashMap::new();
    assert!(!schema.is_active(&schema.inputs[1], &values));
    assert!(!schema.is_active(&schema.inputs[2], &values));
}

#[test]
fn an_active_the_settings_cannot_know_is_warned_about_and_stays_editable() {
    for condition in [
        "close > open",
        "later",
        "array.size(array.new_int()) > 0",
        "1",
        "na",
    ] {
        let source = format!(
            "//@version=6\nindicator(\"T\")\n\
             a = input.int(1, \"A\", active = {condition})\n\
             later = input.bool(true)\nplot(a)"
        );
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        let warnings: Vec<_> = program
            .diagnostics
            .iter()
            .filter(|d| d.code == "PINE7001")
            .collect();
        assert_eq!(warnings.len(), 1, "{condition}: {:?}", program.diagnostics);
        assert!(
            warnings[0].message.contains("`active` is ignored"),
            "{}",
            warnings[0].message
        );
        let input = &program.input_schema.inputs[0];
        assert!(input.active.is_none());
        assert!(
            program
                .input_schema
                .is_active(input, &std::collections::HashMap::new())
        );
    }
    let honoured = compiler::compile(
        "//@version=6\nindicator(\"T\")\non = input.bool(true)\n\
         a = input.int(1, \"A\", active = on)\nplot(a)",
        Limits::default(),
        &NoLibraries,
    );
    assert!(
        honoured.diagnostics.iter().all(|d| d.code != "PINE7001"),
        "{:?}",
        honoured.diagnostics
    );
}

#[test]
fn constants_fold_comparisons_logic_and_ternaries() {
    let session = run(
        "const int n = 3\nconst bool big = n > 2 and not (n == 4)\n\
         const string word = big ? \"yes\" : \"no\"\n\
         x = input.bool(big)\ny = input.string(word)",
        2,
    );
    assert_eq!(value(&session, "x"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "y"), RuntimeValue::String("yes".into()));
}

#[test]
fn array_functions_take_the_reference_parameter_names() {
    let session = run(
        "a = array.from(1, 3, 5, 7)\nb = array.from(9)\n\
         index = array.binary_search(a, val = 5)\n\
         second_largest = array.max(a, nth = 1)\n\
         third_smallest = a.min(2)\n\
         array.concat(id1 = a, id2 = b)\n\
         size = array.size(a)\n\
         cov = array.covariance(id1 = array.from(1, 2, 3), id2 = array.from(3, 2, 1))",
        2,
    );
    assert_eq!(value(&session, "index"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "second_largest"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "third_smallest"), RuntimeValue::Float(5.0));
    assert_eq!(value(&session, "size"), RuntimeValue::Int(5));
    assert!(value(&session, "cov").number().unwrap() < 0.0);
}
