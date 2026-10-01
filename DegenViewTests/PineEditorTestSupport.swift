import Foundation

@testable import DegenView

/// Editor text with its selection written inline: `|` is a caret, `⟦…⟧` a selection.
/// Keeps every editing test a readable before/after pair, and works in UTF-16 throughout.
struct PineEditorFixture {
    let text: String
    let selection: NSRange

    init(_ marked: String) {
        let ns = marked as NSString
        if let open = marked.range(of: "⟦"), let close = marked.range(of: "⟧") {
            let start = ns.range(of: "⟦").location
            let end = ns.range(of: "⟧").location
            selection = NSRange(location: start, length: end - start - 1)
            text = marked.replacingCharacters(in: close, with: "").replacingCharacters(in: open, with: "")
        } else {
            let caret = ns.range(of: "|").location
            precondition(caret != NSNotFound, "fixture needs | or ⟦⟧")
            selection = NSRange(location: caret, length: 0)
            text = ns.replacingCharacters(in: NSRange(location: caret, length: 1), with: "")
        }
    }

    var context: PineEditorContext { PineEditorContext(source: text, selection: selection) }

    static func render(_ text: String, _ selection: NSRange) -> String {
        let ns = text as NSString
        if selection.length == 0 {
            return ns.replacingCharacters(in: NSRange(location: selection.location, length: 0), with: "|")
        }
        let end = ns.replacingCharacters(in: NSRange(location: NSMaxRange(selection), length: 0), with: "⟧")
        return (end as NSString).replacingCharacters(
            in: NSRange(location: selection.location, length: 0), with: "⟦")
    }

    /// The fixture after `edit`, rendered back in the same notation; `nil` for no edit.
    func result(_ edit: PineEditorEdit?) -> String? {
        guard let edit else { return nil }
        let applied = edit.applied(to: text)
        return Self.render(applied.text, applied.selection)
    }
}
