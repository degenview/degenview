//! Pine's builtin symbol names, ported from `PineSymbolCatalog.swift`. This is the single table
//! the structure validator, the identifier rules and `compiler::is_named_constant` read to tell a
//! builtin from a user symbol without compiling. It mirrors the names the runtime dispatches and the
//! tables `builtin_types`/`builtins`/`alert_frequency`/`outputs`/`time` already own; names Pine
//! defines but DegenView does not implement are listed under the `reserved`/`unimplemented` sets.

/// Builtin variables that change per bar or per symbol (`close`, `bar_index`, `syminfo.tickerid`).
pub const VARIABLES: &[&str] = &[
    "open",
    "high",
    "low",
    "close",
    "volume",
    "hl2",
    "hlc3",
    "ohlc4",
    "hlcc4",
    "ta.tr",
    "strategy.position_size",
    "strategy.position_avg_price",
    "strategy.equity",
    "strategy.netprofit",
    "strategy.openprofit",
    "strategy.initial_capital",
    "strategy.grossprofit",
    "strategy.grossloss",
    "time",
    "time_close",
    "bar_index",
    "last_bar_index",
    "last_bar_time",
    "timenow",
    "time_tradingday",
    "chart.left_visible_bar_time",
    "chart.right_visible_bar_time",
    "year",
    "month",
    "dayofmonth",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "strategy.closedtrades",
    "strategy.opentrades",
    "strategy.wintrades",
    "strategy.losstrades",
    "syminfo.ticker",
    "syminfo.tickerid",
    "syminfo.currency",
    "syminfo.type",
    "syminfo.root",
    "syminfo.prefix",
    "syminfo.timezone",
    "syminfo.basecurrency",
    "syminfo.mintick",
    "syminfo.pointvalue",
    "chart.fg_color",
    "chart.bg_color",
    "barstate.isfirst",
    "barstate.islast",
    "barstate.ishistory",
    "barstate.isrealtime",
    "barstate.isnew",
    "barstate.isconfirmed",
    "barstate.islastconfirmedhistory",
    "timeframe.period",
    "timeframe.multiplier",
    "timeframe.isdaily",
    "timeframe.isweekly",
    "timeframe.ismonthly",
    "timeframe.isseconds",
    "timeframe.isminutes",
    "timeframe.isintraday",
    "timeframe.isdwm",
    "timeframe.isticks",
    // Pine variables DegenView does not implement (reading one gives `na`).
    "weekofyear",
    "session.isfirstbar",
    "session.isfirstbar_regular",
    "session.islastbar",
    "session.islastbar_regular",
    "session.ismarket",
    "session.ispremarket",
    "session.ispostmarket",
    "syminfo.session",
    "syminfo.mincontract",
    "syminfo.description",
    "syminfo.volumetype",
];

/// Builtins callable without a namespace (`plot`, `na`, `timestamp`, `year`). Namespaced calls
/// (`ta.ema`) are not listed: a call on a builtin namespace is a builtin. `color` is a cast too,
/// but Pine treats it as a namespace first, so [`is_function`] does not protect it.
pub const FUNCTIONS: &[&str] = &[
    "na",
    "nz",
    "int",
    "float",
    "bool",
    "string",
    "color",
    "timestamp",
    "alert",
    "alertcondition",
    "indicator",
    "strategy",
    "library",
    "input",
    "plot",
    "hline",
    "plotshape",
    "plotchar",
    "plotcandle",
    "fill",
    "bgcolor",
    "barcolor",
    "year",
    "month",
    "dayofmonth",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "fixnan",
    "plotarrow",
    "plotbar",
];

/// Handle and collection type names; these are namespaces, not protected from shadowing.
pub const OBJECT_TYPES: &[&str] = &[
    "line", "label", "box", "table", "array", "matrix", "map", "object",
];

/// Pine named constants DegenView's tables do not own. Each evaluates to its own name, so a name
/// outside this catalog (a typo) is an error rather than a string.
pub const RESERVED_CONSTANTS: &[&str] = &[
    "session.regular",
    "session.extended",
    "xloc.bar_index",
    "xloc.bar_time",
    "yloc.price",
    "yloc.abovebar",
    "yloc.belowbar",
    "barmerge.gaps_on",
    "barmerge.gaps_off",
    "barmerge.lookahead_on",
    "barmerge.lookahead_off",
    "scale.left",
    "scale.right",
    "scale.none",
    "adjustment.none",
    "adjustment.splits",
    "adjustment.dividends",
    "backadjustment.inherit",
    "backadjustment.off",
    "backadjustment.on",
    "settlement_as_close.inherit",
    "settlement_as_close.off",
    "settlement_as_close.on",
    "font.family_default",
    "font.family_monospace",
    "text.align_left",
    "text.align_center",
    "text.align_right",
    "text.align_top",
    "text.align_bottom",
    "text.format_none",
    "text.format_bold",
    "text.format_italic",
    "text.wrap_none",
    "text.wrap_auto",
    "display.pine_screener",
    "plot.style_line",
    "plot.style_linebr",
    "plot.style_stepline",
    "plot.style_stepline_diamond",
    "plot.style_steplinebr",
    "plot.style_histogram",
    "plot.style_columns",
    "plot.style_circles",
    "plot.style_cross",
    "plot.style_area",
    "plot.style_areabr",
    "plot.linestyle_solid",
    "plot.linestyle_dotted",
    "plot.linestyle_dashed",
    "hline.style_solid",
    "hline.style_dotted",
    "hline.style_dashed",
    "currency.AED",
    "currency.ARS",
    "currency.AUD",
    "currency.BDT",
    "currency.BHD",
    "currency.BRL",
    "currency.BTC",
    "currency.CAD",
    "currency.CHF",
    "currency.CLP",
    "currency.CNY",
    "currency.COP",
    "currency.CZK",
    "currency.DKK",
    "currency.EGP",
    "currency.ETH",
    "currency.EUR",
    "currency.GBP",
    "currency.HKD",
    "currency.HUF",
    "currency.IDR",
    "currency.ILS",
    "currency.INR",
    "currency.ISK",
    "currency.JPY",
    "currency.KES",
    "currency.KRW",
    "currency.KWD",
    "currency.LKR",
    "currency.MAD",
    "currency.MXN",
    "currency.MYR",
    "currency.NGN",
    "currency.NOK",
    "currency.NONE",
    "currency.NZD",
    "currency.PEN",
    "currency.PHP",
    "currency.PKR",
    "currency.PLN",
    "currency.QAR",
    "currency.RON",
    "currency.RUB",
    "currency.SAR",
    "currency.SEK",
    "currency.SGD",
    "currency.THB",
    "currency.TND",
    "currency.TRY",
    "currency.TWD",
    "currency.UAH",
    "currency.USD",
    "currency.USDT",
    "currency.UYU",
    "currency.VND",
    "currency.ZAR",
];

/// Pine variables this release does not implement (they evaluate to `na`).
pub const UNIMPLEMENTED_VARIABLES: &[&str] = &[
    "weekofyear",
    "session.isfirstbar",
    "session.isfirstbar_regular",
    "session.islastbar",
    "session.islastbar_regular",
    "session.ismarket",
    "session.ispremarket",
    "session.ispostmarket",
    "syminfo.session",
    "syminfo.mincontract",
    "syminfo.description",
    "syminfo.volumetype",
    "syminfo.shares_outstanding_total",
    "syminfo.shares_outstanding_float",
    "syminfo.employees",
    "syminfo.sector",
    "syminfo.industry",
    "syminfo.country",
    "syminfo.expiration_date",
    "syminfo.current_contract",
    "syminfo.main_tickerid",
    "syminfo.target_price_average",
    "syminfo.target_price_high",
    "syminfo.target_price_low",
    "syminfo.target_price_median",
    "syminfo.target_price_estimates",
    "syminfo.target_price_date",
    "syminfo.recommendations_buy",
    "syminfo.recommendations_buy_strong",
    "syminfo.recommendations_date",
    "syminfo.recommendations_hold",
    "syminfo.recommendations_sell",
    "syminfo.recommendations_sell_strong",
    "syminfo.recommendations_total",
];

/// Pine functions this release does not implement (a call is a runtime error). None today.
pub const RESERVED_FUNCTIONS: &[&str] = &[];

/// Whether `name` is a builtin variable.
pub fn is_variable(name: &str) -> bool {
    VARIABLES.contains(&name)
}

/// Whether `name` is a builtin function a user may not shadow. Object types and `color` stay free.
pub fn is_function(name: &str) -> bool {
    FUNCTIONS.contains(&name) && name != "color" && !OBJECT_TYPES.contains(&name)
}

/// Whether `name` is a named constant DegenView's other tables do not own.
pub fn is_reserved_constant(name: &str) -> bool {
    RESERVED_CONSTANTS.contains(&name)
}

/// Whether `name` is a Pine variable this release does not implement.
pub fn is_unimplemented_variable(name: &str) -> bool {
    UNIMPLEMENTED_VARIABLES.contains(&name)
}

/// Whether `name` is a Pine function this release does not implement.
pub fn is_unimplemented_function(name: &str) -> bool {
    RESERVED_FUNCTIONS.contains(&name)
}

/// The direct members of a namespace path (`"ta"`, `"chart.point"`) for editor completion. The
/// member index is not ported yet, so this always returns nothing — see
/// `PineSymbolCatalog+Members.swift`.
pub fn members(_of: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_and_functions_are_listed() {
        assert!(
            is_variable("close")
                && is_variable("barstate.isconfirmed")
                && is_variable("timeframe.period")
        );
        assert!(!is_variable("plot") && !is_variable("not_a_builtin"));
        assert!(is_function("plot") && is_function("na") && is_function("alertcondition"));
        assert!(!is_function("color"), "color is a namespace first");
        assert!(!is_function("line"), "object types stay free");
        assert!(
            !is_function("ta.ema"),
            "namespaced calls are not unqualified functions"
        );
    }

    #[test]
    fn reserved_and_unimplemented_are_flagged() {
        assert!(is_reserved_constant("plot.style_stepline"));
        assert!(is_reserved_constant("currency.USD"));
        assert!(is_reserved_constant("xloc.bar_time"));
        assert!(!is_reserved_constant("color.red"));
        assert!(is_unimplemented_variable("weekofyear"));
        assert!(!is_unimplemented_function("fixnan"));
        assert!(!is_unimplemented_function("plotarrow"));
        assert!(!is_unimplemented_variable("close"));
    }
}
