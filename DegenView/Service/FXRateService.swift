import Foundation

actor FXRateService {
    static let shared = FXRateService()
    private struct Cache: Codable {
        var fetchedAt: Date
        var rates: [String: Decimal]
    }
    private struct Response: Decodable {
        let base: String
        let date: String
        let rates: [String: Decimal]
    }
    private let store = JSONStore<Cache>(filename: "alert_fx_rates.json")
    private var cache: Cache?

    init() { cache = store.load() }

    func rate(from: PortfolioCurrency, to: PortfolioCurrency) async -> Decimal? {
        guard from != .BTC, to != .BTC else { return nil }
        if from == to { return 1 }
        if cache == nil || Date().timeIntervalSince(cache!.fetchedAt) > 86_400 { await refresh() }
        guard let cache, Date().timeIntervalSince(cache.fetchedAt) <= 5 * 86_400 else { return nil }
        let fromUSD = from == .USD ? Decimal(1) : cache.rates[from.rawValue]
        let toUSD = to == .USD ? Decimal(1) : cache.rates[to.rawValue]
        guard let fromUSD, let toUSD, fromUSD != 0 else { return nil }
        return toUSD / fromUSD
    }

    func refresh() async {
        guard let url = URL(string: "https://api.frankfurter.dev/v2/rates?base=USD&quotes=EUR,GBP,JPY,CHF") else {
            return
        }
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            struct Rate: Decodable {
                let quote: String
                let rate: Decimal
            }
            let values = try JSONDecoder().decode([Rate].self, from: data)
            let next = Cache(
                fetchedAt: Date(), rates: Dictionary(uniqueKeysWithValues: values.map { ($0.quote, $0.rate) }))
            cache = next
            store.save(next)
        } catch { /* retain a still-valid disk value across network failures */  }
    }
}
