import SwiftUI

struct ChartCardView: View {
    @ObservedObject var viewModel: ChartViewModel
    let intervalLabel: String
    var chartHeight: CGFloat = 220
    let onRemove: () -> Void

    @State private var showRemoveConfirmation = false
    @State private var iconURL: URL?

    private var baseSymbol: String {
        switch viewModel.source {
        case .binance:
            let upper = viewModel.ticker.uppercased()
            for quote in ["USDT", "USDC", "BUSD", "USD", "BTC", "ETH", "BNB"] {
                if upper.hasSuffix(quote), upper.count > quote.count {
                    return String(upper.dropLast(quote.count))
                }
            }
            return upper
        case .coingecko, .dexscreener:
            // For non-Binance sources, use ticker as-is for icon lookup
            // Try to extract a clean base symbol
            let parts = viewModel.ticker.components(separatedBy: "/")
            return parts.first?.uppercased() ?? viewModel.ticker.uppercased()
        }
    }

    let onRetry: () -> Void
    let onZoom: (CGFloat) -> Void
    var useLogScale = false

    var body: some View {
        VStack(spacing: 2) {
            headerView
            chartArea
        }
        .padding(6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .alert("Remove \(viewModel.ticker.uppercased())?", isPresented: $showRemoveConfirmation) {
            Button("Remove", role: .destructive, action: onRemove)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This chart will be removed from your view.")
        }
        .task {
            iconURL = await CoinGeckoService.shared.iconURL(for: baseSymbol)
        }
    }

    // MARK: - Header

    private var headerView: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    if let url = iconURL {
                        AsyncImage(url: url) { phase in
                            switch phase {
                            case .success(let image):
                                image.resizable().scaledToFit()
                            default:
                                Color.clear
                            }
                        }
                        .frame(width: 20, height: 20)
                        .clipShape(Circle())
                    }
                    Text(viewModel.ticker.uppercased())
                        .font(.headline)
                        .fontWeight(.bold)
                    Image(systemName: viewModel.source.icon)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(intervalLabel)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 2)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 3))
                }

                if let price = viewModel.currentPrice {
                    Text(price, format: .currency(code: "USD").precision(.fractionLength(2...8)))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if let change = viewModel.priceChangePercent {
                HStack(spacing: 4) {
                    Image(systemName: viewModel.priceChangeIsPositive ? "arrow.up.right" : "arrow.down.right")
                    Text(abs(change), format: .number.precision(.fractionLength(2)))
                        + Text("%")
                }
                .font(.caption.weight(.medium))
                .foregroundStyle(viewModel.priceChangeIsPositive ? .green : .red)
            }

            Button {
                showRemoveConfirmation = true
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 18))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove \(viewModel.ticker)")
        }
    }

    // MARK: - Chart Area

    @ViewBuilder
    private var chartArea: some View {
        if viewModel.isLoading && viewModel.klineData.isEmpty {
            ProgressView()
                .frame(height: max(50, chartHeight))
        } else if let error = viewModel.errorMessage, viewModel.klineData.isEmpty {
            errorView(message: error)
        } else {
            CandleChartView(
                candles: viewModel.klineData,
                chartHeight: chartHeight,
                useLogScale: useLogScale,
                onZoom: onZoom
            )
            .overlay(alignment: .bottom) {
                if let error = viewModel.errorMessage {
                    HStack(spacing: 6) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Text(error)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Button("Retry", action: onRetry)
                            .font(.caption2)
                            .buttonStyle(.plain)
                            .foregroundStyle(.blue)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 4))
                }
            }
        }
    }

    private func errorView(message: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: "wifi.slash")
                .font(.system(size: 32))
                .foregroundStyle(.secondary)
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Retry", action: onRetry)
                .buttonStyle(.bordered)
                .controlSize(.small)
        }
        .frame(height: 220)
    }
}

#Preview {
    let vm = ChartViewModel(ticker: "BTC")
    vm.klineData = MockData.sampleKlines
    vm.currentPrice = 68432.15

    return ChartCardView(
        viewModel: vm,
        intervalLabel: "15m",
        onRemove: {},
        onRetry: {},
        onZoom: { _ in }
    )
    .frame(width: 400)
    .padding()
}
