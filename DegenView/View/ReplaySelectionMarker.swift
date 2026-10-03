import SwiftUI

/// The start-picker's hover marker: a solid line at the candle under the pointer with a date tag on
/// top, and the plot to its right dimmed, since that is the history the replay will hide.
struct ReplaySelectionMarker: View {
    @ObservedObject var viewModel: ChartViewModel
    let date: Date

    var body: some View {
        GeometryReader { geometry in
            let points = viewModel.visibleKlines
            if let index = points.firstIndex(where: { $0.openTime == date }) {
                let plot = viewModel.plot(in: geometry.size)
                let x = plot.x(forIndex: index, slotWidth: plot.slotWidth(forCount: points.count))
                let rect = plot.plotRect
                ZStack(alignment: .topLeading) {
                    Rectangle()
                        .fill(Color.black.opacity(0.14))
                        .frame(width: max(rect.maxX - x, 0), height: rect.height)
                        .offset(x: x, y: rect.minY)
                    Rectangle()
                        .fill(ReplayStyle.accent)
                        .frame(width: 1.5, height: rect.height)
                        .offset(x: x - 0.75, y: rect.minY)
                    tag.position(x: min(max(x, rect.minX + 60), rect.maxX - 60), y: rect.minY + 12)
                }
            }
        }
        .accessibilityHidden(true)
    }

    private var tag: some View {
        Text("Start here · \(ReplayStyle.compactText(date))")
            .font(.system(size: 10.5, weight: .semibold).monospacedDigit())
            .foregroundStyle(.white)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(ReplayStyle.accent, in: Capsule())
            .fixedSize()
    }
}
