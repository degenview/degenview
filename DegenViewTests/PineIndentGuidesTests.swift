import XCTest

@testable import DegenView

final class PineIndentGuidesTests: XCTestCase {
    private func levels(_ source: String) -> [Int] {
        let ns = source as NSString
        return PineIndentGuides.lines(in: ns, covering: NSRange(location: 0, length: ns.length)).map(\.level)
    }

    func testLevelsFollowIndentationSteps() {
        XCTAssertEqual(levels("if a\n    x\n    if b\n        y\nz"), [0, 1, 1, 2, 0])
    }

    func testTabCountsAsOneStep() {
        XCTAssertEqual(levels("if a\n\tx"), [0, 1])
    }

    func testBlankLineInsideABlockKeepsTheGuide() {
        XCTAssertEqual(levels("if a\n    x\n\n    y\nz"), [0, 1, 1, 1, 0])
    }

    func testBlankLineAfterABlockEndsTheGuide() {
        XCTAssertEqual(levels("if a\n    x\n\nz"), [0, 1, 0, 0])
    }

    func testTrailingEmptyLineIsIncluded() {
        XCTAssertEqual(levels("if a\n    x\n"), [0, 1, 0])
    }

    func testOnlyRequestedLinesAreReturnedButNeighboursSettleBlankLevels() {
        let source = "if a\n    x\n\n    y\nz" as NSString
        let blank = NSRange(location: source.range(of: "\n\n").location + 1, length: 0)
        let lines = PineIndentGuides.lines(in: source, covering: blank)
        XCTAssertEqual(lines.map(\.level), [1])
        XCTAssertTrue(lines[0].isBlank)
    }

    func testMultibyteLinesKeepRangesInUTF16() {
        let source = "// 🚀\n    x" as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        XCTAssertEqual(lines[1].range, NSRange(location: 6, length: 5))
    }

    func testActiveGuideForTheCaretLineInsideABlock() {
        let source = "if a\n    x\n    y\nz" as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        let active = PineIndentGuides.activeGuide(in: lines, caretLine: 2)
        XCTAssertEqual(active?.column, 0)
        XCTAssertEqual(active?.lines, 1...2)
    }

    func testActiveGuideForABlockHeaderIsItsBody() {
        let source = "if a\n    x\n    y\nz" as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        let active = PineIndentGuides.activeGuide(in: lines, caretLine: 0)
        XCTAssertEqual(active?.column, 0)
        XCTAssertEqual(active?.lines, 1...2)
    }

    func testNoActiveGuideAtTopLevel() {
        let source = "a\nb" as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        XCTAssertNil(PineIndentGuides.activeGuide(in: lines, caretLine: 1))
    }

    func testActiveGuideIsTheInnermostLevelOfTheCaretLine() {
        let source = "if a\n    if b\n        y\n        z\n    w" as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        let active = PineIndentGuides.activeGuide(in: lines, caretLine: 3)
        XCTAssertEqual(active?.column, 4)
        XCTAssertEqual(active?.lines, 2...3)
    }

    // MARK: - Indentation that is not a multiple of four

    private static let wrappedCall = """
        tpMode = input.string(
            "Static",
            "TP Mode",
             options=[
                 "Off",
                 "Static"
             ],
            group=gTP
        )
        """

    func testGuideSitsAtTheOpenerLinesRealColumn() {
        let source = Self.wrappedCall as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        // Lines: 0 tpMode, 1 "Static", 2 "TP Mode", 3 options=[ (5 spaces), 4 "Off", 5 "Static", 6 ], 7 group, 8 )
        XCTAssertEqual(lines[3].guideColumns, [0])
        XCTAssertEqual(lines[4].guideColumns, [0, 5])
        XCTAssertEqual(lines[5].guideColumns, [0, 5])
        XCTAssertEqual(lines[6].guideColumns, [0])
    }

    func testActiveGuideForAnOffGridBlockIsDrawnAtItsOwnColumn() {
        let source = Self.wrappedCall as NSString
        let lines = PineIndentGuides.lines(in: source, covering: NSRange(location: 0, length: source.length))
        // Caret after `[` on the options line, and on a body line: both emphasize column 5.
        for caretLine in [3, 4] {
            let active = PineIndentGuides.activeGuide(in: lines, caretLine: caretLine)
            XCTAssertEqual(active?.column, 5)
            XCTAssertEqual(active?.lines, 4...5)
        }
    }
}
