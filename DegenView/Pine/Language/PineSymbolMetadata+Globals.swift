import Foundation

extension PineSymbolMetadata {
    /// Builtins callable without a namespace, apart from declarations and plots.
    static let globalEntries = """
        na(x: series any) -> series bool :: Tests whether x is na.
        nz(source: series float, replacement: series float = 0) -> series float :: Replaces na with a number.
        max_bars_back(var: series any, num: const int) -> void :: Sets how many bars of history a series needs.
        alert(message: series string, freq: input string = alert.freq_once_per_bar) -> void :: Raises a script alert.
        alertcondition(condition: series bool, title: const string = na, message: const string = na) -> void :: Defines an alert condition.
        timestamp(year: series int, month: series int, day: series int, hour: series int = 0, minute: series int = 0, second: series int = 0) -> simple int :: UNIX time in milliseconds for a calendar date.
        timestamp(timezone: series string, year: series int, month: series int, day: series int, hour: series int = 0, minute: series int = 0, second: series int = 0) -> simple int :: UNIX time in milliseconds for a calendar date in a time zone.
        time(timeframe: series string = timeframe.period, session: series string = na, timezone: series string = na) -> series int :: Bar open time in a timeframe, or na outside the session.
        time_close(timeframe: series string = timeframe.period, session: series string = na, timezone: series string = na) -> series int :: Bar close time in a timeframe, or na outside the session.
        year(time: series int, timezone: series string = na) -> series int :: Calendar year of a UNIX time.
        month(time: series int, timezone: series string = na) -> series int :: Calendar month of a UNIX time.
        dayofmonth(time: series int, timezone: series string = na) -> series int :: Day of the month of a UNIX time.
        dayofweek(time: series int, timezone: series string = na) -> series int :: Day of the week of a UNIX time.
        hour(time: series int, timezone: series string = na) -> series int :: Hour of a UNIX time.
        minute(time: series int, timezone: series string = na) -> series int :: Minute of a UNIX time.
        second(time: series int, timezone: series string = na) -> series int :: Second of a UNIX time.
        int(x: series any) -> series int :: Casts a value to int.
        float(x: series any) -> series float :: Casts a value to float.
        bool(x: series any) -> series bool :: Casts a value to bool.
        string(x: series any) -> series string :: Casts a value to string.
        color(x: series any) -> series color :: Casts a value to color.
        runtime.error(message: series string) -> void :: Stops the script with an error.
        """

    /// `indicator`, `strategy` and `library`: the arguments the compiler accepts.
    static let declarationEntries = """
        indicator(title: const string, shorttitle: const string = na, overlay: const bool = false, format: const string = na, precision: const int = na, scale: const string = na, max_bars_back: const int = na, max_lines_count: const int = 50, max_labels_count: const int = 50, max_boxes_count: const int = 50, max_polylines_count: const int = 50, behind_chart: const bool = true, explicit_plot_zorder: const bool = false, dynamic_requests: const bool = false) -> void :: Declares the script as an indicator.
        strategy(title: const string, shorttitle: const string = na, overlay: const bool = false, format: const string = na, precision: const int = na, scale: const string = na, pyramiding: const int = 0, calc_on_order_fills: const bool = false, calc_on_every_tick: const bool = false, max_bars_back: const int = na, default_qty_type: const string = strategy.fixed, default_qty_value: const float = 1, initial_capital: const float = 1000000, currency: const string = currency.NONE, slippage: const int = 0, commission_type: const string = strategy.commission.percent, commission_value: const float = 0, process_orders_on_close: const bool = false, close_entries_rule: const string = "FIFO", margin_long: const float = 100, margin_short: const float = 100, max_lines_count: const int = 50, max_labels_count: const int = 50, max_boxes_count: const int = 50, max_polylines_count: const int = 50, behind_chart: const bool = true, explicit_plot_zorder: const bool = false, dynamic_requests: const bool = false) -> void :: Declares the script as a strategy.
        library(title: const string, overlay: const bool = false, dynamic_requests: const bool = false) -> void :: Declares the script as a library.
        """

    /// `plot` and the other drawing-on-the-chart calls.
    static let visualEntries = """
        plot(series: series float, title: const string = "", color: series color = na, linewidth: input int = 1, style: input plot_style = plot.style_line, trackprice: input bool = false, histbase: input float = 0, offset: series int = 0, join: input bool = false, editable: const bool = true, show_last: input int = na, display: input plot_display = display.all) -> plot :: Plots a series on the chart.
        hline(price: input float, title: const string = "", color: input color = na, linestyle: input hline_style = hline.style_solid, linewidth: input int = 1, editable: const bool = true, display: input plot_display = display.all) -> hline :: Draws a horizontal line at a fixed price.
        plotshape(series: series bool, title: const string = "", style: input plot_style = shape.xcross, location: input string = location.abovebar, color: series color = na, offset: series int = 0, text: const string = "", textcolor: input color = color.blue, editable: const bool = true, size: const string = size.auto, show_last: input int = na, display: input plot_display = display.all) -> void :: Plots a marker on bars where series is true.
        plotchar(series: series bool, title: const string = "", char: input string = "\u{2022}", location: input string = location.abovebar, color: series color = na, offset: series int = 0, text: const string = "", textcolor: input color = color.blue, editable: const bool = true, size: const string = size.auto, show_last: input int = na, display: input plot_display = display.all) -> void :: Plots a character on bars where series is true.
        plotcandle(open: series float, high: series float, low: series float, close: series float, title: const string = "", color: series color = na, wickcolor: series color = na, editable: const bool = true, show_last: input int = na, bordercolor: series color = na, display: input plot_display = display.all) -> void :: Plots candles from four series.
        bgcolor(color: series color, offset: series int = 0, editable: const bool = true, show_last: input int = na, title: const string = "", display: input plot_display = display.all) -> void :: Colors the chart background.
        barcolor(color: series color, offset: series int = 0, editable: const bool = true, show_last: input int = na, title: const string = "", display: input plot_display = display.all) -> void :: Colors the bars.
        fill(plot1: plot, plot2: plot, color: series color = na, title: const string = "", editable: const bool = true, fillgaps: const bool = false, display: input plot_display = display.all) -> void :: Fills the space between two plots or hlines.
        fill(plot1: plot, plot2: plot, top_value: series float, bottom_value: series float, top_color: series color, bottom_color: series color, title: const string = "", editable: const bool = true, fillgaps: const bool = false, display: input plot_display = display.all) -> void :: Fills the space between two plots with a gradient.
        """
}
