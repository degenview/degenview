import Foundation

/// Symbol facts a script can read through `syminfo.*`.
struct PineSymbolInfo: Equatable, Sendable {
    var ticker = ""
    var tickerID = ""
    var currency = "USD"
    var type = "crypto"
    /// The asset a pair is priced in terms of (`BTC` of `BTCUSDT`); nil when the symbol is not a pair.
    var baseCurrency: String?

    private static let quotes = [
        "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USD", "EUR", "GBP", "TRY", "JPY", "BRL", "BTC", "ETH", "BNB",
    ]

    /// The base of an exchange pair ticker: `BTC` from `BTC-USD`, `BTC/USD` and `BTCUSDT`. Nil when no
    /// quote currency can be told apart from the name.
    static func baseCurrency(ofPair ticker: String) -> String? {
        let upper = ticker.uppercased()
        if let separator = upper.firstIndex(where: { "-/_".contains($0) }) {
            let base = String(upper[..<separator])
            return base.isEmpty ? nil : base
        }
        for quote in quotes where upper.count > quote.count && upper.hasSuffix(quote) {
            return String(upper.dropLast(quote.count))
        }
        return nil
    }
}
