import Foundation

/// Symbol facts a script can read through `syminfo.*`.
struct PineSymbolInfo: Equatable, Sendable {
    var ticker = ""
    var tickerID = ""
    var currency = "USD"
    var type = "crypto"
}
