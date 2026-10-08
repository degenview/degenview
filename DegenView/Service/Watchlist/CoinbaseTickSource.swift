import Foundation

/// The slice of the Coinbase socket the watchlist uses, so tests can drive ticks by hand.
protocol CoinbaseTickSource: AnyObject {
    func connect(products: [String], onTick: @escaping (CoinbaseTick) -> Void)
    func disconnect()
}

extension CoinbaseWebSocketService: CoinbaseTickSource {}
