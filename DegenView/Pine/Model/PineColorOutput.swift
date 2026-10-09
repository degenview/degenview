import Foundation

struct PineColorOutput: Sendable, Identifiable {
    let id: Int
    var title: String? = nil
    var colors: [UInt32?]
}
