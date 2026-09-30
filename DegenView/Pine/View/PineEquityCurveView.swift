import SwiftUI

/// The strategy's equity over time against its starting capital.
struct PineEquityCurveView: View {
    let equity: [Double]
    let initialCapital: Double

    private static let pointLimit = 400
    private static let height: CGFloat = 64
    private static let inset: CGFloat = 2

    var body: some View {
        Canvas { context, size in
            let values = Self.downsample(equity, to: Self.pointLimit)
            guard values.count > 1 else { return }
            let low = min(values.min() ?? initialCapital, initialCapital)
            let high = max(values.max() ?? initialCapital, initialCapital)
            let span = max(high - low, 1e-9)
            let usable = size.height - 2 * Self.inset
            func y(_ value: Double) -> CGFloat {
                size.height - CGFloat((value - low) / span) * usable - Self.inset
            }

            var baseline = Path()
            baseline.move(to: CGPoint(x: 0, y: y(initialCapital)))
            baseline.addLine(to: CGPoint(x: size.width, y: y(initialCapital)))
            context.stroke(
                baseline, with: .color(.secondary.opacity(0.5)),
                style: StrokeStyle(lineWidth: 1, dash: [3, 3]))

            var line = Path()
            for (i, value) in values.enumerated() {
                let point = CGPoint(x: size.width * CGFloat(i) / CGFloat(values.count - 1), y: y(value))
                if i == 0 { line.move(to: point) } else { line.addLine(to: point) }
            }
            let tint = (values.last ?? initialCapital) >= initialCapital ? PineReportColors.win : PineReportColors.loss
            context.stroke(line, with: .color(tint), lineWidth: 1.5)
        }
        .frame(height: Self.height)
        .overlay(alignment: .topLeading) {
            Text("Equity").font(.caption2).foregroundStyle(.secondary)
        }
    }

    /// Keeps the first and last points and evenly spaced ones between.
    static func downsample(_ values: [Double], to limit: Int) -> [Double] {
        guard values.count > limit, limit > 1 else { return values }
        return (0..<limit).map { values[$0 * (values.count - 1) / (limit - 1)] }
    }
}
