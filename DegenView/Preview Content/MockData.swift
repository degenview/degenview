import Foundation

/// Sample KlineData for SwiftUI previews — simulates a 24-hour period with a gentle uptrend.
enum MockData {
    static let sampleKlines: [KlineData] = {
        let now = Date()
        let dayAgo = now.addingTimeInterval(-86400)
        let count = 96  // 15-minute intervals over 24 hours
        var result: [KlineData] = []

        let basePrice: Double = 67000
        let amplitude: Double = 2000

        for i in 0..<count {
            let fraction = Double(i) / Double(count - 1)
            let time = dayAgo.addingTimeInterval(fraction * 86400)
            let trend = fraction * 1500  // gradual uptrend
            let noise = Double.random(in: -amplitude...amplitude)
            let close = basePrice + trend + noise

            // Reconstruct a minimal raw array to exercise the real initializer
            let raw: [Any] = [
                Int64(time.timeIntervalSince1970 * 1000),  // 0: openTime
                String(format: "%.2f", close - Double.random(in: 0...200)),  // 1: open
                String(format: "%.2f", close + Double.random(in: 0...300)),  // 2: high
                String(format: "%.2f", close - Double.random(in: 0...300)),  // 3: low
                String(format: "%.2f", close),  // 4: close
                String(format: "%.2f", Double.random(in: 100...5000)),  // 5: volume
            ]
            if let kline = KlineData(raw: raw) {
                result.append(kline)
            }
        }
        return result
    }()
}
