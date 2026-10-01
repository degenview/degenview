import Foundation

/// One member of a script-defined `enum`: `name [= "title"]`. The title is what an input shows
/// and `str.tostring` returns; it defaults to the member's name.
struct PineEnumMember: Sendable, Equatable {
    var name: String
    var title: String
}
