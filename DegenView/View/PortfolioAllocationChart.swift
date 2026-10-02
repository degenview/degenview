import SwiftUI

/// Allocation donut with a legend. The biggest positions get their own slice; the long tail is
/// folded into "Other" so a hundred-asset portfolio still reads, and colors never repeat.
struct PortfolioAllocationChart: View {
    let holdings: [PortfolioHolding]
    let formatter: PortfolioValueFormatter
    @State private var hoveredIndex: Int?

    private struct Slice: Identifiable {
        let id: String
        let label: String
        let share: Double
        let value: Decimal?
        let color: Color
    }

    private static let palette: [Color] = [.blue, .orange, .green, .purple, .pink, .cyan, .yellow, .indigo]

    private var slices: [Slice] {
        let sorted = holdings.filter { $0.allocation > 0 }.sorted { $0.allocation > $1.allocation }
        let named = sorted.prefix(Self.palette.count)
        var result = named.enumerated().map { index, holding in
            Slice(
                id: holding.id, label: holding.asset.displayTicker, share: holding.allocation.doubleValue,
                value: holding.currentValue, color: Self.palette[index])
        }
        let rest = sorted.dropFirst(Self.palette.count)
        if !rest.isEmpty {
            result.append(
                Slice(
                    id: "other", label: "Other (\(rest.count))", share: rest.map(\.allocation.doubleValue).reduce(0, +),
                    value: rest.compactMap(\.currentValue).reduce(0, +), color: .gray))
        }
        return result
    }

    var body: some View {
        let slices = slices
        if slices.isEmpty {
            ContentUnavailableView(
                "No Allocations", systemImage: "chart.pie",
                description: Text("Priced holdings appear here."))
        } else {
            HStack(spacing: 20) {
                donut(slices)
                    .frame(width: 190, height: 190)
                legend(slices)
            }
            .animation(.easeOut(duration: 0.15), value: hoveredIndex)
        }
    }

    // MARK: - Donut

    private func donut(_ slices: [Slice]) -> some View {
        GeometryReader { geometry in
            Canvas { context, size in draw(context: &context, size: size, slices: slices) }
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    switch phase {
                    case .active(let location): hoveredIndex = index(at: location, in: geometry.size, slices: slices)
                    case .ended: hoveredIndex = nil
                    }
                }
        }
        .overlay { center(slices) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Allocation chart, \(slices.count) segments")
    }

    private func draw(context: inout GraphicsContext, size: CGSize, slices: [Slice]) {
        var start = -90.0
        let radius = min(size.width, size.height) / 2 - 18
        let gap = slices.count > 1 ? 1.5 : 0.0
        for (index, slice) in slices.enumerated() {
            let end = start + slice.share * 360
            var path = Path()
            path.addArc(
                center: CGPoint(x: size.width / 2, y: size.height / 2), radius: radius,
                startAngle: .degrees(start), endAngle: .degrees(max(start, end - gap)), clockwise: false)
            let dimmed = hoveredIndex != nil && hoveredIndex != index
            context.stroke(
                path, with: .color(slice.color.opacity(dimmed ? 0.3 : 1)),
                style: StrokeStyle(lineWidth: hoveredIndex == index ? 28 : 24, lineCap: .butt))
            start = end
        }
    }

    private func index(at location: CGPoint, in size: CGSize, slices: [Slice]) -> Int? {
        let center = CGPoint(x: size.width / 2, y: size.height / 2)
        let radius = min(size.width, size.height) / 2 - 18
        guard abs(hypot(location.x - center.x, location.y - center.y) - radius) <= 18 else { return nil }
        var degrees = atan2(location.y - center.y, location.x - center.x) * 180 / .pi
        if degrees < -90 { degrees += 360 }
        var start = -90.0
        for (index, slice) in slices.enumerated() {
            let end = start + slice.share * 360
            if degrees >= start && degrees < end { return index }
            start = end
        }
        return nil
    }

    @ViewBuilder private func center(_ slices: [Slice]) -> some View {
        VStack(spacing: 2) {
            if let hoveredIndex, slices.indices.contains(hoveredIndex) {
                let slice = slices[hoveredIndex]
                Text(slice.label).font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(formatter.percent(Decimal(slice.share), digits: 1))
                    .font(.title3.weight(.semibold)).monospacedDigit()
                Text(slice.value.map(formatter.money) ?? "—").font(.caption).foregroundStyle(.secondary)
            } else {
                Text("\(holdings.count)").font(.title.weight(.semibold)).monospacedDigit()
                Text(holdings.count == 1 ? "Asset" : "Assets").font(.caption).foregroundStyle(.secondary)
            }
        }
        .frame(width: 110)
        .allowsHitTesting(false)
    }

    // MARK: - Legend

    private func legend(_ slices: [Slice]) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(slices.enumerated()), id: \.element.id) { index, slice in
                    HStack(spacing: 8) {
                        Circle().fill(slice.color).frame(width: 8, height: 8)
                        Text(slice.label).font(.subheadline.weight(.medium)).lineLimit(1)
                        Spacer(minLength: 8)
                        Text(formatter.percent(Decimal(slice.share), digits: 1))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(
                        hoveredIndex == index ? Color.primary.opacity(0.07) : .clear,
                        in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                    )
                    .opacity(hoveredIndex == nil || hoveredIndex == index ? 1 : 0.45)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        if inside {
                            hoveredIndex = index
                        } else if hoveredIndex == index {
                            hoveredIndex = nil
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(slice.label), \(formatter.percent(Decimal(slice.share))) of portfolio")
                }
            }
        }
        .frame(maxWidth: .infinity)
    }
}
