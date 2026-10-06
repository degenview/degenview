import Foundation

extension PineSymbolMetadata {
    /// `request.*`, `timeframe.*` functions and `ticker.*`.
    static let requestEntries = """
        request.security(symbol: series string, timeframe: series string, expression: series any, gaps: barmerge_gaps = barmerge.gaps_off, lookahead: barmerge_lookahead = barmerge.lookahead_off, ignore_invalid_symbol: const bool = false, currency: series string = na) -> series any :: Evaluates an expression on another symbol or timeframe.
        request.security_lower_tf(symbol: series string, timeframe: series string, expression: series any, ignore_invalid_symbol: const bool = false, currency: series string = na) -> array<any> :: Evaluates an expression on a lower timeframe, one element per intrabar.
        timeframe.in_seconds(timeframe: simple string = timeframe.period) -> simple int :: Length of a timeframe in seconds.
        timeframe.from_seconds(seconds: simple int) -> simple string :: Timeframe text for a number of seconds.
        timeframe.change(timeframe: series string) -> series bool :: True on the first bar of a higher-timeframe bar.
        ticker.new(prefix: simple string, ticker: simple string, session: simple string = na, adjustment: simple string = na) -> simple string :: Builds a symbol id.
        ticker.modify(tickerid: simple string, session: simple string = na, adjustment: simple string = na) -> simple string :: Changes the session or adjustment of a symbol id.
        ticker.standard(symbol: simple string = syminfo.tickerid) -> simple string :: Standard-chart version of a symbol id.
        ticker.inherit(from_tickerid: simple string, symbol: simple string) -> simple string :: Applies the settings of one symbol id to another.
        """
}
