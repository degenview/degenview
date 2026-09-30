import Foundation

struct PineColorOutput: Sendable, Identifiable {
    let id: Int
    var colors: [UInt32?]
}
