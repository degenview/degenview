import XCTest

@testable import DegenView

final class PineIndentationEngineTests: XCTestCase {
    private func newline(_ before: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineIndentationEngine.newline(in: fixture.context))
    }

    private func closing(_ before: String, _ key: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineEditorPairing.typed(key, in: fixture.context))
    }

    private func indent(_ before: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineIndentationEngine.indent(in: fixture.context))
    }

    private func outdent(_ before: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineIndentationEngine.outdent(in: fixture.context))
    }

    private func paste(_ pasted: String, into before: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineIndentationEngine.reindentPaste(pasted, in: fixture.context))
    }

    // MARK: - Blocks

    func testBlockOpenersIndent() {
        XCTAssertEqual(newline("if close > open|"), "if close > open\n    |")
        XCTAssertEqual(newline("else|"), "else\n    |")
        XCTAssertEqual(newline("else if a|"), "else if a\n    |")
        XCTAssertEqual(newline("for i = 0 to 10|"), "for i = 0 to 10\n    |")
        XCTAssertEqual(newline("for x in xs|"), "for x in xs\n    |")
        XCTAssertEqual(newline("while a < b|"), "while a < b\n    |")
        XCTAssertEqual(newline("switch kind|"), "switch kind\n    |")
        XCTAssertEqual(newline("x = if a|"), "x = if a\n    |")
        XCTAssertEqual(newline("x = switch k|"), "x = switch k\n    |")
    }

    func testFunctionsAndTypesIndent() {
        XCTAssertEqual(newline("double(x) =>|"), "double(x) =>\n    |")
        XCTAssertEqual(newline("method m(this) =>|"), "method m(this) =>\n    |")
        XCTAssertEqual(newline("type Point|"), "type Point\n    |")
        XCTAssertEqual(newline("enum Mode|"), "enum Mode\n    |")
    }

    func testSwitchArmIndents() {
        XCTAssertEqual(newline("switch k\n    1 =>|"), "switch k\n    1 =>\n        |")
    }

    func testNestedBlocks() {
        XCTAssertEqual(
            newline("if condition\n    if otherCondition|"), "if condition\n    if otherCondition\n        |")
        XCTAssertEqual(
            newline("if a\n    if b\n        x = 1|"), "if a\n    if b\n        x = 1\n        |")
    }

    func testOrdinaryLinesKeepTheirIndentation() {
        XCTAssertEqual(newline("    value = close|"), "    value = close\n    |")
        XCTAssertEqual(newline("x = 1|"), "x = 1\n|")
        XCTAssertEqual(newline("// if x|"), "// if x\n|")
    }

    func testTrailingCommentDoesNotHideABlockOpener() {
        XCTAssertEqual(newline("if a // note|"), "if a // note\n    |")
    }

    func testOperatorAtLineEndContinues() {
        XCTAssertEqual(newline("total = a +|"), "total = a +\n    |")
        XCTAssertEqual(newline("x = foo and|"), "x = foo and\n    |")
    }

    func testReturnMidLineKeepsIndentOnly() {
        XCTAssertEqual(newline("if a| and b"), "if a\n| and b")
        XCTAssertEqual(newline("    x = |1"), "    x = \n    |1")
    }

    func testReturnInsideIndentationDropsTheOldIndentation() {
        XCTAssertEqual(newline("if a\n    |"), "if a\n\n    |")
        XCTAssertEqual(newline("    |"), "\n    |")
    }

    func testSelectionIsReplacedByTheLineBreak() {
        XCTAssertEqual(newline("x = ⟦1⟧"), "x = \n    |")
    }

    // MARK: - Multiline calls

    func testReturnInsideAnOpenCallIndentsOneLevel() {
        XCTAssertEqual(newline("value = math.max(|"), "value = math.max(\n    |")
        XCTAssertEqual(newline("plotshape(|"), "plotshape(\n    |")
    }

    func testReturnBetweenEmptyPairSplitsIt() {
        XCTAssertEqual(newline("plotshape(|)"), "plotshape(\n    |\n)")
        XCTAssertEqual(newline("    f(|)"), "    f(\n        |\n    )")
        XCTAssertEqual(newline("a = [|]"), "a = [\n    |\n]")
    }

    func testContinuationLinesKeepTheirIndentation() {
        XCTAssertEqual(
            newline("plotshape(\n    bullish,|"), "plotshape(\n    bullish,\n    |")
        XCTAssertEqual(
            newline("value = math.max(\n    ta.sma(close, 20),|\n)"),
            "value = math.max(\n    ta.sma(close, 20),\n    |\n)")
    }

    func testReturnAfterNestedClosedCallStaysInTheOuterCall() {
        XCTAssertEqual(newline("f(g(a), |"), "f(g(a), \n    |")
    }

    func testBlockKeywordInsideCallDoesNotOpenABlock() {
        XCTAssertEqual(newline("f(a ? b : c,|"), "f(a ? b : c,\n    |")
    }

    // MARK: - Closing delimiter

    func testClosingParenAlignsWithItsOpeningLine() {
        XCTAssertEqual(
            closing("plotshape(\n    bullish,\n    title=\"Bullish\"\n    |", ")"),
            "plotshape(\n    bullish,\n    title=\"Bullish\"\n)|")
        XCTAssertEqual(
            closing("    f(\n        a\n        |", ")"), "    f(\n        a\n    )|")
        XCTAssertEqual(closing("a = [\n    1\n    |", "]"), "a = [\n    1\n]|")
    }

    func testClosingDelimiterAfterCodeIsTypedNormally() {
        XCTAssertNil(closing("f(\n    a|", ")"))
    }

    func testMismatchedClosingKindIsTypedNormally() {
        XCTAssertNil(closing("f(\n    |", "]"))
    }

    // MARK: - Tab / Shift-Tab

    func testTabInsertsSpacesToNextLevel() {
        XCTAssertEqual(indent("|"), "    |")
        XCTAssertEqual(indent("x = close|"), "x = close   |")
        XCTAssertEqual(indent("ab|"), "ab  |")
        XCTAssertEqual(indent("    |x"), "        |x")
    }

    func testTabNeverInsertsALiteralTab() {
        XCTAssertFalse(indent("a|")!.contains("\t"))
    }

    func testTabIndentsEveryLineOfAMultilineSelection() {
        XCTAssertEqual(indent("⟦x = close\ny = open⟧"), "⟦    x = close\n    y = open⟧")
        XCTAssertEqual(indent("x = ⟦close\ny = o⟧pen"), "    x = ⟦close\n    y = o⟧pen")
    }

    func testTabSkipsEmptyLinesAndExcludesALineTheSelectionOnlyTouches() {
        XCTAssertEqual(indent("⟦a\n\nb⟧"), "⟦    a\n\n    b⟧")
        XCTAssertEqual(indent("⟦a\nb\n⟧c"), "⟦    a\n    b\n⟧c")
    }

    func testShiftTabRemovesOneLevel() {
        XCTAssertEqual(outdent("        x|"), "    x|")
        XCTAssertEqual(outdent("  x|"), "x|")
        XCTAssertEqual(outdent("\tx|"), "x|")
    }

    func testShiftTabOnUnindentedLineDoesNothing() {
        XCTAssertNil(outdent("x|"))
    }

    func testShiftTabOutdentsEveryLineOfAMultilineSelection() {
        XCTAssertEqual(outdent("⟦    x = close\n    y = open⟧"), "⟦x = close\ny = open⟧")
        XCTAssertEqual(outdent("⟦        a\n    b⟧"), "⟦    a\nb⟧")
    }

    func testIndentationEditsStayCorrectAroundMultibyteText() {
        XCTAssertEqual(indent("⟦// 🚀\nm = \"価格\"⟧"), "⟦    // 🚀\n    m = \"価格\"⟧")
    }

    // MARK: - Paste

    func testPastedStructureIsRebasedOntoTheCurrentIndentation() {
        XCTAssertEqual(
            paste("x = close\nif x > open\n    y = high", into: "if a\n    |"),
            "if a\n    x = close\n    if x > open\n        y = high|")
    }

    func testPastedBlockCopiedFromADeeperLevelIsFlattenedToTheTarget() {
        XCTAssertEqual(
            paste("    x = 1\n    y = 2", into: "if a\n    |"), "if a\n    x = 1\n    y = 2|")
    }

    func testPastedFirstLineThatOpensABlockKeepsItsBody() {
        XCTAssertEqual(
            paste("if a\n    b", into: "if z\n    |"), "if z\n    if a\n        b|")
    }

    func testPasteIsLeftAloneOutsideAnIndentedPrefix() {
        XCTAssertNil(paste("a\nb", into: "|"))
        XCTAssertNil(paste("a\nb", into: "x = |"))
        XCTAssertNil(paste("single line", into: "    |"))
    }
}
