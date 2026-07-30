import Foundation

// MARK: - API models

private struct MarketCoin: Codable {
    let id: String
    let symbol: String
    let image: String        // URL string
}

// MARK: - Cache

private struct IconCache: Codable {
    var iconURLs: [String: String]   // "btc" → "https://..."
    var updatedAt: Date
}

// MARK: - Service

final class CoinGeckoService {
    static let shared = CoinGeckoService()

    private let baseURL = "https://api.coingecko.com/api/v3"
    private let cacheURL: URL
    private var cache: IconCache

    init() {
        cacheURL = AppSupport.directory.appendingPathComponent("icon_cache.json")

        if let data = try? Data(contentsOf: cacheURL),
           let cached = try? JSONDecoder().decode(IconCache.self, from: data) {
            cache = cached
        } else {
            cache = IconCache(iconURLs: [:], updatedAt: .distantPast)
        }
    }

    // MARK: - Public

    func iconURL(for symbol: String) async -> URL? {
        let key = symbol.lowercased()

        if let urlString = cache.iconURLs[key] {
            return URL(string: urlString)
        }

        await refreshCacheIfNeeded()

        if let urlString = cache.iconURLs[key] {
            return URL(string: urlString)
        }

#if DEBUG
        print("[CoinGecko] Unknown symbol: \(symbol)")
#endif
        return nil
    }

    // MARK: - Private

    /// Fetch top 250 coins by market cap. Symbol → image URL.
    /// Highest-cap coin wins per symbol (e.g. "btc" → bitcoin, not batcat).
    private func refreshCacheIfNeeded() async {
        let stale = cache.updatedAt.addingTimeInterval(Timeout.iconCacheStaleness)
        guard Date() > stale else { return }

        let urlString = "\(baseURL)/coins/markets?vs_currency=usd&order=market_cap_desc&per_page=\(Timeout.iconMaxCoins)&sparkline=false"
        guard let url = URL(string: urlString) else { return }

        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            let coins = try JSONDecoder().decode([MarketCoin].self, from: data)

            var map: [String: String] = [:]
            for coin in coins {
                if map[coin.symbol] == nil {
                    map[coin.symbol] = coin.image
                }
            }
            cache.iconURLs = map
            cache.updatedAt = Date()
            saveCache()
        } catch {
#if DEBUG
            print("[CoinGecko] Failed to refresh: \(error.localizedDescription)")
#endif
        }
    }

    private func saveCache() {
        guard let data = try? JSONEncoder().encode(cache) else { return }
        try? data.write(to: cacheURL, options: .atomic)
    }
}
