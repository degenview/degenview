import Foundation

/// Partial decode of Binance exchangeInfo response — only the fields we need for validation.
struct ExchangeInfoResponse: Decodable {
    let symbols: [SymbolInfo]
}

struct SymbolInfo: Decodable {
    let symbol: String
    let status: String
    let baseAsset: String
    let quoteAsset: String
}
