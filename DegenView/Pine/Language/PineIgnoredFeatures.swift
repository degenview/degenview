import Foundation

/// What TradingView offers that this engine accepts and then does not act on. Each use raises a warning
/// (`PineIgnoredFeatureValidator`) so a script that looks right is not quietly different here.
///
/// An entry belongs here when ignoring it can change what the user sees or what a strategy does. Pure hints
/// with no observable effect (`max_bars_back`, `dynamic_requests`) are left out. Every explanation says what
/// the engine does instead. Confirm an entry is really never read by its handler in `Pine/Runtime` before
/// adding it, and remove it when the engine starts to honour it.
enum PineIgnoredFeatures {
    /// Arguments, by function. A declaration (`indicator`, `strategy`, `library`) is keyed by its name.
    static let arguments: [String: [String: String]] = {
        var table: [String: [String: String]] = [
            "indicator": declaration,
            "library": declaration,
            "strategy": declaration.merging([
                "margin_long": "There is no margin model, so no margin call ever closes a position.",
                "margin_short": "There is no margin model, so no margin call ever closes a position.",
            ]) { first, _ in first },
            "strategy.entry": entry,
            "strategy.order": entry,
            "strategy.exit": [
                "comment": "Exit comments are not shown in the trade list.",
                "comment_profit": "Exit comments are not shown in the trade list.",
                "comment_loss": "Exit comments are not shown in the trade list.",
                "comment_trailing": "Exit comments are not shown in the trade list.",
                "alert_message": "Order fill alerts are not sent.",
                "alert_profit": "Order fill alerts are not sent.",
                "alert_loss": "Order fill alerts are not sent.",
                "alert_trailing": "Order fill alerts are not sent.",
                "oca_name": "Orders are never grouped; each fills on its own.",
            ],
            "strategy.close": close,
            "strategy.close_all": close,
            "plot": [
                "offset": "The plot is not shifted; it is drawn on the bar that produced each value.",
                "trackprice": "No price line is drawn across the chart.",
                "join": "Isolated points are not connected.",
                "editable": "Plot settings cannot be edited from the chart.",
                "show_last": "The plot is drawn on every bar, not only the last ones.",
                "format": "The value is formatted by the chart's own rules.",
                "precision": "The value is formatted by the chart's own rules.",
                "force_overlay": "The plot stays in the pane the script declares.",
                "linestyle": "The line is always solid.",
            ],
            "hline": [
                "linestyle": "The line is always solid.",
                "linewidth": "The line is always drawn at the default width.",
                "editable": "The line cannot be edited from the chart.",
            ],
            "bgcolor": colorSeries,
            "barcolor": colorSeries,
            "fill": [
                "fillgaps": "Gaps in the plots are not bridged.",
                "editable": "The fill cannot be edited from the chart.",
                "show_last": "The fill is drawn on every bar, not only the last ones.",
            ],
            "plotcandle": [
                "offset": "The candles are not shifted; each is drawn on the bar that produced it.",
                "editable": "The candles cannot be edited from the chart.",
                "show_last": "The candles are drawn on every bar, not only the last ones.",
            ],
            "box.new": [
                "text_wrap": "Box text is never wrapped.",
                "text_font_family": "Box text uses the default font.",
                "text_formatting": "Box text has no bold or italic formatting.",
            ],
            "request.security": request.merging([
                "calc_bars_count": "The whole history is always requested."
            ]) { first, _ in first },
            "request.security_lower_tf": request,
        ]
        let marker = [
            "text": "The text next to the marker is not drawn.",
            "textcolor": "The text next to the marker is not drawn.",
            "offset": "The markers are not shifted; each is drawn on the bar that produced it.",
            "transp": "Transparency is not applied; use `color.new(color, transparency)` instead.",
            "editable": "The markers cannot be edited from the chart.",
            "show_last": "The markers are drawn on every bar, not only the last ones.",
            "force_overlay": "The markers stay in the pane the script declares.",
        ]
        table["plotshape"] = marker
        table["plotchar"] = marker
        return table
    }()

    /// Whole functions that do nothing, by name prefix.
    static let functionPrefixes: [(prefix: String, explanation: String)] = [
        (
            "strategy.risk.",
            "Risk limits are never enforced, so a drawdown or loss limit does not stop the strategy."
        )
    ]

    /// Argument values that make the engine draw something else. Keyed by function, then argument, then the
    /// Pine constant.
    static let values: [String: [String: [String: String]]] = [
        "plot": [
            "style": ["plot.style_stepline_diamond": "It is drawn as a plain line."]
        ]
    ]

    /// Shown for every variable `PineSymbolCatalog.isUnimplementedVariable` lists.
    static let unimplementedVariable = "It is not modelled here, so it is always `na`."

    private static let declaration: [String: String] = [
        "explicit_plot_zorder": "Plots and drawings are painted in the app's own order.",
        "scale": "The chart has one value axis, so the scale is not changed.",
    ]

    private static let entry: [String: String] = [
        "oca_name": "Orders are never grouped; each fills on its own.",
        "oca_type": "Orders are never grouped; each fills on its own.",
        "comment": "Order comments are not shown in the trade list.",
        "alert_message": "Order fill alerts are not sent.",
    ]

    private static let close: [String: String] = [
        "comment": "Order comments are not shown in the trade list.",
        "alert_message": "Order fill alerts are not sent.",
        "immediately": "The position closes on the next bar's open, as with any market order.",
    ]

    private static let colorSeries: [String: String] = [
        "offset": "The colors are not shifted; each applies to the bar that produced it.",
        "title": "The title is not listed anywhere.",
        "editable": "The color cannot be edited from the chart.",
        "show_last": "The colors are applied to every bar, not only the last ones.",
        "force_overlay": "The colors apply to the pane the script declares.",
    ]

    private static let request: [String: String] = [
        "currency": "Prices are not converted to another currency."
    ]
}
