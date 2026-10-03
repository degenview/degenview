import XCTest

@testable import DegenView

final class PineEditorAnalysisTests: XCTestCase {
    private let source = "fast = ta.ema(close, 12)\nfoo(a) =>\n    a + fast\nplot(foo(1))\n"

    func testTheSameTextIsAnalysedOnce() {
        let cache = PineEditorAnalysisCache()
        let first = cache.analysis(for: source)
        let second = cache.analysis(for: source)
        XCTAssertEqual(cache.buildCount, 1)
        XCTAssertEqual(first.version, second.version)
    }

    func testMovingTheCaretNeverLexesOrIndexesAgain() {
        let cache = PineEditorAnalysisCache()
        let lexes = PineLexicalSnapshot.buildCount
        _ = cache.analysis(for: source)
        let afterFirst = PineLexicalSnapshot.buildCount
        XCTAssertLessThanOrEqual(afterFirst - lexes, 1)
        for caret in 0...(source as NSString).length {
            let analysis = cache.analysis(for: source)
            _ = analysis.index.scope(atOffset: caret, lineIndent: 0)
            _ = PineEditorContext(source: source, selection: NSRange(location: caret, length: 0))
        }
        XCTAssertEqual(cache.buildCount, 1)
        XCTAssertEqual(PineLexicalSnapshot.buildCount, afterFirst, "no further lex for the same text")
    }

    func testAnEditBuildsANewVersion() {
        let cache = PineEditorAnalysisCache()
        let first = cache.analysis(for: source)
        let second = cache.analysis(for: source + "x = 1\n")
        XCTAssertEqual(cache.buildCount, 2)
        XCTAssertNotEqual(first.version, second.version)
        XCTAssertTrue(second.index.declarations.contains { $0.name == "x" })
    }

    func testAnalysisSharesTheEditorsLexicalSnapshot() {
        let cache = PineEditorAnalysisCache()
        let analysis = cache.analysis(for: source)
        let context = PineEditorContext(source: source, selection: NSRange(location: 0, length: 0))
        XCTAssertEqual(analysis.lexical.items, context.snapshot.items)
    }

    func testHighlightingAndTheIndexResolveShadowingTheSameWay() {
        let shadowed = "close = 123\nplot(close)\nplot(open)\n"
        let cache = PineEditorAnalysisCache()
        let analysis = cache.analysis(for: shadowed)
        let spans = PineSyntaxClassifier.classify(shadowed, analysis: cache)
        func category(of word: String, occurrence: Int) -> PineSyntaxCategory? {
            let ranges = (shadowed as NSString).ranges(of: word)
            guard occurrence < ranges.count else { return nil }
            return spans.first { $0.range == ranges[occurrence] }?.category
        }
        // `close` is the script's own variable from the second line on; `open` is still the builtin.
        XCTAssertEqual(category(of: "close", occurrence: 1), .identifier)
        XCTAssertEqual(category(of: "open", occurrence: 0), .builtinVariable)
        let offset = (shadowed as NSString).range(of: "plot(close)").location + 5
        XCTAssertEqual(
            analysis.index.resolve("close", scope: 0, atOffset: offset)?.kind, .variable)
        XCTAssertNil(analysis.index.resolve("open", scope: 0, atOffset: offset))
    }
}

extension NSString {
    fileprivate func ranges(of word: String) -> [NSRange] {
        var result: [NSRange] = []
        var search = NSRange(location: 0, length: length)
        while true {
            let found = range(of: word, options: .literal, range: search)
            guard found.location != NSNotFound else { return result }
            result.append(found)
            search = NSRange(location: NSMaxRange(found), length: length - NSMaxRange(found))
        }
    }
}
