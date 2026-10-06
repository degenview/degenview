import XCTest

@testable import DegenView

final class PineCompletionContextTests: XCTestCase {
    private func context(_ marked: String, explicit: Bool = false) -> PineCompletionContext {
        PineCompletionFixture(marked, explicit: explicit).context
    }

    // MARK: Suppression

    func testCommentsAndStringsSuppressCompletion() {
        XCTAssertEqual(context("// ta.rs|").suppression, .comment)
        XCTAssertEqual(context("x = 1 // ta.rs|").suppression, .comment)
        XCTAssertEqual(context("x = \"ta.rs|\"").suppression, .string)
        XCTAssertEqual(context("label.new(text=\"ta.rs|\")").suppression, .string)
        XCTAssertNil(context("x = \"a\" + ta.rs|").suppression, "after the closing quote it is code again")
        XCTAssertEqual(context("x = ⟦ta.rs⟧").suppression, .selection)
    }

    func testNothingSuppressedInPlainCode() {
        XCTAssertNil(context("plot(clo|)").suppression)
        XCTAssertNil(context("|").suppression)
    }

    // MARK: Words and ranges

    func testPrefixAndReplacementRange() {
        let c = context("plot(clos|)")
        XCTAssertEqual(c.prefix, "clos")
        XCTAssertEqual(c.replacementRange, NSRange(location: 5, length: 4))
    }

    func testReplacementCoversTheWholeWordWhenTheCaretIsInsideIt() {
        let c = context("x = ta.r|sx")
        XCTAssertEqual(c.prefix, "r")
        XCTAssertEqual(c.replacementRange, NSRange(location: 7, length: 3))
    }

    func testEmptyPrefixAtAFreshPosition() {
        let c = context("x = |")
        XCTAssertEqual(c.prefix, "")
        XCTAssertEqual(c.replacementRange, NSRange(location: 4, length: 0))
    }

    func testRangesAreUTF16AfterNonBMPText() {
        let c = context("// 😀 日本語\nmyVal = 1\nplot(myV|)")
        XCTAssertEqual(c.prefix, "myV")
        XCTAssertEqual(c.replacementRange, NSRange(location: 25, length: 3))
    }

    func testRangesSurviveCRLF() {
        let c = context("a = 1\r\nplot(a|)\r\n")
        XCTAssertEqual(c.prefix, "a")
        XCTAssertEqual(c.replacementRange, NSRange(location: 12, length: 1))
    }

    func testAcceptanceRangesAfterEmojiInsideAString() {
        let fixture = PineCompletionFixture("s = \"😀\"\nplot(clo|)")
        XCTAssertEqual(fixture.accepting("close"), "s = \"😀\"\nplot(close|)")
    }

    // MARK: Position

    func testStatementStartAndExpression() {
        XCTAssertEqual(context("pl|").position, .statementStart)
        XCTAssertEqual(context("x = clo|").position, .expression)
        XCTAssertEqual(context("plot(clo|)").position, .expression)
        XCTAssertEqual(context("foo(a) =>\n    ta.sma(a, 1)\npl|").position, .statementStart)
    }

    func testAContinuedLineIsNotAStatementStart() {
        XCTAssertEqual(context("x = ta.sma(close,\n  cl|").position, .expression)
        XCTAssertEqual(context("x = 1 +\n    cl|").position, .expression)
    }

    func testMemberAccess() {
        XCTAssertEqual(context("ta.|").position, .member(base: ["ta"]))
        XCTAssertEqual(context("x = ta.r|").position, .member(base: ["ta"]))
        XCTAssertEqual(context("chart.point.|").position, .member(base: ["chart", "point"]))
        XCTAssertEqual(context("color.r|").position, .member(base: ["color"]), "color lexes as a type keyword")
        XCTAssertEqual(context("ta.|").prefix, "")
    }

    func testADotAfterAValueIsNotAMember() {
        XCTAssertEqual(context("f().|").suppression, .valueMember)
        XCTAssertEqual(context("a[0].|").suppression, .valueMember)
        XCTAssertEqual(context("(1 + 2).|").suppression, .valueMember)
    }

    func testNamingADeclarationIsSuppressed() {
        XCTAssertEqual(context("float |").suppression, .declarationName)
        XCTAssertEqual(context("float my|").suppression, .declarationName)
        XCTAssertEqual(context("var float |").suppression, .declarationName)
        XCTAssertEqual(context("Point |").suppression, .declarationName)
        XCTAssertEqual(context("for |").suppression, .declarationName)
        XCTAssertEqual(context("method |").suppression, .declarationName)
        XCTAssertEqual(context("type |").suppression, .declarationName)
        XCTAssertEqual(context("array<float> |").suppression, .declarationName)
        XCTAssertNil(context("for i = 0 to cl|").suppression)
        XCTAssertNil(context("plot(a, b|)").suppression)
    }

    func testAfterVarATypeIsExpected() {
        XCTAssertEqual(context("var |").position, .typeExpected)
        XCTAssertEqual(context("type Point\n    fl|").position, .typeExpected)
    }

    func testImportPaths() {
        XCTAssertEqual(context("import |").position, .importPath(typedBefore: ""))
        XCTAssertEqual(context("import user/|").position, .importPath(typedBefore: "user/"))
        XCTAssertEqual(context("import user/Li|").position, .importPath(typedBefore: "user/"))
        XCTAssertEqual(context("import user/Lib/1 as |").suppression, .declarationName)
    }

    // MARK: Scope

    func testScopeFollowsTheCaretsIndentation() {
        let inside = context("foo(a) =>\n    b = a\n    |")
        let outside = context("foo(a) =>\n    b = a\n|")
        XCTAssertNotEqual(inside.scope, 0)
        XCTAssertEqual(outside.scope, 0)
        XCTAssertEqual(inside.lineIndent, 4)
    }

    // MARK: Incomplete code

    func testIncompleteSourceStillHasAContext() {
        for marked in ["x = ta.rs|\nif\nfoo(", "foo(\nx = 1\nta.r|", "if\nplot(clo|", "=> =>\nta.|", "((((\n[[[\ncl|"] {
            let c = context(marked)
            XCTAssertNil(c.suppression, marked)
        }
        XCTAssertEqual(context("x = ta.rs|\nif\nfoo(").position, .member(base: ["ta"]))
        XCTAssertEqual(context("foo(\nx = 1\nta.r|").position, .member(base: ["ta"]))
    }

    // MARK: Call sites

    func testCallSite() throws {
        let site = try XCTUnwrap(context("ta.sma(close, |").callSite)
        XCTAssertEqual(site.callee, ["ta", "sma"])
        XCTAssertEqual(site.argumentIndex, 1)
        XCTAssertTrue(site.isAtArgumentStart)
        XCTAssertNil(context("x = 1|").callSite)
    }

    func testNestedCallsNameTheInnermost() throws {
        XCTAssertEqual(try XCTUnwrap(context("ta.ema(ta.sma(close, 20), |").callSite).callee, ["ta", "ema"])
        XCTAssertEqual(try XCTUnwrap(context("ta.ema(ta.sma(clo|, 20), 1)").callSite).callee, ["ta", "sma"])
        XCTAssertEqual(try XCTUnwrap(context("ta.ema((1 + 2) * |").callSite).callee, ["ta", "ema"])
    }

    func testCommasInStringsAndNestedBracketsDoNotCount() throws {
        XCTAssertEqual(try XCTUnwrap(context("foo(\"a,b\", |").callSite).argumentIndex, 1)
        XCTAssertEqual(try XCTUnwrap(context("foo(bar(1, 2), [1, 2], |").callSite).argumentIndex, 2)
        XCTAssertEqual(try XCTUnwrap(context("foo(1, // x, y\n    |").callSite).argumentIndex, 1)
    }

    func testNamedArguments() throws {
        let site = try XCTUnwrap(context("plot(close, title = \"x\", co|").callSite)
        XCTAssertEqual(site.namedArguments, ["title"])
        XCTAssertEqual(site.positionalBefore, 1)
        XCTAssertTrue(site.isAtArgumentStart)
        let active = try XCTUnwrap(context("plot(close, title = |").callSite)
        XCTAssertEqual(active.activeName, "title")
        XCTAssertFalse(try XCTUnwrap(context("plot(close + cl|").callSite).isAtArgumentStart)
    }

    func testAGroupingParenthesisIsNotACall() throws {
        XCTAssertEqual(try XCTUnwrap(context("plot((1 + |)").callSite).callee, ["plot"])
        XCTAssertNil(context("x = (1 + |)").callSite)
        XCTAssertNil(context("if (a > |)").callSite)
    }

    func testAStrayOpenParenAboveDoesNotEncloseLaterStatements() {
        XCTAssertNil(context("plot(\nx = 1\ny = |").callSite)
    }
}
