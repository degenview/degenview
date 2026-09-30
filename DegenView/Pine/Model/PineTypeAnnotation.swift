import Foundation

/// What a declaration wrote before its name: `[qualifier] [type] name = …`.
struct PineTypeAnnotation: Sendable, Equatable {
    var type: PineValueType?
    var qualifier: PineQualifier?

    init(type: PineValueType? = nil, qualifier: PineQualifier? = nil) {
        self.type = type
        self.qualifier = qualifier
    }
}
