import Foundation

/// A UTF-16 range of the editor text and what it is.
struct PineHighlightSpan: Equatable, Sendable {
    let range: NSRange
    let category: PineSyntaxCategory
}
