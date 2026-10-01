import XCTest

@testable import DegenView

final class PineDelimiterMatcherTests: XCTestCase {
    /// The matched pair for a caret written as `|`, as the text of each delimiter's location.
    private func match(_ marked: String) -> (source: String, match: PineDelimiterMatcher.Match?) {
        let fixture = PineEditorFixture(marked)
        let snapshot = PineLexicalSnapshot(source: fixture.text)
        return (fixture.text, PineDelimiterMatcher.match(at: fixture.selection.location, in: snapshot))
    }

    private func locations(_ marked: String) -> (delimiter: Int, partner: Int?)? {
        guard let match = match(marked).match else { return nil }
        return (match.delimiter.location, match.partner?.location)
    }

    func testCaretBeforeClosingFindsOpener() {
        let result = locations("ta.sma(close, 20|)")
        XCTAssertEqual(result?.delimiter, 16)
        XCTAssertEqual(result?.partner, 6)
    }

    func testCaretBeforeOpeningFindsCloser() {
        let result = locations("ta.sma|(close, 20)")
        XCTAssertEqual(result?.delimiter, 6)
        XCTAssertEqual(result?.partner, 16)
    }

    func testCaretAfterClosingFindsOpener() {
        XCTAssertEqual(locations("f(a)|")?.partner, 1)
    }

    func testNestedExpressionsResolveToTheRightLevel() {
        let source = "math.max(ta.sma(close, 20), ta.ema(close, 50))"
        let ns = source as NSString
        let outerOpen = ns.range(of: "(").location
        let outerClose = ns.length - 1
        let innerOpen = ns.range(of: "(", options: [], range: NSRange(location: outerOpen + 1, length: 20)).location
        let snapshot = PineLexicalSnapshot(source: source)

        let outer = PineDelimiterMatcher.match(at: outerOpen, in: snapshot)
        XCTAssertEqual(outer?.partner?.location, outerClose)
        XCTAssertEqual(outer?.depth, 0)
        let inner = PineDelimiterMatcher.match(at: innerOpen, in: snapshot)
        XCTAssertEqual(inner?.partner?.location, ns.range(of: ")").location)
        XCTAssertEqual(inner?.depth, 1)
    }

    func testSiblingCallsEachResolve() {
        // foo(bar(baz()), qux())
        let source = "foo(bar(baz()), qux())"
        let snapshot = PineLexicalSnapshot(source: source)
        func partner(_ offset: Int) -> Int? {
            PineDelimiterMatcher.match(at: offset, in: snapshot)?.partner?.location
        }
        XCTAssertEqual(partner(3), 21)
        XCTAssertEqual(partner(7), 13)
        XCTAssertEqual(partner(11), 12)
        XCTAssertEqual(partner(19), 20)
    }

    func testDelimitersInStringsAreIgnored() {
        XCTAssertNil(locations("message = \"fake |( parenthesis\""))
        let result = locations("f(\"a ( b\"|)")
        XCTAssertEqual(result?.partner, 1)
    }

    func testDelimitersInCommentsAreIgnored() {
        XCTAssertNil(locations("// this |) is not syntax"))
        let result = locations("f(a // )\n|)")
        XCTAssertEqual(result?.partner, 1)
    }

    func testUnmatchedDelimitersAreReported() {
        XCTAssertTrue(match("ta.sma(close, 20))|").match?.isUnmatched == true)
        XCTAssertTrue(match("foo = |(close + open").match?.isUnmatched == true)
    }

    func testNoMatchAwayFromDelimiters() {
        XCTAssertNil(match("fo|o(a)").match)
    }

    func testMatchAfterMultibyteText() {
        let result = locations("// 🚀\nplot(\"価格\"|)")
        XCTAssertEqual(result?.partner, ("// 🚀\nplot" as NSString).length)
    }

    func testEnclosingOpener() {
        let fixture = PineEditorFixture("f(a, g(b), |c)")
        let snapshot = PineLexicalSnapshot(source: fixture.text)
        XCTAssertEqual(
            PineDelimiterMatcher.enclosingOpener(before: fixture.selection.location, in: snapshot)?.location, 1)
        let inner = PineEditorFixture("f(a, g(|b), c)")
        XCTAssertEqual(
            PineDelimiterMatcher.enclosingOpener(
                before: inner.selection.location, in: PineLexicalSnapshot(source: inner.text))?.location, 6)
        XCTAssertNil(
            PineDelimiterMatcher.enclosingOpener(before: 0, in: PineLexicalSnapshot(source: "f(a)")))
    }
}
