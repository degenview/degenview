import XCTest

@testable import DegenView

final class PineSourceSymbolIndexTests: XCTestCase {
    private struct Fixture {
        let text: String
        let caret: Int
        let snapshot: PineLexicalSnapshot
        let index: PineSourceSymbolIndex

        init(_ marked: String) {
            let parts = marked.components(separatedBy: "|")
            text = parts.joined()
            caret = parts.count > 1 ? (parts[0] as NSString).length : 0
            snapshot = PineLexicalSnapshot(source: text)
            index = PineSourceSymbolIndex(snapshot: snapshot, source: text)
        }

        /// Width of the caret line's indentation, as completion will compute it.
        var lineIndent: Int {
            let before = (text as NSString).substring(to: caret)
            let line = before.split(separator: "\n", omittingEmptySubsequences: false).last ?? ""
            return line.prefix { $0 == " " }.count
        }

        var scope: Int { index.scope(atOffset: caret, lineIndent: lineIndent) }

        func visibleNames() -> [String] {
            index.visibleDeclarations(scope: scope, atOffset: caret).map(\.name)
        }

        /// Whether the identifier token starting at `offset` resolves to a user declaration.
        func isUserDefined(at offset: Int) -> Bool {
            guard
                let position = snapshot.tokens.indices.first(where: {
                    snapshot.range(of: snapshot.tokens[$0])?.location == offset
                }), case .identifier(let name) = snapshot.tokens[position].kind
            else { return false }
            return index.resolve(name, atToken: position) != nil
        }
    }

    func testAVariableIsVisibleFromTheEndOfItsStatement() {
        let f = Fixture("a = 1\nb = a\nplot(|)")
        XCTAssertEqual(Set(f.visibleNames()), ["a", "b"])
        XCTAssertTrue(f.isUserDefined(at: (f.text as NSString).range(of: "a\nplot").location))
    }

    func testANameIsNotVisibleInItsOwnStatement() {
        let f = Fixture("close = close + 1")
        XCTAssertFalse(f.isUserDefined(at: (f.text as NSString).range(of: "close +").location))
    }

    func testLocalsAndParametersStayInsideTheirFunction() {
        let source = "foo(source, length) =>\n    fooLocal = close\n    |\n\nbar() =>\n    barLocal = close\n"
        let inside = Fixture(source)
        XCTAssertTrue(Set(inside.visibleNames()).isSuperset(of: ["source", "length", "fooLocal", "foo"]))
        XCTAssertFalse(inside.visibleNames().contains("barLocal"))
        XCTAssertFalse(inside.visibleNames().contains("bar"), "declared after the caret")

        let outside = Fixture("foo(source, length) =>\n    fooLocal = close\n\n|")
        XCTAssertEqual(Set(outside.visibleNames()), ["foo"])
    }

    func testAnIndentedBlankLineAfterTheBodyIsStillInside() {
        let f = Fixture("foo(a) =>\n    b = a\n    |")
        XCTAssertTrue(f.visibleNames().contains("a"))
        let g = Fixture("foo(a) =>\n    b = a\n|")
        XCTAssertFalse(g.visibleNames().contains("a"))
    }

    func testInlineFunctionParametersAreVisibleOnlyInItsBody() {
        let f = Fixture("f(x) => x * 2\ny = |")
        XCTAssertFalse(f.visibleNames().contains("x"))
        XCTAssertTrue(f.visibleNames().contains("f"))
        let g = Fixture("f(x) => ta.sma(|)")
        XCTAssertTrue(g.visibleNames().contains("x"))
    }

    func testLoopVariablesLiveInTheLoopBody() {
        let f = Fixture("for i = 0 to 10\n    plot(|)\nplot(1)")
        XCTAssertTrue(f.visibleNames().contains("i"))
        let g = Fixture("for [i, v] in xs\n    a = 1\nplot(|)")
        XCTAssertFalse(g.visibleNames().contains("i"))
        XCTAssertFalse(g.visibleNames().contains("a"))
    }

    func testAnUnclosedCallDoesNotSwallowTheScopesAfterIt() {
        // The lexer emits no newline or dedent after the `(`; the index must not depend on them.
        let f = Fixture("foo(a) =>\n    plot(a\nb = 1\nplot(|)")
        XCTAssertTrue(f.visibleNames().contains("b"))
        XCTAssertFalse(f.visibleNames().contains("a"))
    }

    func testAWrappedLineJoinsItsStatement() {
        let f = Fixture("x = ta.sma(close,\n  20)\nplot(|)")
        XCTAssertEqual(f.visibleNames(), ["x"])
    }

    func testShadowingTheInnermostDeclarationWins() {
        let f = Fixture("value = 1\nfoo(value) =>\n    value + |")
        let declaration = f.index.resolve("value", scope: f.scope, atOffset: f.caret)
        XCTAssertEqual(declaration?.kind, .parameter)
        XCTAssertEqual(f.visibleNames().filter { $0 == "value" }.count, 1)
    }

    func testFunctionDeclarationCarriesItsParameters() throws {
        let f = Fixture("myAverage(source, float length = 14) =>\n    ta.sma(source, length)\n|")
        let function = try XCTUnwrap(f.index.declarations.first { $0.name == "myAverage" })
        XCTAssertEqual(function.kind, .function)
        XCTAssertEqual(function.parameters?.map(\.name), ["source", "length"])
        XCTAssertEqual(function.parameters?.last?.typeText, "float")
        XCTAssertEqual(function.parameters?.last?.defaultText, "14")
    }

    func testMethodsAndExports() throws {
        let f = Fixture("export method area(Shape this) =>\n    this.w\nexport total = 1\n|")
        let area = try XCTUnwrap(f.index.declarations.first { $0.name == "area" })
        XCTAssertEqual(area.kind, .method)
        XCTAssertTrue(area.isExported)
        XCTAssertEqual(area.parameters?.first?.name, "this")
        XCTAssertEqual(Set(f.index.exports.map(\.name)), ["area", "total"])
    }

    func testImportsRecordPathAndAlias() throws {
        let f = Fixture("import user/MyLib/1 as lib\nimport user/Other/2\nimport user/")
        XCTAssertEqual(f.index.imports.map(\.alias), ["lib", "Other", nil])
        XCTAssertEqual(f.index.imports.map(\.path), ["user/MyLib/1", "user/Other/2", "user/"])
        XCTAssertNotNil(f.index.importDeclaration(alias: "lib"))
    }

    func testUserTypesAndEnums() throws {
        let f = Fixture(
            "type Point\n    float x\n    float y = 0\n    Point next\nenum Mode\n    fast\n    slow = \"Slow\"\n|")
        let point = try XCTUnwrap(f.index.types["Point"])
        XCTAssertEqual(point.fields.map(\.name), ["x", "y", "next"])
        XCTAssertEqual(point.fields.last?.typeName, "Point")
        XCTAssertEqual(point.fields[1].defaultText, "0")
        XCTAssertEqual(f.index.enums["Mode"]?.members, ["fast", "slow"])
        XCTAssertTrue(Set(f.visibleNames()).isSuperset(of: ["Point", "Mode"]))
        XCTAssertFalse(f.visibleNames().contains("x"), "fields are not variables")
    }

    func testVariableTypesComeFromAnnotationsAndConstructors() throws {
        let f = Fixture("Point p = na\nq = Point.new(1, 2)\nr = 3\n|")
        XCTAssertEqual(f.index.declarations.first { $0.name == "p" }?.typeName, "Point")
        XCTAssertEqual(f.index.declarations.first { $0.name == "q" }?.typeName, "Point")
        XCTAssertNil(f.index.declarations.first { $0.name == "r" }?.typeName)
    }

    func testTypesDoNotShadowBuiltins() {
        let f = Fixture("type close\n    float x\nplot(close)")
        XCTAssertFalse(f.isUserDefined(at: (f.text as NSString).range(of: "close)").location))
    }

    func testEnclosingFunction() {
        let f = Fixture("foo(a) =>\n    |")
        XCTAssertEqual(f.index.enclosingFunction(of: f.scope)?.name, "foo")
        XCTAssertNil(Fixture("x = 1\n|").index.enclosingFunction(of: 0))
    }

    func testIndexWorksOnEmptyAndOddSources() {
        XCTAssertEqual(Fixture("").index.declarations.count, 0)
        XCTAssertEqual(Fixture("\n\n   \n").index.scopes.count, 1)
        _ = Fixture("((((\n  [[[\n=> =>\nfor\nimport\ntype\n")
        _ = Fixture("日本語 = 1\n😀 = 2\nplot(日本語)")
    }
}
