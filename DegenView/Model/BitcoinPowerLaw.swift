import Foundation

struct BitcoinPowerLawConfig: Codable, Equatable, Hashable {
    static let defaultIntercept = -16.493
    static let defaultExponent = 5.688
    static let defaultLowerMultiplier = pow(10, -0.4)
    static let defaultUpperMultiplier = pow(10, 0.4)

    var intercept: Double = Self.defaultIntercept
    var exponent: Double = Self.defaultExponent
    var lowerMultiplier: Double = Self.defaultLowerMultiplier
    var upperMultiplier: Double = Self.defaultUpperMultiplier

    static let `default` = BitcoinPowerLawConfig()

    var isValid: Bool {
        intercept.isFinite && exponent.isFinite && exponent > 0
            && lowerMultiplier.isFinite && lowerMultiplier > 0
            && upperMultiplier.isFinite && upperMultiplier > 0
    }

    func price(days: Double) -> Double? {
        guard isValid, days > 0 else { return nil }
        let value = pow(10, intercept) * pow(days, exponent)
        return value.isFinite && value > 0 ? value : nil
    }

    func bandPrices(days: Double) -> (lower: Double, upper: Double)? {
        guard let model = price(days: days) else { return nil }
        return (model * lowerMultiplier, model * upperMultiplier)
    }
}

enum BitcoinPowerLawModel {
    static let genesisDate = Date(timeIntervalSince1970: 1_230_940_800)  // 2009-01-03 UTC

    static func daysSinceGenesis(_ date: Date) -> Double {
        date.timeIntervalSince(genesisDate) / 86_400
    }

    static func projectionEnd(from date: Date, calendar: Calendar = utcCalendar) -> Date {
        calendar.date(byAdding: .year, value: 10, to: date) ?? date.addingTimeInterval(315_576_000)
    }

    static var utcCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }
}

struct BitcoinDailyClose: Codable, Equatable, Identifiable {
    var id: Date { date }
    let date: Date
    let close: Double
}

struct BitcoinPowerLawPlot {
    let xRange: ClosedRange<Double>
    let yRange: ClosedRange<Double>

    init(xRange: ClosedRange<Double>, yRange: ClosedRange<Double>) {
        self.xRange = xRange
        self.yRange = yRange
    }

    init?(history: [BitcoinDailyClose], config: BitcoinPowerLawConfig, now: Date = Date()) {
        guard let first = history.first, config.isValid else { return nil }
        let firstDays = BitcoinPowerLawModel.daysSinceGenesis(first.date)
        let end = BitcoinPowerLawModel.projectionEnd(from: now)
        let endDays = BitcoinPowerLawModel.daysSinceGenesis(end)
        guard firstDays > 0, endDays > firstDays else { return nil }

        var logarithmicPrices = history.compactMap { $0.close > 0 ? log10($0.close) : nil }
        if let startBands = config.bandPrices(days: firstDays),
            let endBands = config.bandPrices(days: endDays)
        {
            logarithmicPrices += [startBands.lower, startBands.upper, endBands.lower, endBands.upper].map(log10)
        }
        guard let minimum = logarithmicPrices.min(), let maximum = logarithmicPrices.max() else { return nil }
        let padding = max(0.08, (maximum - minimum) * 0.06)
        xRange = log10(firstDays)...log10(endDays)
        yRange = (minimum - padding)...(maximum + padding)
    }

    func point(days: Double, price: Double) -> CGPoint? {
        guard days > 0, price > 0 else { return nil }
        return CGPoint(
            x: (log10(days) - xRange.lowerBound) / (xRange.upperBound - xRange.lowerBound),
            y: (log10(price) - yRange.lowerBound) / (yRange.upperBound - yRange.lowerBound)
        )
    }

    func value(at point: CGPoint) -> (days: Double, price: Double) {
        let x = xRange.lowerBound + point.x * (xRange.upperBound - xRange.lowerBound)
        let y = yRange.lowerBound + point.y * (yRange.upperBound - yRange.lowerBound)
        return (pow(10, x), pow(10, y))
    }

    func scaled(xZoom: Double, yZoom: Double) -> BitcoinPowerLawPlot {
        let safeXZoom = xZoom.clamped(to: 1...20)
        let safeYZoom = yZoom.clamped(to: 0.25...20)
        let xSpan = (xRange.upperBound - xRange.lowerBound) / safeXZoom
        let yMidpoint = (yRange.lowerBound + yRange.upperBound) / 2
        let yHalfSpan = (yRange.upperBound - yRange.lowerBound) / (2 * safeYZoom)
        return BitcoinPowerLawPlot(
            xRange: (xRange.upperBound - xSpan)...xRange.upperBound,
            yRange: (yMidpoint - yHalfSpan)...(yMidpoint + yHalfSpan)
        )
    }
}
