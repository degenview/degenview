import SwiftUI

struct PaperChartTradingOverlay: View {
    let candles: [KlineData]
    let positions: [PaperPosition]
    let orders: [PaperOrder]
    let accountCurrency: PaperCurrency
    let unrealizedPnL: (PaperPosition) -> Decimal
    let onModify: (PaperOrder, Decimal) -> Void
    let onCancel: (PaperOrder) -> Void
    let onClose: (PaperPosition) -> Void

    var body: some View {
        GeometryReader { geometry in
            let range = priceRange
            ZStack(alignment: .topLeading) {
                ForEach(positions) { position in
                    marker(y: y(position.averageEntryPrice, in: geometry.size, range: range), color: .blue) {
                        HStack(spacing: 5) {
                            Text(
                                "PAPER \(position.side.rawValue.uppercased()) \(quantity(position.quantity, position.instrument)) @ \(price(position.averageEntryPrice, position.instrument))  \(PaperTradingFormatter.signedMoney(unrealizedPnL(position), currency: accountCurrency))"
                            )
                            Button {
                                onClose(position)
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                            }
                            .buttonStyle(.plain).accessibilityLabel(
                                "Close paper \(position.side.rawValue) position, \(quantity(position.quantity, position.instrument))"
                            )
                        }
                    }
                }
                ForEach(orders) { order in
                    if let price = markerPrice(order) {
                        DraggableOrderMarker(
                            order: order, initialPrice: price, range: range,
                            size: geometry.size, onModify: onModify, onCancel: onCancel)
                    }
                }
            }
        }
    }

    private var priceRange: ClosedRange<Decimal> {
        let candlePrices = candles.flatMap { [Decimal($0.lowPrice), Decimal($0.highPrice)] }
        let tradingPrices = positions.map(\.averageEntryPrice) + orders.compactMap(markerPrice)
        let values = candlePrices + tradingPrices
        let low = values.min() ?? 0
        let high = values.max() ?? 1
        let padding = max((high - low) * Decimal(string: "0.05")!, Decimal(string: "0.00000001")!)
        return (low - padding)...(high + padding)
    }

    private func markerPrice(_ order: PaperOrder) -> Decimal? {
        switch order.type {
        case .limit: order.limitPrice
        case .stop: order.stopPrice
        case .stopLimit: order.stopTriggered ? order.limitPrice : order.stopPrice
        case .market: nil
        }
    }
    private func y(_ price: Decimal, in size: CGSize, range: ClosedRange<Decimal>) -> CGFloat {
        let fraction = ((price - range.lowerBound) / (range.upperBound - range.lowerBound)).doubleValue
        return max(0, min(size.height - 20, size.height * CGFloat(1 - fraction)))
    }
    private func marker<Content: View>(y: CGFloat, color: Color, @ViewBuilder content: () -> Content) -> some View {
        HStack(spacing: 0) {
            Rectangle().fill(color).frame(height: 1)
            content().font(.caption2.monospacedDigit()).padding(.horizontal, 5).padding(.vertical, 2).background(
                color.opacity(0.9), in: Capsule()
            ).foregroundStyle(.white)
        }
        .frame(maxWidth: .infinity).offset(y: y)
    }
    private func price(_ value: Decimal, _ instrument: PaperInstrument) -> String {
        PaperTradingFormatter.price(value, instrument: instrument)
    }
    private func quantity(_ value: Decimal, _ instrument: PaperInstrument) -> String {
        PaperTradingFormatter.quantity(value, instrument: instrument)
    }

    private struct DraggableOrderMarker: View {
        let order: PaperOrder
        let initialPrice: Decimal
        let range: ClosedRange<Decimal>
        let size: CGSize
        let onModify: (PaperOrder, Decimal) -> Void
        let onCancel: (PaperOrder) -> Void
        @State private var dragY: CGFloat?

        var body: some View {
            let baseY = y(initialPrice)
            let displayed = dragY.map(price) ?? initialPrice
            HStack(spacing: 0) {
                Rectangle().fill(color).frame(height: 1)
                HStack(spacing: 5) {
                    Text(
                        "PAPER \(order.side.rawValue.uppercased()) \(order.type.rawValue.uppercased()) \(quantity(order.remainingQuantity)) @ \(price(displayed))"
                    )
                    Button {
                        onCancel(order)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }.buttonStyle(.plain)
                        .accessibilityLabel(
                            "Cancel paper \(order.side.rawValue) \(order.type.rawValue) order at \(price(displayed))")
                }.font(.caption2.monospacedDigit()).padding(.horizontal, 5).padding(.vertical, 2).background(
                    color.opacity(0.9), in: Capsule()
                ).foregroundStyle(.white)
            }
            .frame(maxWidth: .infinity).offset(y: dragY ?? baseY).contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 2).onChanged { dragY = max(0, min(size.height - 20, $0.location.y)) }
                    .onEnded { value in
                        let snapped = snap(price(max(0, min(size.height - 20, value.location.y))))
                        dragY = nil
                        onModify(order, snapped)
                    }
            )
            .help("Drag to modify; release to commit")
        }
        private var color: Color {
            order.role == .takeProfit ? .green : (order.role == .stopLoss || order.type == .stop ? .red : .orange)
        }
        private func y(_ price: Decimal) -> CGFloat {
            size.height * CGFloat(1 - ((price - range.lowerBound) / (range.upperBound - range.lowerBound)).doubleValue)
        }
        private func price(_ y: CGFloat) -> Decimal {
            range.upperBound - Decimal(Double(y / max(1, size.height))) * (range.upperBound - range.lowerBound)
        }
        private func snap(_ value: Decimal) -> Decimal {
            Decimal.rounded(value / order.instrument.tickSize, scale: 0) * order.instrument.tickSize
        }
        private func price(_ value: Decimal) -> String {
            PaperTradingFormatter.price(value, instrument: order.instrument)
        }
        private func quantity(_ value: Decimal) -> String {
            PaperTradingFormatter.quantity(value, instrument: order.instrument)
        }
    }
}
