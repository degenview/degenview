import Foundation

// MARK: - Gamma search payload

/// `GET gamma-api.polymarket.com/public-search` — events, each carrying its markets.
struct PolymarketSearchResponse: Decodable {
    let events: [PolymarketEvent]?
}

/// A group of related questions, e.g. "How many Fed rate cuts in 2026?".
struct PolymarketEvent: Decodable {
    let id: String?
    let title: String?
    let slug: String?
    let icon: String?
    let image: String?
    let markets: [PolymarketMarket]?

    /// Event artwork, used when a market carries none of its own.
    var artworkURL: URL? {
        PolymarketMarket.firstURL(icon, image)
    }
}

/// One tradable question with YES/NO outcomes — a single chartable bet.
///
/// Gamma serializes `outcomes`, `outcomePrices` and `clobTokenIds` as JSON *strings*
/// containing arrays, not as arrays. They need a second decode pass.
///
/// `volume`/`liquidity` are deliberately not decoded: Gamma types them as numbers on
/// events and as strings on markets, and search results already arrive ranked.
struct PolymarketMarket: Decodable {
    let id: String?
    let question: String?
    let groupItemTitle: String?
    let slug: String?
    let icon: String?
    let image: String?
    let active: Bool?
    let closed: Bool?
    let outcomes: String?
    let outcomePrices: String?
    let clobTokenIds: String?

    /// CLOB token id for the YES outcome. NO is just `1 − YES`, so it isn't charted.
    var yesTokenID: String? {
        Self.decodeJSONStringArray(clobTokenIds).first
    }

    /// Current YES probability, 0…1.
    var yesPrice: Double? {
        Self.decodeJSONStringArray(outcomePrices).first.flatMap(Double.init)
    }

    /// Short label for this market within its event ("↑ 100,000", "2 cuts"), falling
    /// back to the full question for single-market events.
    var shortTitle: String? {
        if let group = groupItemTitle, !group.isEmpty { return group }
        return question
    }

    var artworkURL: URL? {
        Self.firstURL(icon, image)
    }

    var isTradable: Bool {
        closed != true && yesTokenID != nil
    }

    // MARK: - Parsing helpers

    /// Decode a JSON array that arrived wrapped in a string, e.g. `"[\"Yes\", \"No\"]"`.
    static func decodeJSONStringArray(_ raw: String?) -> [String] {
        guard let raw, let data = raw.data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([String].self, from: data)) ?? []
    }

    /// First non-empty string that parses as a URL.
    static func firstURL(_ candidates: String?...) -> URL? {
        for candidate in candidates {
            if let candidate, !candidate.isEmpty, let url = URL(string: candidate) {
                return url
            }
        }
        return nil
    }
}

// MARK: - CLOB price history payload

/// `GET clob.polymarket.com/prices-history`
struct PolymarketPriceHistory: Decodable {
    let history: [PolymarketPricePoint]?
}

struct PolymarketPricePoint: Decodable {
    /// Unix timestamp, seconds.
    let t: Double
    /// Price as a probability, 0…1.
    let p: Double

    var kline: KlineData {
        KlineData(time: Date(timeIntervalSince1970: t), price: p)
    }
}
