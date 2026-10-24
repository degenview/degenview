//! End-to-end checks of `request.footprint()`: each test compiles a small original script and runs
//! it over 5-minute chart bars whose 1-minute intrabars a fake provider serves, then reads the
//! variables the last bar left behind.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_support::{SecurityDataProvider, SecurityKey};
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::{RunError, Session};
use crate::pine::token::Limits;

const MINUTE: i64 = 60_000;

fn candle(open_time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + MINUTE - 1,
        open,
        high,
        low,
        close,
        volume,
    }
}

/// Chart bars `spacing` minutes long, from time 0.
fn chart(count: usize, spacing: i64) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let open_time = index as i64 * spacing * MINUTE;
            let mut bar = candle(open_time, 100.0, 105.0, 100.0, 105.0, 50.0);
            bar.close_time = open_time + spacing * MINUTE - 1;
            bar
        })
        .collect()
}

/// Five 1-minute intrabars per 5-minute bar, each filling one whole row of height 1: 100-101 up to
/// 104-105. All buy, except the second, which closed down and so is 10 sold on 101-102.
fn intrabars(count: usize) -> Vec<Kline> {
    let mut candles = Vec::new();
    for bar in 0..count as i64 {
        for step in 0..5 {
            let open_time = (bar * 5 + step) * MINUTE;
            let low = 100.0 + step as f64;
            candles.push(if step == 1 {
                candle(open_time, low + 1.0, low + 1.0, low, low, 10.0)
            } else {
                candle(open_time, low, low + 1.0, low, low + 1.0, 10.0)
            });
        }
    }
    candles
}

struct Intrabars(Vec<Kline>);

impl SecurityDataProvider for Intrabars {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        (key.interval == 60.0 && key.symbol.is_empty()).then(|| self.0.clone())
    }
}

fn session(body: &str, mintick: f64, provider: Option<Vec<Kline>>) -> Session {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        Some(mintick),
        SymbolInfo::default(),
        provider.map(|candles| Box::new(Intrabars(candles)) as Box<dyn SecurityDataProvider>),
        None,
    )
}

/// `body` run over three 5-minute bars with the fixture's intrabars.
fn run(body: &str) -> Session {
    let mut session = session(body, 1.0, Some(intrabars(3)));
    if let Err(error) = session.evaluate(&chart(3, 5)) {
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

fn float(session: &Session, name: &str) -> f64 {
    value(session, name)
        .number()
        .unwrap_or_else(|| panic!("{name} is {:?}", value(session, name)))
}

fn failure(mut session: Session, bars: &[Kline]) -> String {
    match session.evaluate(bars) {
        Err(RunError::Script(diagnostic)) => format!("{} {}", diagnostic.code, diagnostic.message),
        other => panic!("expected a script error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn the_bar_totals_are_the_intrabars_split_by_direction() {
    let session = run("footprint fp = request.footprint(1)\n\
         buy = na(fp) ? na : fp.buy_volume()\n\
         sell = na(fp) ? na : fp.sell_volume()\n\
         total = na(fp) ? na : fp.total_volume()\n\
         delta = na(fp) ? na : fp.delta()\n\
         plot(close)");
    assert_eq!(float(&session, "buy"), 40.0);
    assert_eq!(float(&session, "sell"), 10.0);
    assert_eq!(float(&session, "total"), 50.0);
    assert_eq!(float(&session, "delta"), 30.0);
}

#[test]
fn function_syntax_and_method_syntax_agree() {
    let session = run("footprint fp = request.footprint(1)\n\
         method_delta = fp.delta()\n\
         function_delta = footprint.delta(fp)\n\
         method_up = fp.poc().up_price()\n\
         function_up = volume_row.up_price(footprint.poc(fp))\n\
         plot(close)");
    assert_eq!(float(&session, "method_delta"), 30.0);
    assert_eq!(float(&session, "function_delta"), 30.0);
    assert_eq!(float(&session, "method_up"), 103.0);
    assert_eq!(float(&session, "function_up"), 103.0);
}

#[test]
fn the_poc_and_value_area_rows_are_volume_rows() {
    let session = run("footprint fp = request.footprint(1, 70)\n\
         volume_row poc = fp.poc()\n\
         volume_row vah = fp.vah()\n\
         volume_row val = fp.val()\n\
         poc_down = poc.down_price()\n\
         poc_total = poc.total_volume()\n\
         vah_up = vah.up_price()\n\
         val_down = val.down_price()\n\
         plot(close)");
    // Every row holds 10, so the point of control is the middle one, and 70 % of 50 takes all five.
    assert_eq!(float(&session, "poc_down"), 102.0);
    assert_eq!(float(&session, "poc_total"), 10.0);
    assert_eq!(float(&session, "vah_up"), 105.0);
    assert_eq!(float(&session, "val_down"), 100.0);
}

#[test]
fn the_rows_are_an_array_from_the_lowest_price_up() {
    let session = run("footprint fp = request.footprint(1)\n\
         array<volume_row> rows = fp.rows()\n\
         count = rows.size()\n\
         first_down = rows.first().down_price()\n\
         sold_in_second = rows.get(1).sell_volume()\n\
         sold = 0.0\n\
         for row in rows\n\
         \x20   sold += row.sell_volume()\n\
         plot(close)");
    assert_eq!(float(&session, "count"), 5.0);
    assert_eq!(float(&session, "first_down"), 100.0);
    assert_eq!(float(&session, "sold_in_second"), 10.0);
    assert_eq!(float(&session, "sold"), 10.0);
}

#[test]
fn a_row_is_found_by_a_price_inside_it_and_na_outside() {
    let session = run("footprint fp = request.footprint(1)\n\
         inside = fp.get_row_by_price(103.5).down_price()\n\
         outside = na(fp.get_row_by_price(99.0))\n\
         plot(close)");
    assert_eq!(float(&session, "inside"), 103.0);
    assert_eq!(value(&session, "outside"), RuntimeValue::Bool(true));
}

#[test]
fn imbalances_follow_the_percentage_argument() {
    let source = |percent: &str| {
        format!(
            "footprint fp = request.footprint(1, 70, {percent})\n\
             array<volume_row> rows = fp.rows()\n\
             buy_at_2 = rows.get(2).has_buy_imbalance()\n\
             sell_at_1 = rows.get(1).has_sell_imbalance()\n\
             buy_at_0 = rows.get(0).has_buy_imbalance()\n\
             plot(close)"
        )
    };
    // Row 1 sold 10 and row 2 bought 10 against it, so each is an imbalance at 100 %, not at 300 %.
    let level = run(&source("imbalance_percent = 100"));
    assert_eq!(value(&level, "buy_at_2"), RuntimeValue::Bool(true));
    assert_eq!(value(&level, "sell_at_1"), RuntimeValue::Bool(true));
    assert_eq!(value(&level, "buy_at_0"), RuntimeValue::Bool(false));
    let default = run(&source("300"));
    assert_eq!(value(&default, "buy_at_2"), RuntimeValue::Bool(false));
    assert_eq!(value(&default, "sell_at_1"), RuntimeValue::Bool(false));
}

#[test]
fn a_footprint_can_be_named_in_declarations_functions_and_methods() {
    let session = run("var footprint held = na\n\
         footprint fp = request.footprint(1)\n\
         held := fp\n\
         total_of(footprint f) =>\n\
         \x20   f.total_volume()\n\
         method top(volume_row this) =>\n\
         \x20   this.up_price()\n\
         method stats(footprint this) =>\n\
         \x20   this.buy_volume()\n\
         from_function = total_of(held)\n\
         from_method = fp.poc().top()\n\
         from_footprint_method = fp.stats()\n\
         plot(close)");
    assert_eq!(float(&session, "from_function"), 50.0);
    assert_eq!(float(&session, "from_method"), 103.0);
    assert_eq!(float(&session, "from_footprint_method"), 40.0);
}

#[test]
fn an_earlier_bars_footprint_is_reached_with_history() {
    let session = run("footprint fp = request.footprint(1)\n\
         footprint before = fp[1]\n\
         known = not na(before)\n\
         before_total = na(before) ? na : before.total_volume()\n\
         plot(close)");
    assert_eq!(value(&session, "known"), RuntimeValue::Bool(true));
    assert_eq!(float(&session, "before_total"), 50.0);
}

#[test]
fn a_bar_without_intrabars_has_no_footprint() {
    // The provider has candles for the first bar only.
    let mut missing = session(
        "footprint fp = request.footprint(1)\n\
         none = na(fp)\n\
         plot(close)",
        1.0,
        Some(intrabars(1)),
    );
    missing.evaluate(&chart(3, 5)).unwrap();
    assert_eq!(value(&missing, "none"), RuntimeValue::Bool(true));
    // No provider at all.
    let mut nothing = session(
        "footprint fp = request.footprint(1)\n\
         none = na(fp)\n\
         plot(close)",
        1.0,
        None,
    );
    nothing.evaluate(&chart(3, 5)).unwrap();
    assert_eq!(value(&nothing, "none"), RuntimeValue::Bool(true));
}

#[test]
fn a_one_minute_chart_has_no_footprint() {
    let mut one_minute = session(
        "footprint fp = request.footprint(1)\n\
         none = na(fp)\n\
         plot(close)",
        1.0,
        Some(intrabars(3)),
    );
    one_minute.evaluate(&chart(15, 1)).unwrap();
    assert_eq!(value(&one_minute, "none"), RuntimeValue::Bool(true));
}

#[test]
fn rows_a_tick_tall_on_a_wide_bar_are_no_footprint() {
    // 5 of range at a tick of 0.001 is five thousand rows.
    let mut wide = session(
        "footprint fp = request.footprint(1)\n\
         none = na(fp)\n\
         plot(close)",
        0.001,
        Some(intrabars(3)),
    );
    wide.evaluate(&chart(3, 5)).unwrap();
    assert_eq!(value(&wide, "none"), RuntimeValue::Bool(true));
}

#[test]
fn a_script_may_repeat_its_request_but_not_change_it() {
    let same = run("a = request.footprint(1, 70)\n\
         b = request.footprint(1, 70)\n\
         both = not na(a) and not na(b)\n\
         plot(close)");
    assert_eq!(value(&same, "both"), RuntimeValue::Bool(true));
    let different = session(
        "a = request.footprint(1)\n\
         b = request.footprint(2)\n\
         plot(close)",
        1.0,
        Some(intrabars(3)),
    );
    let message = failure(different, &chart(3, 5));
    assert!(message.starts_with("PINE4033"), "{message}");
}

#[test]
fn bad_arguments_are_errors() {
    for (arguments, code) in [
        ("0", "PINE4035"),
        ("1.5", "PINE4035"),
        ("na", "PINE4035"),
        ("1, 0", "PINE4035"),
        ("1, 101", "PINE4035"),
        ("1, 70, 0", "PINE4035"),
    ] {
        let script = session(
            &format!("fp = request.footprint({arguments})\nplot(close)"),
            1.0,
            Some(intrabars(3)),
        );
        let message = failure(script, &chart(3, 5));
        assert!(message.starts_with(code), "{arguments}: {message}");
    }
}

#[test]
fn a_footprint_function_needs_an_id_that_is_not_na() {
    let script = session(
        "footprint fp = na\n\
         x = footprint.delta(fp)\n\
         plot(close)",
        1.0,
        Some(intrabars(3)),
    );
    let message = failure(script, &chart(3, 5));
    assert!(message.starts_with("PINE4036"), "{message}");
}

#[test]
fn a_request_inside_another_request_is_an_error() {
    let script = session(
        "x = request.security(syminfo.tickerid, \"5\", request.footprint(1))\n\
         plot(close)",
        1.0,
        Some(intrabars(3)),
    );
    let message = failure(script, &chart(3, 5));
    assert!(message.starts_with("PINE4034"), "{message}");
}

#[test]
fn the_request_compiles_with_one_warning_about_its_limits() {
    let source = "//@version=6\nindicator(\"T\")\nfootprint fp = request.footprint(1)\nplot(close)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let codes: Vec<&str> = program
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(codes, ["PINE7001"], "{:?}", program.diagnostics);
    assert!(
        program.diagnostics[0].message.contains("1-minute"),
        "{}",
        program.diagnostics[0].message
    );
}

/// Scripts in the shape TradingView's documentation shows (written fresh here) compile with no
/// error: the labels, boxes and linefills they draw are the engine's own.
#[test]
fn scripts_in_the_documented_shapes_compile() {
    let summary = "//@version=6\n\
        indicator(\"Footprint summary\", overlay = true, max_labels_count = 50)\n\
        int ticksInput = input.int(100, \"Ticks per row\", minval = 1)\n\
        int vaInput = input.int(70, \"Value area %\", minval = 1)\n\
        footprint reqFootprint = request.footprint(ticksInput, vaInput)\n\
        if barstate.islast and not na(reqFootprint)\n\
        \x20   float buyVol = reqFootprint.buy_volume()\n\
        \x20   float sellVol = reqFootprint.sell_volume()\n\
        \x20   float delta = reqFootprint.delta()\n\
        \x20   volume_row poc = reqFootprint.poc()\n\
        \x20   volume_row vah = reqFootprint.vah()\n\
        \x20   volume_row val = reqFootprint.val()\n\
        \x20   color tint = delta >= 0 ? color.teal : color.maroon\n\
        \x20   label.new(bar_index, high, \"Buy: \" + str.tostring(buyVol, format.volume) + \"\\nPOC: \" + str.tostring(poc.up_price()), color = tint, textcolor = color.white)\n\
        \x20   linefill.new(line.new(bar_index, vah.up_price(), bar_index - 1, vah.up_price()), line.new(bar_index, val.down_price(), bar_index - 1, val.down_price()), color.new(color.orange, 80))\n\
        plot(not na(reqFootprint) ? reqFootprint.total_volume() : na, \"Total\", display = display.data_window)\n";
    let rows = "//@version=6\n\
        indicator(\"Footprint rows\", max_boxes_count = 500)\n\
        int imbalanceInput = input.int(300, \"Imbalance %\", 1)\n\
        footprint reqFootprint = request.footprint(100, imbalance_percent = imbalanceInput)\n\
        if barstate.islast and not na(reqFootprint)\n\
        \x20   array<volume_row> volumeRowsArray = reqFootprint.rows()\n\
        \x20   for row in volumeRowsArray\n\
        \x20       bool buyImbalance = row.has_buy_imbalance()\n\
        \x20       bool sellImbalance = row.has_sell_imbalance()\n\
        \x20       float upPrice = row.up_price()\n\
        \x20       float dnPrice = row.down_price()\n\
        \x20       box.new(bar_index, upPrice, bar_index + 1, dnPrice, #787b8650, 1, text = str.tostring(row.delta()))\n\
        plot(close)\n";
    for (source, draws_labels) in [(summary, true), (rows, false)] {
        let program = compiler::compile(source, Limits::default(), &NoLibraries);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        let errors: Vec<_> = program
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.severity == crate::pine::token::DiagnosticSeverity::Error
            })
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        // And they run: one row 100 ticks tall holds the whole bar.
        let mut running = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            Some(0.01),
            SymbolInfo::default(),
            Some(Box::new(Intrabars(intrabars(3)))),
            None,
        );
        if let Err(error) = running.evaluate(&chart(3, 5)) {
            panic!("{error:?}");
        }
        let output = running.output();
        if draws_labels {
            assert!(!output.labels.is_empty());
        } else {
            assert!(!output.boxes.is_empty());
        }
    }
}

/// A provider whose candles can grow after the first request, like a live market.
#[derive(Clone)]
struct Growing(std::sync::Arc<std::sync::Mutex<Vec<Kline>>>);

impl SecurityDataProvider for Growing {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        (key.interval == 60.0).then(|| self.0.lock().unwrap().clone())
    }
}

/// The last value of the plot of the footprint's total volume, `-1` where it is `na`.
fn total_after(
    controller: &mut crate::pine::execution::ExecutionController,
    bars: &[Kline],
    now_ms: i64,
) -> Option<f64> {
    let update = controller.update(bars, None, now_ms);
    assert!(update.error.is_none(), "{:?}", update.error);
    update.output.plots[0].values.last().copied().flatten()
}

fn controller(provider: Growing) -> crate::pine::execution::ExecutionController {
    crate::pine::execution::ExecutionController::new(
        "//@version=6\nindicator(\"T\")\n\
         footprint fp = request.footprint(1)\n\
         plot(na(fp) ? -1.0 : fp.total_volume())",
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &NoLibraries,
        Some(Box::new(provider)),
        None,
    )
    .unwrap()
}

#[test]
fn a_bar_that_opens_after_the_first_request_gets_its_footprint() {
    let candles = std::sync::Arc::new(std::sync::Mutex::new(intrabars(3)));
    let mut controller = controller(Growing(candles.clone()));
    let closed = i64::MAX / 2;
    assert_eq!(
        total_after(&mut controller, &chart(3, 5), closed),
        Some(50.0)
    );
    // A fourth bar closes, and its candles arrive with it.
    *candles.lock().unwrap() = intrabars(4);
    assert_eq!(
        total_after(&mut controller, &chart(4, 5), closed),
        Some(50.0)
    );
}

#[test]
fn the_forming_bar_gets_the_intrabars_so_far() {
    let candles = std::sync::Arc::new(std::sync::Mutex::new(intrabars(3)));
    let mut controller = controller(Growing(candles.clone()));
    assert_eq!(
        total_after(&mut controller, &chart(3, 5), i64::MAX / 2),
        Some(50.0)
    );
    // The fourth bar has run for three minutes of five: three of its intrabars are in.
    *candles.lock().unwrap() = intrabars(4)[..18].to_vec();
    let three_minutes_in = (15 + 3) * MINUTE;
    assert_eq!(
        total_after(&mut controller, &chart(4, 5), three_minutes_in),
        Some(30.0)
    );
}

#[test]
fn older_history_loaded_later_gets_footprints_too() {
    // The chart first holds bars 1 and 2; then bar 0 is prepended.
    let candles = std::sync::Arc::new(std::sync::Mutex::new(intrabars(3)[5..].to_vec()));
    let mut controller = crate::pine::execution::ExecutionController::new(
        "//@version=6\nindicator(\"T\")\n\
         footprint fp = request.footprint(1)\n\
         plot(na(fp) ? -1.0 : fp.total_volume())",
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &NoLibraries,
        Some(Box::new(Growing(candles.clone()))),
        None,
    )
    .unwrap();
    let closed = i64::MAX / 2;
    let bars = chart(3, 5);
    let update = controller.update(&bars[1..], None, closed);
    assert_eq!(update.output.plots[0].values, [Some(50.0), Some(50.0)]);
    *candles.lock().unwrap() = intrabars(3);
    let update = controller.update(&bars, None, closed);
    assert_eq!(
        update.output.plots[0].values,
        [Some(50.0), Some(50.0), Some(50.0)]
    );
}
