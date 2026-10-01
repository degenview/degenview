import Foundation

struct PineInputDefinition: Codable, Equatable, Hashable, Sendable, Identifiable {
    var id: String
    var type: PineValueType
    var defaultValue: PineInputValue
    var title: String?
    var tooltip: String?
    var group: String?
    var inline: String?
    var confirm: Bool
    var minValue: Double?
    var maxValue: Double?
    var step: Double?
    var options: [PineInputValue]?
    /// What to show for each of `options`, when it differs from the value (`input.enum` members have titles).
    var optionTitles: [String]? = nil
}
