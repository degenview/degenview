import AppKit
import XCTest

@testable import DegenView

final class PineSyntaxClassifierTests: XCTestCase {
    private typealias Category = PineSyntaxCategory

    /// Every classified piece of `source` as (text, category), in source order.
    private func pieces(_ source: String) -> [(text: String, category: Category)] {
        let text = source as NSString
        return PineSyntaxClassifier.classify(source)
            .sorted { $0.range.location < $1.range.location }
            .map { (text.substring(with: $0.range), $0.category) }
    }

    /// Category of the `occurrence`th (0-based) piece spelled `text`.
    private func category(
        of text: String, in source: String, occurrence: Int = 0,
        file: StaticString = #filePath, line: UInt = #line
    ) -> Category? {
        let matches = pieces(source).filter { $0.text == text }
        guard occurrence < matches.count else {
            XCTFail("no piece \(text) #\(occurrence) in \(source)", file: file, line: line)
            return nil
        }
        return matches[occurrence].category
    }

    private func assertCategory(
        _ expected: Category, _ text: String, in source: String, occurrence: Int = 0,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(
            category(of: text, in: source, occurrence: occurrence, file: file, line: line), expected,
            "\(text) in \(source)", file: file, line: line)
    }

    // MARK: - Builtin variables

    func testBareBuiltinVariables() {
        for name in ["open", "high", "low", "close", "volume", "bar_index", "hl2", "hlc3", "ohlc4",
            "hlcc4", "time", "time_close", "last_bar_index"]
        {
            assertCategory(.builtinVariable, name, in: name)
        }
    }

    func testUserAssignmentOfBuiltin() {
        let source = "foo = close"
        assertCategory(.identifier, "foo", in: source)
        assertCategory(.builtinVariable, "close", in: source)
    }

    func testBuiltinNamesNeverMatchBySubstring() {
        let source = "closeValue = close\nmyclose = volumeAverage + timeOffset"
        assertCategory(.identifier, "closeValue", in: source)
        assertCategory(.builtinVariable, "close", in: source)
        for name in ["myclose", "volumeAverage", "timeOffset"] {
            assertCategory(.identifier, name, in: source)
        }
    }

    // MARK: - Namespaces

    func testNamespaceFunction() {
        let source = "fast = ta.ema(close, 20)"
        assertCategory(.identifier, "fast", in: source)
        assertCategory(.builtinNamespace, "ta", in: source)
        assertCategory(.builtinFunction, "ema", in: source)
        assertCategory(.builtinVariable, "close", in: source)
        assertCategory(.number, "20", in: source)
        assertCategory(.punctuation, ".", in: source)
    }

    func testNamespaceConstant() {
        let source = "c = color.red"
        assertCategory(.identifier, "c", in: source)
        assertCategory(.builtinNamespace, "color", in: source)
        assertCategory(.builtinConstant, "red", in: source)
    }

    func testNamespaceValue() {
        let source = "confirmed = barstate.isconfirmed"
        assertCategory(.identifier, "confirmed", in: source)
        assertCategory(.builtinNamespace, "barstate", in: source)
        assertCategory(.builtinVariable, "isconfirmed", in: source)
    }

    func testCallVersusValueUnderSameNamespace() {
        assertCategory(.builtinFunction, "entry", in: "strategy.entry(\"L\", strategy.long)")
        assertCategory(.builtinConstant, "long", in: "strategy.entry(\"L\", strategy.long)")
        assertCategory(.builtinVariable, "position_size", in: "x = strategy.position_size")
        assertCategory(.builtinFunction, "strategy", in: "strategy(\"S\")")
        assertCategory(.builtinNamespace, "strategy", in: "x = strategy.equity")
    }

    func testQualifiedBuiltinsAcrossNamespaces() {
        assertCategory(.builtinFunction, "int", in: "length = input.int(14)")
        assertCategory(.builtinFunction, "max", in: "math.max(a, b)")
        assertCategory(.builtinConstant, "pi", in: "x = math.pi")
        assertCategory(.builtinVariable, "tickerid", in: "x = syminfo.tickerid")
        assertCategory(.builtinVariable, "period", in: "x = timeframe.period")
        assertCategory(.builtinFunction, "security", in: "request.security(a, b, c)")
        assertCategory(.builtinConstant, "style_dotted", in: "line.style_dotted")
        assertCategory(.builtinConstant, "tiny", in: "size.tiny")
        assertCategory(.builtinConstant, "percent", in: "strategy.commission.percent")
        assertCategory(.builtinNamespace, "commission", in: "strategy.commission.percent")
    }

    func testMembersOfUserValuesAreNotBuiltin() {
        assertCategory(.identifier, "close", in: "foo.close")
        assertCategory(.identifier, "red", in: "foo.red")
        assertCategory(.identifier, "ema", in: "foo().ema")
        assertCategory(.identifier, "close", in: "arr[0].close")
    }

    // MARK: - Functions

    func testBuiltinVersusUserFunction() {
        let source = "plot(close)\nmyPlot(close)"
        assertCategory(.builtinFunction, "plot", in: source)
        assertCategory(.identifier, "myPlot", in: source)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 0)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 1)
    }

    func testGlobalBuiltinFunctions() {
        for name in ["indicator", "plotshape", "plotchar", "hline", "fill", "alert", "alertcondition",
            "bgcolor", "barcolor", "nz", "timestamp"]
        {
            assertCategory(.builtinFunction, name, in: "\(name)(x)")
        }
        assertCategory(.builtinFunction, "na", in: "na(x)")
        assertCategory(.builtinConstant, "na", in: "x = na")
    }

    func testBareNameBothVariableAndFunction() {
        assertCategory(.builtinVariable, "year", in: "x = year")
        assertCategory(.builtinFunction, "year", in: "x = year(time)")
    }

    func testNamedArgumentsAreNotBuiltin() {
        let source = "plot(close, color=color.red, title=\"t\")"
        assertCategory(.identifier, "color", in: source, occurrence: 0)
        assertCategory(.builtinNamespace, "color", in: source, occurrence: 1)
        assertCategory(.identifier, "title", in: source)
        assertCategory(.identifier, "close", in: "plot(close=1)")
    }

    // MARK: - Keywords, types, qualifiers

    func testKeywords() {
        for word in ["and", "or", "not", "if", "else", "var", "varip", "for", "while", "switch",
            "break", "continue"]
        {
            assertCategory(.keyword, word, in: "\(word) x")
        }
        for word in ["import", "export", "method", "type", "enum", "as"] {
            assertCategory(.keyword, word, in: "\(word) x")
        }
    }

    func testForHeaderWords() {
        assertCategory(.keyword, "to", in: "for i = 0 to 5 by 2\n    x := i\n")
        assertCategory(.keyword, "by", in: "for i = 0 to 5 by 2\n    x := i\n")
        assertCategory(.keyword, "in", in: "for x in xs\n    y := x\n")
        // Outside a for header they are ordinary names.
        assertCategory(.identifier, "to", in: "to = 1")
        assertCategory(.identifier, "by", in: "x = by + 1")
    }

    func testTypesAndQualifiers() {
        for type in ["int", "float", "bool", "string", "color"] {
            assertCategory(.type, type, in: "\(type) x = na")
        }
        for type in ["line", "label", "box", "table", "array", "linefill", "polyline", "map", "matrix"] {
            assertCategory(.type, type, in: "\(type) x = na")
        }
        for qualifier in ["const", "input", "simple", "series"] {
            assertCategory(.type, qualifier, in: "\(qualifier) float x = 1")
        }
        assertCategory(.builtinNamespace, "input", in: "x = input.int(1)")
        assertCategory(.builtinFunction, "float", in: "x = float(y)")
        assertCategory(.builtinNamespace, "line", in: "l = line.new(1, 2, 3, 4)")
        assertCategory(.type, "array", in: "array<float> a = array.new_float()")
        assertCategory(.builtinNamespace, "array", in: "array<float> a = array.new_float()", occurrence: 1)
    }

    func testLiterals() {
        assertCategory(.builtinConstant, "true", in: "x = true")
        assertCategory(.builtinConstant, "false", in: "x = false")
        assertCategory(.colorLiteral, "#FF8800", in: "x = #FF8800")
        assertCategory(.number, "1.5e3", in: "x = 1.5e3")
        assertCategory(.operator, "+", in: "x = a + b")
        assertCategory(.operator, "=>", in: "f(a) => a")
    }

    // MARK: - Lexical precedence

    func testCommentsHideBuiltins() {
        let source = "// close ta.ema color.red"
        XCTAssertEqual(pieces(source).map(\.category), [.comment])
        let trailing = pieces("x = 1 // close ta.ema")
        XCTAssertEqual(trailing.last?.category, .comment)
        XCTAssertFalse(trailing.contains { $0.text == "close" })
    }

    func testStringsHideBuiltins() {
        let source = "x = \"close ta.ema color.red\""
        let strings = pieces(source).filter { $0.category == .string }
        XCTAssertEqual(strings.map(\.text), ["\"close ta.ema color.red\""])
        XCTAssertFalse(pieces(source).contains { $0.category == .builtinVariable })
        // `//` inside a string is not a comment.
        XCTAssertEqual(pieces("x = \"a // b\"").filter { $0.category == .comment }.count, 0)
    }

    func testAnnotationsAreNotOrdinaryComments() {
        let source = "//@version=6\nindicator(\"T\")"
        assertCategory(.annotation, "//@version=6", in: source)
        let described = pieces("//@description Shows close")
        XCTAssertEqual(described.map(\.category), [.annotation, .comment])
        XCTAssertEqual(described.first?.text, "//@description")
        // A space after the slashes is a plain comment.
        XCTAssertEqual(pieces("// @version=6").map(\.category), [.comment])
    }

    // MARK: - Shadowing

    func testDeclarationShadowsBuiltin() {
        let source = "close = 123\nplot(close)"
        assertCategory(.identifier, "close", in: source, occurrence: 0)
        assertCategory(.identifier, "close", in: source, occurrence: 1)
    }

    func testDeclarationIsVisibleOnlyAfterItsStatement() {
        let source = "x = close\nclose = x"
        assertCategory(.builtinVariable, "close", in: source, occurrence: 0)
        assertCategory(.identifier, "close", in: source, occurrence: 1)
    }

    func testFunctionParameterShadowsOnlyInsideBody() {
        let source = "f(close) =>\n    close * 2\nplot(close)"
        assertCategory(.identifier, "f", in: source)
        assertCategory(.identifier, "close", in: source, occurrence: 0)
        assertCategory(.identifier, "close", in: source, occurrence: 1)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 2)
    }

    func testOneLineFunctionParameterEndsWithTheLine() {
        let source = "f(close) => close * 2\nplot(close)"
        assertCategory(.identifier, "close", in: source, occurrence: 1)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 2)
    }

    func testUserFunctionShadowsBuiltinFunction() {
        let source = "nz(x) => x\ny = nz(1)"
        assertCategory(.identifier, "nz", in: source, occurrence: 0)
        assertCategory(.identifier, "nz", in: source, occurrence: 1)
    }

    func testLoopVariablesAndTuples() {
        let loop = "for close = 0 to 3\n    x := close\nplot(close)"
        assertCategory(.identifier, "close", in: loop, occurrence: 1)
        assertCategory(.builtinVariable, "close", in: loop, occurrence: 2)
        let tuple = "[open, high] = f()\nplot(open)"
        assertCategory(.identifier, "open", in: tuple, occurrence: 1)
        assertCategory(.builtinVariable, "close", in: "[open, high] = f()\nplot(close)")
    }

    func testVarDeclarationShadows() {
        assertCategory(.identifier, "volume", in: "var float volume = 0\nplot(volume)", occurrence: 1)
    }

    func testBlockLocalDeclarationEndsAtDedent() {
        let source = "if true\n    close = 1\nplot(close)"
        assertCategory(.builtinVariable, "close", in: source, occurrence: 1)
    }

    // MARK: - Text encoding

    func testUnicodeAroundIdentifiers() {
        let source = "日本 = close // é\nπ = \"é\" + open"
        let found = pieces(source)
        XCTAssertEqual(found.first { $0.text == "日本" }?.category, .identifier)
        XCTAssertEqual(found.first { $0.text == "close" }?.category, .builtinVariable)
        XCTAssertEqual(found.first { $0.text == "π" }?.category, .identifier)
        XCTAssertEqual(found.first { $0.text == "open" }?.category, .builtinVariable)
        XCTAssertEqual(found.first { $0.text == "\"é\"" }?.category, .string)
    }

    func testCRLFLineEndingsKeepRanges() {
        let source = "a = 1\r\nb = close\r\nc = ta.sma(close, 5)"
        assertCategory(.identifier, "b", in: source)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 0)
        assertCategory(.builtinFunction, "sma", in: source)
        assertCategory(.builtinVariable, "close", in: source, occurrence: 1)
    }

    func testOversizedSourceKeepsLexicalColoring() {
        let source = "// c\n" + String(repeating: "x = close\n", count: 12_000)
        let found = PineSyntaxClassifier.classify(source)
        XCTAssertEqual(found.map(\.category), [.comment])
    }

    func testIncompleteInputDoesNotCrash() {
        for source in ["", "ta.", "ta.ema(", "\"abc", "x = ", "for", "f(", "[a, b", "color.", "..", "#12"] {
            _ = PineSyntaxClassifier.classify(source)
        }
    }

    // MARK: - Highlighter

    @MainActor
    func testHighlighterColorsBuiltinsApartFromUserNames() throws {
        let textView = NSTextView()
        textView.string = "foo = close"
        PineSyntaxHighlighter.apply(to: textView)
        let storage = try XCTUnwrap(textView.textStorage)
        let user = storage.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor
        let builtin = storage.attribute(.foregroundColor, at: 6, effectiveRange: nil) as? NSColor
        XCTAssertEqual(user, .labelColor)
        XCTAssertEqual(builtin, PineSyntaxTheme.standard.color(for: .builtinVariable))
        XCTAssertNotEqual(user, builtin)
    }

    func testThemeSeparatesTheBuiltinCategories() {
        let theme = PineSyntaxTheme.standard
        let colors = [
            Category.builtinNamespace, .builtinFunction, .builtinVariable, .builtinConstant, .keyword,
            .type, .number, .string, .annotation,
        ].compactMap { theme.color(for: $0) }
        XCTAssertEqual(Set(colors).count, colors.count)
        XCTAssertNil(theme.color(for: .identifier))
    }
}
