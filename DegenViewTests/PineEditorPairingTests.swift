import XCTest

@testable import DegenView

final class PineEditorPairingTests: XCTestCase {
    private func typed(_ before: String, _ key: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineEditorPairing.typed(key, in: fixture.context))
    }

    private func backspace(_ before: String) -> String? {
        let fixture = PineEditorFixture(before)
        return fixture.result(PineEditorPairing.backspace(in: fixture.context))
    }

    // MARK: - Insertion

    func testOpeningDelimitersPair() {
        XCTAssertEqual(typed("|", "("), "(|)")
        XCTAssertEqual(typed("foo|", "("), "foo(|)")
        XCTAssertEqual(typed("a = |", "["), "a = [|]")
        XCTAssertEqual(typed("ta.sma|", "("), "ta.sma(|)")
    }

    func testBracesAreNotPaired() {
        XCTAssertNil(typed("|", "{"))
        XCTAssertNil(typed("a|}", "}"))
    }

    func testNoPairBeforeWordCharacters() {
        XCTAssertNil(typed("|foo", "("))
    }

    func testPairBeforeClosingDelimiter() {
        XCTAssertEqual(typed("foo(a, |)", "("), "foo(a, (|))")
    }

    func testQuotesPairAndKeepTheCaretBetween() {
        XCTAssertEqual(typed("|", "\""), "\"|\"")
        XCTAssertEqual(typed("title = |", "'"), "title = '|'")
    }

    func testNoQuotePairAfterWordCharacterOrQuote() {
        XCTAssertNil(typed("don|", "'"))
        XCTAssertNil(typed("\"\"|", "\""))
    }

    // MARK: - Context

    func testNothingPairsInsideComments() {
        XCTAssertNil(typed("// call foo|", "("))
        XCTAssertNil(typed("// say |", "\""))
    }

    func testBracketsDoNotPairInsideStrings() {
        XCTAssertNil(typed("x = \"a|b\"", "("))
        XCTAssertNil(typed("x = \"a|b\"", "'"))
    }

    func testSameQuoteInsideStringIsEscaped() {
        XCTAssertEqual(typed("x = \"a|b\"", "\""), "x = \"a\\\"|b\"")
    }

    func testUnterminatedStringIsClosedByAQuote() {
        XCTAssertNil(typed("x = \"abc|", "\""))
    }

    // MARK: - Overtype

    func testClosingDelimiterIsOvertyped() {
        XCTAssertEqual(typed("foo(|)", ")"), "foo()|")
        XCTAssertEqual(typed("ta.sma(close, 20|)", ")"), "ta.sma(close, 20)|")
        XCTAssertEqual(typed("a[1|]", "]"), "a[1]|")
    }

    func testStrayClosingDelimiterIsTypedNormally() {
        XCTAssertNil(typed("a|)", ")"))
    }

    func testClosingQuoteIsOvertyped() {
        XCTAssertEqual(typed("x = \"abc|\"", "\""), "x = \"abc\"|")
        XCTAssertEqual(typed("x = 'abc|'", "'"), "x = 'abc'|")
    }

    func testOvertypeWorksPastMultibyteText() {
        XCTAssertEqual(typed("m = \"価格\"\nplot(🚀|)", ")"), "m = \"価格\"\nplot(🚀)|")
        XCTAssertEqual(typed("// 🚀\nplot(|)", ")"), "// 🚀\nplot()|")
    }

    // MARK: - Backspace

    func testBackspaceRemovesEmptyPairs() {
        XCTAssertEqual(backspace("(|)"), "|")
        XCTAssertEqual(backspace("ta.sma(|)"), "ta.sma|")
        XCTAssertEqual(backspace("a[|]"), "a|")
        XCTAssertEqual(backspace("x = \"|\""), "x = |")
        XCTAssertEqual(backspace("x = '|'"), "x = |")
    }

    func testBackspaceKeepsPairsWithContent() {
        XCTAssertNil(backspace("(a|)"))
        XCTAssertNil(backspace("(|a)"))
        XCTAssertNil(backspace("x = \"a|\""))
    }

    func testBackspaceInsideStringTextIsPlain() {
        XCTAssertNil(backspace("x = \"(|)\""))
    }

    func testBackspaceAfterMultibyteText() {
        XCTAssertEqual(backspace("// 🚀\nf(|)"), "// 🚀\nf|")
    }

    // MARK: - Wrapping

    func testSelectionIsWrapped() {
        XCTAssertEqual(typed("⟦close⟧", "("), "(⟦close⟧)")
        XCTAssertEqual(typed("⟦close + open⟧", "\""), "\"⟦close + open⟧\"")
        XCTAssertEqual(typed("x = ⟦a⟧", "["), "x = [⟦a⟧]")
    }

    func testClosingKeyReplacesSelectionNormally() {
        XCTAssertNil(typed("⟦close⟧", ")"))
    }

    func testWrapSurvivesMultibyteSelection() {
        XCTAssertEqual(typed("⟦🚀 価格⟧", "("), "(⟦🚀 価格⟧)")
    }

    func testOrdinaryCharactersAreNotIntercepted() {
        XCTAssertNil(typed("fo|", "o"))
        XCTAssertNil(typed("a|", "é"))
        XCTAssertNil(typed("a|", "🚀"))
    }
}
