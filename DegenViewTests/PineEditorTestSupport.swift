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

/// A completion request written inline: `|` is the caret. Builds its own analysis cache, so tests
/// never share state, and exposes the engine's answer and the result of accepting a candidate.
struct PineCompletionFixture {
    let editor: PineEditorFixture
    let analysis: PineEditorAnalysisSnapshot
    let context: PineCompletionContext

    init(_ marked: String, explicit: Bool = false) {
        editor = PineEditorFixture(marked)
        analysis = PineEditorAnalysisCache().analysis(for: editor.text)
        context = PineCompletionContext(analysis: analysis, selection: editor.selection, explicit: explicit)
    }

    func items(libraries: PineLibraryExportProviding = PineNoLibraryExports()) -> [PineCompletionItem] {
        PineCompletionEngine.complete(context, analysis: analysis, libraries: libraries)
    }

    func labels(libraries: PineLibraryExportProviding = PineNoLibraryExports()) -> [String] {
        items(libraries: libraries).map(\.label)
    }

    func item(_ label: String, libraries: PineLibraryExportProviding = PineNoLibraryExports()) -> PineCompletionItem? {
        items(libraries: libraries).first { $0.label == label }
    }

    /// The text after accepting the candidate named `label`, in `|` notation; nil when there is none.
    func accepting(_ label: String, libraries: PineLibraryExportProviding = PineNoLibraryExports()) -> String? {
        guard let item = item(label, libraries: libraries),
            let acceptance = PineCompletionInsertion.accept(item, in: editor.context)
        else { return nil }
        return editor.result(acceptance.edit)
    }
}

/// A library set for tests: exports by import path, and the names on offer.
struct StubLibraryExports: PineLibraryExportProviding {
    var exports: [String: [PineLibraryExport]] = [:]

    func exports(forImportPath path: String) -> [PineLibraryExport]? { exports[path] }
    func libraryNames() -> [String] {
        exports.keys.compactMap { $0.split(separator: "/").dropFirst().first.map(String.init) }.sorted()
    }
}
