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
                    let pnl = unrealizedPnL(position)
                    let size = quantity(position.quantity, position.instrument)
                    let entry = price(position.averageEntryPrice, position.instrument)
                    PaperChartMarkerPill(
                        tag: position.side.badgeText, tint: position.side.tint,
                        text: "\(size) @ \(entry)",
                        pnlText: PaperTradingFormatter.signedMoney(pnl, currency: accountCurrency),
                        pnlColor: PaperTradingStyle.pnl(pnl),
                        closeLabel: "Close paper \(position.side.rawValue) position, \(size)"
                    ) {
                        onClose(position)
                    }
                    .frame(maxWidth: .infinity)
                    .offset(y: y(position.averageEntryPrice, in: geometry.size, range: range))
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
            PaperChartMarkerPill(
                tag: order.role == .entry ? order.type.badgeText : order.role.badgeText, tint: color,
                text: "\(order.side.badgeText) \(quantity(order.remainingQuantity)) @ \(price(displayed))",
                closeLabel: "Cancel paper \(order.side.rawValue) \(order.type.rawValue) order at \(price(displayed))"
            ) {
                onCancel(order)
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
        /// Take profit green, stop loss and plain stops red, resting limits in the accent colour.
        private var color: Color {
            switch order.role {
            case .takeProfit: PaperTradingStyle.buy
            case .stopLoss: PaperTradingStyle.sell
            case .entry: order.type == .stop || order.type == .stopLimit ? .orange : .accentColor
            }
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
