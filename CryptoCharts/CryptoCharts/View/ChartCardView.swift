import SwiftUI

struct ChartCardView: View {
    @ObservedObject var viewModel: ChartViewModel
    var chartHeight: CGFloat = 220
    let onRemove: () -> Void
    let onRetry: () -> Void
    let onZoom: (CGFloat) -> Void
    let onUpdateTicker: (String, DataSourceType) -> Void
    let onStyleChanged: () -> Void

    @State private var showSettings = false
    @State private var iconURL: URL?

    var body: some View {
        VStack(spacing: 2) {
            headerView
            chartArea
        }
        .padding(6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .task {
            iconURL = await CoinGeckoService.shared.iconURL(for: viewModel.baseSymbol)
        }
        .sheet(isPresented: $showSettings) {
            ChartSettingsSheet(
                viewModel: viewModel,
                onUpdateTicker: onUpdateTicker,
                onRemove: onRemove,
                onStyleChanged: onStyleChanged
            )
        }
    }

    // MARK: - Header

    private var headerView: some View {
        HStack(alignment: .firstTextBaseline) {
            Button {
                showSettings = true
            } label: {
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
                        Image(systemName: "gearshape.fill")
                            .font(.caption2)
                            .foregroundStyle(.secondary.opacity(0.6))
                    }

                    if let price = viewModel.currentPrice {
                        Text(price, format: .currency(code: "USD").precision(.fractionLength(2...8)))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .buttonStyle(.plain)

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
        }
    }

    // MARK: - Chart Area

    @ViewBuilder
    private var chartArea: some View {
        if viewModel.isLoading && viewModel.klineData.isEmpty {
            ProgressView()
                .frame(height: max(ChartLayout.chartMinHeight, chartHeight))
        } else if let error = viewModel.errorMessage, viewModel.klineData.isEmpty {
            errorView(message: error)
        } else {
            CandleChartView(
                candles: viewModel.klineData,
                chartHeight: chartHeight,
                bullishColor: viewModel.bullishColor,
                bearishColor: viewModel.bearishColor,
                yAxisDecimalPlaces: viewModel.yAxisDecimalPlaces
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
        .frame(height: ChartLayout.defaultChartHeight)
    }
}

#Preview {
    let vm = ChartViewModel(ticker: "BTC")
    vm.klineData = MockData.sampleKlines
    vm.currentPrice = 68432.15

    return ChartCardView(
        viewModel: vm,
        onRemove: {},
        onRetry: {},
        onZoom: { _ in },
        onUpdateTicker: { _, _ in },
        onStyleChanged: {}
    )
    .frame(width: 400)
    .padding()
}
