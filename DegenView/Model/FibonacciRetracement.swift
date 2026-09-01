import Foundation

enum DrawingLineStyle: String, Codable, CaseIterable, Hashable {
    case solid
    case dashed
    case dotted

    var title: String { rawValue.capitalized }
}

enum FibonacciLabelFormat: String, Codable, CaseIterable, Hashable {
    case absolute
    case percentage

    var title: String { rawValue.capitalized }
}

enum FibonacciLabelPosition: String, Codable, CaseIterable, Hashable {
    case left
    case right

    var title: String { rawValue.capitalized }
}

enum DrawingTimeframeVisibility: String, Codable, CaseIterable, Hashable {
    case all
    case intraday
    case daily
    case weeklyAndMonthly

    var title: String {
        switch self {
        case .all: return "All Timeframes"
        case .intraday: return "Intraday"
        case .daily: return "Daily"
        case .weeklyAndMonthly: return "Weekly / Monthly"
        }
    }

    func includes(_ range: TimeRange) -> Bool {
        switch self {
        case .all: return true
        case .intraday: return range == .oneHour
        case .daily: return range == .oneDay || range == .threeMonths
        case .weeklyAndMonthly: return range == .oneWeek || range == .oneMonth || range == .oneYear
        }
    }
}

struct FibonacciLevel: Codable, Equatable, Hashable, Identifiable {
    var id = UUID()
    var ratio: Decimal
    var isVisible = true
    var color: TrendLineColor = .blue
    var opacity = 1.0
    var customText = ""
}

struct FibonacciStyle: Codable, Equatable, Hashable {
    var showTrendLine = true
    var trendLineColor: TrendLineColor = .gray
    var trendLineOpacity = 0.8
    var trendLineThickness: TrendLineThickness = .thin
    var trendLineStyle: DrawingLineStyle = .dashed
    var levelLineThickness: TrendLineThickness = .thin
    var levelLineStyle: DrawingLineStyle = .solid
    var extendLeft = false
    var extendRight = false
    var backgroundVisible = true
    var backgroundOpacity = 0.08
    var reverse = false
    var showPrices = false
    var showLevels = true
    var labelFormat: FibonacciLabelFormat = .absolute
    var labelPosition: FibonacciLabelPosition = .right
    var showCustomText = true
    var textPosition: FibonacciLabelPosition = .left
    var fontSize = 10.0
    var useOneColor = false
    var oneColor: TrendLineColor = .blue
    var useLogCalculation = false
}

enum FibonacciDefaults {
    static let maximumLevelCount = 24

    /// A useful, deliberately conservative TradingView-like set. TradingView's help
    /// page documents the common ratios but not the complete current preset.
    static let levels: [FibonacciLevel] = [
        FibonacciLevel(ratio: 0, color: .gray),
        FibonacciLevel(ratio: Decimal(string: "0.236")!, color: .red),
        FibonacciLevel(ratio: Decimal(string: "0.382")!, color: .orange),
        FibonacciLevel(ratio: Decimal(string: "0.5")!, color: .green),
        FibonacciLevel(ratio: Decimal(string: "0.618")!, color: .blue),
        FibonacciLevel(ratio: Decimal(string: "0.786")!, color: .purple),
        FibonacciLevel(ratio: 1, color: .gray),
    ]
}

struct FibonacciRetracementDrawing: Codable, Equatable, Hashable, Identifiable {
    static let schemaVersion = 1

    var schemaVersion = Self.schemaVersion
    var id = UUID()
    var point1: TrendAnchor
    var point2: TrendAnchor
    var levels = FibonacciDefaults.levels
    var style = FibonacciStyle()
    var timeframeVisibility: DrawingTimeframeVisibility = .all
    var isLocked = false
    var isHidden = false
    var createdAt = Date()
    var updatedAt = Date()

    mutating func addLevel(_ level: FibonacciLevel) -> Bool {
        guard levels.count < FibonacciDefaults.maximumLevelCount else { return false }
        levels.append(level)
        updatedAt = Date()
        return true
    }
}

enum FibonacciCalculationMode: Equatable {
    case linear
    case logarithmic
}

enum FibonacciCalculationError: Error, Equatable {
    case nonPositiveLogAnchor
    case nonFiniteResult
}

enum FibonacciCalculator {
    /// Canonical orientation: ratio 0 maps to point 1 and ratio 1 maps to point 2.
    /// Reverse reflects the mapping without mutating either stored anchor.
    static func price(
        point1: Double,
        point2: Double,
        ratio: Decimal,
        reverse: Bool,
        mode: FibonacciCalculationMode
    ) throws -> Double {
        let orientedRatio = reverse ? 1 - ratio : ratio
        let r = NSDecimalNumber(decimal: orientedRatio).doubleValue
        let result: Double
        switch mode {
        case .linear:
            result = point1 + r * (point2 - point1)
        case .logarithmic:
            guard point1 > 0, point2 > 0 else { throw FibonacciCalculationError.nonPositiveLogAnchor }
            result = exp(log(point1) + r * (log(point2) - log(point1)))
        }
        guard result.isFinite else { throw FibonacciCalculationError.nonFiniteResult }
        return result
    }

    static func prices(for drawing: FibonacciRetracementDrawing) -> [(FibonacciLevel, Double)] {
        let mode: FibonacciCalculationMode = drawing.style.useLogCalculation ? .logarithmic : .linear
        return drawing.levels.compactMap { level in
            guard
                let price = try? price(
                    point1: drawing.point1.price,
                    point2: drawing.point2.price,
                    ratio: level.ratio,
                    reverse: drawing.style.reverse,
                    mode: mode
                )
            else { return nil }
            return (level, price)
        }
    }
}

struct FibonacciTemplate: Codable, Equatable, Hashable, Identifiable {
    var id = UUID()
    var name: String
    var levels: [FibonacciLevel]
    var style: FibonacciStyle
}
