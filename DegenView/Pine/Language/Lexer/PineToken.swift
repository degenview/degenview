import Foundation

struct PineToken: Equatable, Sendable {
    let kind: PineTokenKind
    let range: PineSourceRange
}
