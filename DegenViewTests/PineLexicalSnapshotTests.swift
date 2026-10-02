import XCTest

@testable import DegenView

final class PineLexicalSnapshotTests: XCTestCase {
    private func text(_ source: String, _ range: NSRange) -> String {
        (source as NSString).substring(with: range)
    }

    func testStringsOfEitherQuote() {
        let source = "a = \"x(\"\nb = 'y)'\n"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertEqual(snapshot.strings.map { text(source, $0.range) }, ["\"x(\"", "'y)'"])
        XCTAssertTrue(snapshot.strings.allSatisfy(\.isTerminated))
        XCTAssertTrue(snapshot.delimiters.isEmpty)
    }

    func testUnterminatedString() {
        let snapshot = PineLexicalSnapshot(source: "a = \"abc")
        XCTAssertEqual(snapshot.strings.count, 1)
        XCTAssertFalse(snapshot.strings[0].isTerminated)
    }

    func testEscapedQuoteDoesNotTerminate() {
        let snapshot = PineLexicalSnapshot(source: #"a = "x\""#)
        XCTAssertFalse(snapshot.strings[0].isTerminated)
    }

    func testCommentsAreFoundOutsideStringsOnly() {
        let source = "a = \"//no\" // yes ( \nb = 1\n// last"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertEqual(snapshot.comments.map { text(source, $0) }, ["// yes ( ", "// last"])
        XCTAssertTrue(snapshot.delimiters.isEmpty)
    }

    func testDelimitersPairAcrossNesting() {
        let source = "foo(bar(baz()), qux())"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertEqual(snapshot.delimiters.count, 8)
        // foo( ... qux() )
        XCTAssertEqual(snapshot.delimiters[snapshot.partners[0]].location, 21)
        XCTAssertEqual(snapshot.depths[1], 1)
    }

    func testMismatchedKindsStayUnmatched() {
        let snapshot = PineLexicalSnapshot(source: "a = (1]")
        XCTAssertEqual(snapshot.partners, [-1, -1])
    }

    func testOffsetsAreUTF16WithEmojiAndCJK() {
        let source = "// 🚀 signal\nmessage = \"価格\"\nplot(close)"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertEqual(text(source, snapshot.comments[0]), "// 🚀 signal")
        XCTAssertEqual(text(source, snapshot.strings[0].range), "\"価格\"")
        let open = snapshot.delimiters[0]
        XCTAssertEqual(text(source, open.range), "(")
        XCTAssertEqual((source as NSString).substring(from: open.location - 4), "plot(close)")
    }

    func testCRLFOffsetsMapBackToTheEditorText() {
        let source = "a = (1)\r\nb = 'x'\r\n"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertEqual(text(source, snapshot.strings[0].range), "'x'")
        XCTAssertEqual(text(source, snapshot.delimiters[1].range), ")")
    }

    func testStringAndCommentLookups() {
        let source = "x = \"ab\" // c"
        let snapshot = PineLexicalSnapshot(source: source)
        XCTAssertNotNil(snapshot.stringLiteral(containing: 5))
        XCTAssertNil(snapshot.stringLiteral(containing: 9))
        XCTAssertNotNil(snapshot.comment(containing: 11))
        XCTAssertNil(snapshot.comment(containing: 3))
    }

    func testSharedSnapshotIsReusedForSameText() {
        let first = PineLexicalSnapshot.shared(for: "a = 1")
        let second = PineLexicalSnapshot.shared(for: "a = 1")
        XCTAssertEqual(first.items, second.items)
    }
}
