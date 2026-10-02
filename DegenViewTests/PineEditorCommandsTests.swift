import XCTest

@testable import DegenView

final class PineEditorCommandsTests: XCTestCase {
    private func run(
        _ before: String, _ command: (PineEditorContext) -> PineEditorEdit?
    ) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(command(fixture.context))
    }

    private func toggle(_ before: String) -> String? {
        run(before) { PineEditorCommands.toggleComment(in: $0) }
    }

    // MARK: - Comments

    func testToggleCommentsAndUncommentsOneLine() {
        XCTAssertEqual(toggle("plot(close)|"), "// plot(close)|")
        XCTAssertEqual(toggle("// plot(close)|"), "plot(close)|")
    }

    func testCommentKeepsIndentationBeforeTheMarker() {
        XCTAssertEqual(toggle("    plot(close)|"), "    // plot(close)|")
        XCTAssertEqual(toggle("    // plot(close)|"), "    plot(close)|")
    }

    func testUncommentWithoutSpaceAfterMarker() {
        XCTAssertEqual(toggle("//plot(close)|"), "plot(close)|")
    }

    func testMultilineSelectionTogglesEveryLine() {
        XCTAssertEqual(toggle("⟦a = 1\nb = 2⟧"), "⟦// a = 1\n// b = 2⟧")
        XCTAssertEqual(toggle("⟦// a = 1\n// b = 2⟧"), "⟦a = 1\nb = 2⟧")
    }

    func testDifferentlyIndentedLinesCommentAtTheShallowestColumn() {
        XCTAssertEqual(
            toggle("⟦if a\n    plot(b)⟧"), "⟦// if a\n//     plot(b)⟧")
        XCTAssertEqual(
            toggle("    ⟦x\n        y⟧"), "    ⟦// x\n    //     y⟧")
    }

    func testMixedSelectionCommentsEverythingFirst() {
        XCTAssertEqual(toggle("⟦// a\nb⟧"), "⟦// // a\n// b⟧")
    }

    func testBlankLinesAreSkipped() {
        XCTAssertEqual(toggle("⟦a\n\nb⟧"), "⟦// a\n\n// b⟧")
        XCTAssertEqual(toggle("⟦// a\n\n// b⟧"), "⟦a\n\nb⟧")
    }

    func testToggleAroundMultibyteText() {
        XCTAssertEqual(toggle("m = \"価格\"|"), "// m = \"価格\"|")
        XCTAssertEqual(toggle("// 🚀 signal|"), "🚀 signal|")
    }

    // MARK: - Duplicate

    func testDuplicateDownCopiesTheLineAndFollowsIt() {
        XCTAssertEqual(
            run("a|\nb") { PineEditorCommands.duplicate(down: true, in: $0) }, "a\na|\nb")
    }

    func testDuplicateUpKeepsTheSelectionOnTheUpperCopy() {
        XCTAssertEqual(
            run("a\nb|") { PineEditorCommands.duplicate(down: false, in: $0) }, "a\nb|\nb")
    }

    func testDuplicateLastLineWithoutTerminator() {
        XCTAssertEqual(
            run("a\nb|") { PineEditorCommands.duplicate(down: true, in: $0) }, "a\nb\nb|")
    }

    func testDuplicateSelectedLines() {
        XCTAssertEqual(
            run("⟦a\nb⟧\nc") { PineEditorCommands.duplicate(down: true, in: $0) },
            "a\nb\n⟦a\nb⟧\nc")
    }

    // MARK: - Move

    func testMoveDownSwapsWithTheNextLine() {
        XCTAssertEqual(run("a|\nb\nc") { PineEditorCommands.move(down: true, in: $0) }, "b\na|\nc")
    }

    func testMoveUpSwapsWithThePreviousLine() {
        XCTAssertEqual(run("a\nb|\nc") { PineEditorCommands.move(down: false, in: $0) }, "b|\na\nc")
    }

    func testMoveKeepsIndentationAndSelection() {
        XCTAssertEqual(
            run("if a\n    ⟦x\n    y⟧\nz") { PineEditorCommands.move(down: true, in: $0) },
            "if a\nz\n    ⟦x\n    y⟧")
    }

    func testMoveAtTheEdgesDoesNothing() {
        XCTAssertNil(run("a|\nb") { PineEditorCommands.move(down: false, in: $0) })
        XCTAssertNil(run("a\nb|") { PineEditorCommands.move(down: true, in: $0) })
    }

    func testMoveLastLineUpWithoutTerminator() {
        XCTAssertEqual(run("a\nb|") { PineEditorCommands.move(down: false, in: $0) }, "b|\na")
    }

    func testMoveDownOntoUnterminatedLastLine() {
        XCTAssertEqual(run("a|\nb") { PineEditorCommands.move(down: true, in: $0) }, "b\na|")
    }

    func testMoveAroundMultibyteLines() {
        XCTAssertEqual(
            run("// 🚀|\nm = \"価格\"") { PineEditorCommands.move(down: true, in: $0) },
            "m = \"価格\"\n// 🚀|")
    }
}
