import Foundation

/// Coin names and artwork for the assets a portfolio shows.
///
/// `PortfolioAsset.name` is whatever the search label was (`BTC/USDT`), so a readable name
/// ("Bitcoin") has to be looked up. Both lookups ride on `IconResolver`'s cached market
/// snapshot; a miss leaves the asset's own label in place and the icon on its monogram.
@MainActor
final class PortfolioAssetInfoViewModel: ObservableObject {
    @Published private(set) var resolvedNames: [String: String] = [:]
    @Published private(set) var iconURLs: [String: URL] = [:]

    private let resolver: IconResolver
    private var requested: Set<String> = []

    init(resolver: IconResolver = .shared) {
        self.resolver = resolver
    }

    /// The line under a ticker: a real name when one is known, else the label as stored.
    func subtitle(for asset: PortfolioAsset) -> String {
        Self.subtitle(for: asset, resolvedName: resolvedNames[asset.key])
    }

    func iconURL(for asset: PortfolioAsset) -> URL? { iconURLs[asset.key] }

    /// Stored name (when it says more than the symbol) → name embedded in the symbol → looked-up
    /// coin name → the symbol exactly as stored.
    nonisolated static func subtitle(for asset: PortfolioAsset, resolvedName: String?) -> String {
        if asset.name != asset.symbol, asset.name.caseInsensitiveCompare(asset.displayTicker) != .orderedSame,
            !asset.name.isEmpty
        {
            return asset.name
        }
        return asset.embeddedName ?? resolvedName ?? asset.symbol
    }

    /// Looks up whatever isn't known yet; assets already asked about are not asked again.
    func load(_ assets: [PortfolioAsset]) {
        let fresh = assets.filter { requested.insert($0.key).inserted }
        guard !fresh.isEmpty else { return }
        for asset in fresh {
            Task { [weak self, resolver] in
                async let name = Self.lookUpName(for: asset, using: resolver)
                async let icon = resolver.iconURL(
                    ticker: asset.marketSymbol, source: asset.source, baseSymbol: asset.displayTicker)
                let (resolvedName, iconURL) = await (name, icon)
                guard let self else { return }
                if let resolvedName { self.resolvedNames[asset.key] = resolvedName }
                if let iconURL { self.iconURLs[asset.key] = iconURL }
            }
        }
    }

    /// Only sources whose ticker is a real coin symbol or id. A DEX ticker can collide with an
    /// unrelated, larger coin — a wrong name is worse than none — and equities carry their own.
    nonisolated private static func lookUpName(for asset: PortfolioAsset, using resolver: IconResolver) async
        -> String?
    {
        switch asset.source {
        case .coingecko:
            return await resolver.coinName(forCoinID: asset.marketSymbol)
        case .binance, .coinbase:
            return await resolver.coinName(forSymbol: asset.displayTicker)
        case .dexscreener, .alpaca, .polymarket, .kalshi, .coinMarketCap:
            return nil
        }
    }
}
