import Foundation

/// Values known when an alert triggers. Anything the trigger did not carry stays nil, and its
/// placeholder is left in the text instead of being invented.
struct AlertMessageContext: Equatable, Sendable {
    var ticker: String?
    var exchange: String?
    /// TradingView-style: `1`, `60`, `D`. See `AlertMessageRenderer.tradingViewInterval(_:)`.
    var interval: String?
    var open: Double?
    var high: Double?
    var low: Double?
    var close: Double?
    var volume: Double?
    /// Bar open time.
    var time: Date?
    /// When the message is rendered.
    var now: Date

    init(
        ticker: String? = nil, exchange: String? = nil, interval: String? = nil,
        open: Double? = nil, high: Double? = nil, low: Double? = nil, close: Double? = nil,
        volume: Double? = nil, time: Date? = nil, now: Date = Date()
    ) {
        self.ticker = ticker
        self.exchange = exchange
        self.interval = interval
        self.open = open
        self.high = high
        self.low = low
        self.close = close
        self.volume = volume
        self.time = time
        self.now = now
    }
}

/// Expands TradingView-style `{{placeholder}}` tokens.
///
/// One left-to-right pass over exact `{{name}}` tokens: substituted values are never scanned again,
/// an unknown name or an unclosed `{{` stays as written, and text around a token is untouched.
/// Rendering finishes before the Content-Type is decided, so a template that only becomes JSON once
/// its values are in still goes out as JSON.
struct AlertMessageRenderer {
    /// Every placeholder the renderer can resolve; documentation and the editors read this.
    /// What a price alert with a webhook but no message of its own posts.
    static let defaultPriceAlertTemplate = "{{ticker}} alert: {{close}}"

    static let supportedPlaceholders = [
        "ticker", "exchange", "open", "high", "low", "close", "volume", "time", "timenow", "interval",
    ]

    static func render(_ template: String, context: AlertMessageContext) -> String {
        let scalars = Array(template.unicodeScalars)
        var output = String.UnicodeScalarView()
        var index = 0
        while index < scalars.count {
            if scalars[index] == "{", index + 1 < scalars.count, scalars[index + 1] == "{",
                let close = closingBraces(in: scalars, from: index + 2)
            {
                let name = String(String.UnicodeScalarView(scalars[(index + 2)..<close]))
                if let value = value(for: name, context: context) {
                    output.append(contentsOf: value.unicodeScalars)
                    index = close + 2
                    continue
                }
            }
            output.append(scalars[index])
            index += 1
        }
        return String(output)
    }

    private static func closingBraces(in scalars: [Unicode.Scalar], from start: Int) -> Int? {
        var index = start
        while index + 1 < scalars.count {
            if scalars[index] == "}", scalars[index + 1] == "}" { return index }
            index += 1
        }
        return nil
    }

    private static func value(for name: String, context: AlertMessageContext) -> String? {
        switch name {
        case "ticker": context.ticker
        case "exchange": context.exchange
        case "interval": context.interval
        case "open": context.open.map(number)
        case "high": context.high.map(number)
        case "low": context.low.map(number)
        case "close": context.close.map(number)
        case "volume": context.volume.map(number)
        case "time": context.time.map(timestamp)
        case "timenow": timestamp(context.now)
        default: nil
        }
    }

    /// Plain decimal digits, no grouping, no exponent: `100000.5`, `0.00000278`.
    static func number(_ value: Double) -> String {
        guard value.isFinite else { return "" }
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.numberStyle = .decimal
        formatter.usesGroupingSeparator = false
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = 12
        return formatter.string(from: NSNumber(value: value)) ?? String(value)
    }

    /// ISO 8601 in UTC, like TradingView's `{{time}}`: `2024-05-01T13:45:00Z`.
    static func timestamp(_ date: Date) -> String {
        date.formatted(Date.ISO8601FormatStyle())
    }

    /// Maps DegenView's interval names (`1m`, `1h`, `1d`, `1w`, `1M`) to TradingView's
    /// (`1`, `60`, `D`, `W`, `M`). Unknown names pass through unchanged.
    static func tradingViewInterval(_ interval: String) -> String {
        switch interval {
        case "1d": return "D"
        case "1w": return "W"
        case "1M": return "M"
        case "3M": return "3M"
        case "1Y": return "12M"
        default: break
        }
        if interval.hasSuffix("m"), let minutes = Int(interval.dropLast()) { return String(minutes) }
        if interval.hasSuffix("h"), let hours = Int(interval.dropLast()) { return String(hours * 60) }
        if interval.hasSuffix("d"), let days = Int(interval.dropLast()) { return "\(days)D" }
        return interval
    }
}
