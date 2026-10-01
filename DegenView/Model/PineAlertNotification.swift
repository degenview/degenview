import Foundation

/// One Pine alert that passed routing and is ready to reach the user.
struct PineAlertNotification: Codable, Identifiable, Equatable, Sendable {
    var id = UUID()
    var subscriptionID: UUID
    var scriptName: String
    var chartID: UUID
    var symbolKey: String
    var timeframe: String
    /// Open time of the bar the script raised the alert on.
    var barTime: Date
    var message: String
    var frequency: PineAlertFrequency
    var triggeredAt = Date()
    var isConfirmed: Bool

    /// The ticker without its `"<source>:"` prefix.
    var symbol: String {
        symbolKey.split(separator: ":", maxSplits: 1).last.map(String.init) ?? symbolKey
    }

    var title: String { "\(scriptName) · \(symbol) \(timeframe)" }
}
