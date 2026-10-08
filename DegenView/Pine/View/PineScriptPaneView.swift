import SwiftUI

// MARK: - PineScriptPaneView

/// Own pane under the price chart for `overlay=false` scripts (RSI, MACD, …), with a
/// value axis fitted to the script's outputs. Shares the price chart's horizontal
/// layout, so bar slots line up with the candles above; the time axis stays there.
struct PineScriptPaneView: View {
    let pine: PineVisualOutput
    let candles: [KlineData]
    var height: CGFloat
    var style: ChartStyle = .default
    /// The price chart's reserved slots right of the last candle, so bar slots still line up.
    var futureSlots = 0

    var body: some View {
        GeometryReader { geometry in
            let script = PineChartLayer(pine: pine, candles: candles, inPane: true)
            let plot = ChartPlot(
                plotRect: ChartPlot.rect(in: geometry.size, insets: insets),
                priceRange: script.valueRange(padding: style.pricePadding),
                style: style,
                scale: .number,
                yAxisDecimalPlaces: nil,
                futureSlots: futureSlots
            )

            Canvas { context, size in
                var divider = Path()
                divider.move(to: .zero)
                divider.addLine(to: CGPoint(x: size.width, y: 0))
                context.stroke(divider, with: .color(style.gridColor), lineWidth: 1)

                plot.drawGrid(&context)
                context.drawLayer { layer in
                    layer.clip(to: Path(plot.plotRect))
                    script.drawBackground(&layer, plot: plot)
                    script.drawForeground(&layer, plot: plot)
                }
                script.drawTables(&context, plot: plot)
            }
        }
        .frame(height: max(0, height))
        .clipped()
    }

    /// The price chart's side insets, so bar slots line up; top and bottom leave just
    /// enough room for the outermost axis labels.
    private var insets: EdgeInsets {
        var insets = style.chartInsets
        insets.top = 8
        insets.bottom = 8
        return insets
    }
}

#Preview {
    PineScriptPaneView(pine: .empty, candles: MockData.sampleKlines, height: 80)
        .frame(width: 400)
        .padding()
}
