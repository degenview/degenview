import Foundation

/// A prediction-market source that can list what is being traded most right now.
///
/// Polymarket does (by 24h volume). Kalshi has no such endpoint, so the Add Chart sheet shows
/// its trending list only for sources that conform.
protocol TrendingMarketsDataSource: PredictionMarketDataSource {
    func trendingMarkets(limit: Int) async throws -> [TickerSearchResult]
}
