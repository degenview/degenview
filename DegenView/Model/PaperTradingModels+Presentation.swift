import SwiftUI

// Human-readable labels and tints for the Paper Trading enums. Views never show a raw value
// (`stopLimit`, `partiallyFilled`) — everything user-facing goes through here.

extension PaperInstrument {
    /// The asset being traded: `BTC` for `BTC/USDT`; a name without a pair is used as it is.
    var baseSymbol: String {
        if displayName.contains("/"), let base = displayName.components(separatedBy: "/").first, !base.isEmpty {
            return base.uppercased()
        }
        return symbol.uppercased()
    }
}

extension PaperOrderSide {
    var label: String { self == .buy ? "Buy" : "Sell" }
    var badgeText: String { self == .buy ? "BUY" : "SELL" }
    var tint: Color { self == .buy ? PaperTradingStyle.buy : PaperTradingStyle.sell }
}

extension PaperPositionSide {
    var label: String { self == .long ? "Long" : "Short" }
    var badgeText: String { self == .long ? "LONG" : "SHORT" }
    var tint: Color { self == .long ? PaperTradingStyle.buy : PaperTradingStyle.sell }
}

extension PaperOrderType {
    var label: String {
        switch self {
        case .market: "Market"
        case .limit: "Limit"
        case .stop: "Stop"
        case .stopLimit: "Stop Limit"
        }
    }

    var systemImage: String {
        switch self {
        case .market: "bolt.fill"
        case .limit: "arrow.down.to.line"
        case .stop: "hand.raised.fill"
        case .stopLimit: "hand.raised.square.fill"
        }
    }

    /// Short tag for a chart marker.
    var badgeText: String {
        switch self {
        case .market: "MKT"
        case .limit: "LMT"
        case .stop: "STP"
        case .stopLimit: "STP LMT"
        }
    }
}

extension PaperTimeInForce {
    var label: String { self == .day ? "Day" : "Good till canceled" }
}

extension PaperOrderRole {
    var label: String {
        switch self {
        case .entry: "Entry"
        case .takeProfit: "Take profit"
        case .stopLoss: "Stop loss"
        }
    }

    var badgeText: String {
        switch self {
        case .entry: "ENTRY"
        case .takeProfit: "TP"
        case .stopLoss: "SL"
        }
    }
}

extension PaperOrderStatus {
    var label: String {
        switch self {
        case .pendingSubmission: "Pending"
        case .working: "Working"
        case .partiallyFilled: "Partial"
        case .filled: "Filled"
        case .pendingCancel: "Cancelling"
        case .canceled: "Canceled"
        case .rejected: "Rejected"
        case .expired: "Expired"
        }
    }

    var tone: SettingsStatusBadge.Tone {
        switch self {
        case .filled: .good
        case .partiallyFilled, .pendingCancel: .warning
        case .rejected: .bad
        case .pendingSubmission, .working, .canceled, .expired: .neutral
        }
    }
}

extension Array where Element == PaperClosedTrade {
    var netPnL: Decimal { reduce(0) { $0 + $1.netPnL } }

    /// The share of trades that closed above zero after commission (`0.5` is half), nil with no trades.
    var winRate: Decimal? {
        guard !isEmpty else { return nil }
        return Decimal(filter { $0.netPnL > 0 }.count) / Decimal(count)
    }
}

extension PaperOrderEventKind {
    var label: String {
        switch self {
        case .placed: "Placed"
        case .accepted: "Accepted"
        case .partiallyFilled: "Partial fill"
        case .filled: "Filled"
        case .modified: "Modified"
        case .canceled: "Canceled"
        case .rejected: "Rejected"
        case .expired: "Expired"
        }
    }

    var tone: SettingsStatusBadge.Tone {
        switch self {
        case .filled: .good
        case .partiallyFilled, .modified: .warning
        case .rejected: .bad
        case .placed, .accepted, .canceled, .expired: .neutral
        }
    }
}
