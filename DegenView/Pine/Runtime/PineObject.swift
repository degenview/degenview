import Foundation

/// One instance of a script-defined `type`. Fields are dynamic: the declaration only supplies the
/// names and defaults.
struct PineObject: Equatable, Sendable {
    var typeName: String
    var fields: [String: PineRuntimeValue]
}
