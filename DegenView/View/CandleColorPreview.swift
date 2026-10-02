import SwiftUI

/// A tiny preview of the chosen colors: two candles (one up, one down), or a rising and a
/// falling line segment for the line charts of prediction markets.
struct CandleColorPreview: View {
    let bullish: Color
    let bearish: Color
    let isLine: Bool

    var body: some View {
        Canvas { context, size in
            if isLine {
                drawLines(context: context, size: size)
            } else {
                drawCandles(context: context, size: size)
            }
        }
        .frame(width: 96, height: 56)
        .accessibilityHidden(true)
    }

    private func drawCandles(context: GraphicsContext, size: CGSize) {
        // x position, wick top/bottom, body top/bottom as fractions of the height.
        let candles: [(x: CGFloat, wick: (CGFloat, CGFloat), body: (CGFloat, CGFloat), color: Color)] = [
            (0.24, (0.05, 0.85), (0.22, 0.68), bullish),
            (0.52, (0.12, 0.92), (0.28, 0.72), bearish),
            (0.80, (0.02, 0.78), (0.14, 0.58), bullish),
        ]
        let width: CGFloat = 14
        for candle in candles {
            let x = candle.x * size.width
            var wick = Path()
            wick.move(to: CGPoint(x: x, y: candle.wick.0 * size.height))
            wick.addLine(to: CGPoint(x: x, y: candle.wick.1 * size.height))
            context.stroke(wick, with: .color(candle.color), lineWidth: 1.5)
            let body = CGRect(
                x: x - width / 2, y: candle.body.0 * size.height, width: width,
                height: (candle.body.1 - candle.body.0) * size.height)
            context.fill(Path(roundedRect: body, cornerRadius: 2), with: .color(candle.color))
        }
    }

    private func drawLines(context: GraphicsContext, size: CGSize) {
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x * size.width, y: y * size.height) }
        var up = Path()
        up.move(to: point(0.04, 0.8))
        up.addLine(to: point(0.2, 0.6))
        up.addLine(to: point(0.34, 0.68))
        up.addLine(to: point(0.5, 0.28))
        var down = Path()
        down.move(to: point(0.5, 0.28))
        down.addLine(to: point(0.66, 0.42))
        down.addLine(to: point(0.8, 0.3))
        down.addLine(to: point(0.96, 0.82))
        let style = StrokeStyle(lineWidth: 2.25, lineCap: .round, lineJoin: .round)
        context.stroke(up, with: .color(bullish), style: style)
        context.stroke(down, with: .color(bearish), style: style)
    }
}
