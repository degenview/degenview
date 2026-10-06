import XCTest

@testable import DegenView

final class PineCompletionInsertionTests: XCTestCase {
    private func accept(_ marked: String, _ label: String) -> String? {
        PineCompletionFixture(marked).accepting(label)
    }

    private func acceptance(_ marked: String, _ label: String) throws -> PineCompletionInsertion.Acceptance {
        let fixture = PineCompletionFixture(marked)
        let item = try XCTUnwrap(fixture.item(label))
        return try XCTUnwrap(PineCompletionInsertion.accept(item, in: fixture.editor.context))
    }

    // MARK: Callables

    func testCallableIsPairedWhenTypingAParenthesisWouldPair() {
        XCTAssertEqual(accept("ta.rs|", "rsi"), "ta.rsi(|)")
        XCTAssertEqual(accept("plot(ta.rs|)", "rsi"), "plot(ta.rsi(|))")
        XCTAssertEqual(accept("x = ta.rs|\n", "rsi"), "x = ta.rsi(|)\n")
    }

    func testAnExistingParenthesisIsNeverDuplicated() {
        XCTAssertEqual(accept("ta.rs|(", "rsi"), "ta.rsi(|")
        XCTAssertEqual(accept("ta.rs|()", "rsi"), "ta.rsi(|)")
        XCTAssertEqual(accept("ta.rs|(close, 14)", "rsi"), "ta.rsi(|close, 14)")
    }

    func testCallableBeforeATokenItWouldSwallowGetsABareParenthesis() {
        XCTAssertEqual(accept("ta.rs| + 1", "rsi"), "ta.rsi(|) + 1", "whitespace allows a pair")
        XCTAssertEqual(accept("x = [ta.rs|]", "rsi"), "x = [ta.rsi(|)]")
        XCTAssertEqual(accept("ta.rs|\"x\"", "rsi"), "ta.rsi(|\"x\"", "a pair would swallow the string")
        XCTAssertEqual(accept("ta.rs|2", "rsi"), "ta.rsi(|)", "the whole word is replaced")
    }

    func testAcceptedCodeIsBalanced() throws {
        for marked in ["ta.rs|", "plot(ta.rs|)", "ta.rs|(", "ta.rs|()", "plot(close, ta.rs|, 1)"] {
            let fixture = PineCompletionFixture(marked)
            let item = try XCTUnwrap(fixture.item("rsi"), marked)
            let edit = try XCTUnwrap(PineCompletionInsertion.accept(item, in: fixture.editor.context)).edit
            let text = edit.applied(to: fixture.editor.text).text
            let snapshot = PineLexicalSnapshot(source: text)
            let unmatchedBefore = PineLexicalSnapshot(source: fixture.editor.text).partners.filter { $0 < 0 }.count
            XCTAssertEqual(snapshot.partners.filter { $0 < 0 }.count, unmatchedBefore, marked)
            XCTAssertFalse(text.contains("ta.ta."), marked)
            XCTAssertFalse(text.contains("(("), marked)
        }
    }

    func testUserCallablesFollowTheSameRule() {
        let source = "myAverage(source, length) =>\n    ta.sma(source, length)\nx = myA|"
        let expected = "myAverage(source, length) =>\n    ta.sma(source, length)\nx = myAverage(|)"
        XCTAssertEqual(accept(source, "myAverage"), expected)
    }

    // MARK: Replacement

    func testOnlyTheMemberIsReplacedAfterANamespace() {
        XCTAssertEqual(accept("ta.|", "rsi"), "ta.rsi(|)")
        XCTAssertEqual(accept("ta.r|", "roc"), "ta.roc(|)")
        XCTAssertEqual(accept("x = ta.r|si", "rsi"), "x = ta.rsi(|)")
    }

    func testVariablesGetOnlyTheirName() {
        XCTAssertEqual(accept("plot(clo|)", "close"), "plot(close|)")
        XCTAssertEqual(accept("plot(clo|se)", "close"), "plot(close|)")
        XCTAssertEqual(accept("x = bar_|", "bar_index"), "x = bar_index|")
        XCTAssertEqual(accept("myValue = 1\nx = myVa|", "myValue"), "myValue = 1\nx = myValue|")
        XCTAssertEqual(accept("color.r|", "red"), "color.red|")
    }

    func testNothingIsAddedForCapitalisationOrNaming() {
        XCTAssertEqual(accept("x = CLO|", "close"), "x = close|")
    }

    // MARK: Namespaces

    func testNamespaceInsertsItsDotAndOpensMembers() throws {
        XCTAssertEqual(accept("x = t|", "ta"), "x = ta.|")
        XCTAssertEqual(try acceptance("x = t|", "ta").followUp, .memberCompletion)
        XCTAssertEqual(accept("x = t|.sma", "ta"), "x = ta.|sma", "an existing dot is stepped over")
        XCTAssertEqual(accept("x = co|", "color"), "x = color.|")
    }

    func testFollowUps() throws {
        XCTAssertEqual(try acceptance("ta.rs|", "rsi").followUp, .signatureHelp)
        XCTAssertEqual(try acceptance("plot(clo|)", "close").followUp, .none)
        XCTAssertEqual(try acceptance("plot(close, ti|)", "title").followUp, .none)
    }

    // MARK: Argument names

    func testArgumentNameInsertsEquals() {
        XCTAssertEqual(accept("plot(close, ti|)", "title"), "plot(close, title = |)")
        XCTAssertEqual(accept("plot(close, ti| = \"x\")", "title"), "plot(close, title| = \"x\")")
    }

    // MARK: Stale ranges

    func testAnItemWhoseRangeIsNoLongerAWordIsRefused() throws {
        let fixture = PineCompletionFixture("plot(clo|)")
        let item = try XCTUnwrap(fixture.item("close"))
        let changed = PineEditorFixture("plot(1 + |)")
        XCTAssertNil(PineCompletionInsertion.accept(item, in: changed.context))
        let beyond = PineEditorFixture("x|")
        XCTAssertNil(PineCompletionInsertion.accept(item, in: beyond.context))
    }

    // MARK: Unicode

    func testRangesAreCorrectAfterNonBMPCharacters() {
        XCTAssertEqual(
            accept("// 😀 日本語 é\nmyValue = 1\nplot(myVa|)", "myValue"),
            "// 😀 日本語 é\nmyValue = 1\nplot(myValue|)")
    }
}
