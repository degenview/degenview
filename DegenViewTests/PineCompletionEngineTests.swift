import XCTest

@testable import DegenView

final class PineCompletionEngineTests: XCTestCase {
    private let testLibrary = StubLibraryExports(exports: [
        "user/TestLib/1": [
            PineLibraryExport(
                name: "myEMA", kind: .function,
                parameters: [
                    .init(name: "source", typeText: "float", defaultText: nil),
                    .init(name: "length", typeText: "int", defaultText: "14"),
                ]),
            PineLibraryExport(name: "ratio", kind: .variable, parameters: nil),
            PineLibraryExport(name: "Band", kind: .type, parameters: nil),
        ]
    ])

    // MARK: Builtins

    func testBuiltinMemberPrefix() {
        let fixture = PineCompletionFixture("plot(ta.rs|)")
        let labels = fixture.labels()
        XCTAssertTrue(labels.contains("rsi"))
        XCTAssertTrue(labels.allSatisfy { $0.lowercased().hasPrefix("rs") }, "\(labels)")
        XCTAssertFalse(labels.contains("close"))
    }

    func testRsiRowCarriesItsSignature() throws {
        let rsi = try XCTUnwrap(PineCompletionFixture("ta.rs|").item("rsi"))
        XCTAssertEqual(rsi.kind, .function)
        XCTAssertEqual(rsi.style, .callable)
        XCTAssertEqual(rsi.detail, "ta.rsi(source, length) → series float")
        XCTAssertEqual(rsi.documentation, "Relative Strength Index.")
        XCTAssertEqual(rsi.origin, .builtin)
    }

    func testNamespaceMembersAreExactlyThatNamespace() {
        for namespace in ["ta", "math", "str", "color", "strategy", "barstate", "syminfo", "timeframe", "plot"] {
            let items = PineCompletionFixture("\(namespace).|").items()
            XCTAssertFalse(items.isEmpty, namespace)
            let expected = Set(PineSymbolCatalog.members(of: namespace).map(\.label))
            XCTAssertEqual(Set(items.map(\.label)), expected, namespace)
        }
    }

    func testEveryCatalogNamespaceCompletes() {
        for member in PineSymbolCatalog.members(of: "") where member.kind == .namespace {
            XCTAssertFalse(PineCompletionFixture("\(member.label).|").items().isEmpty, member.label)
        }
    }

    func testGlobalVariable() throws {
        let fixture = PineCompletionFixture("plot(clo|)")
        let close = try XCTUnwrap(fixture.item("close"))
        XCTAssertEqual(close.kind, .variable)
        XCTAssertEqual(close.detail, "series float")
        XCTAssertEqual(close.documentation, "Closing price of the current bar; the last price on the realtime bar.")
        XCTAssertTrue(PineCompletionFixture("plot(op|)").labels().contains("open"))
        XCTAssertTrue(PineCompletionFixture("plot(bar_|)").labels().contains("bar_index"))
    }

    func testNamedConstants() throws {
        let red = try XCTUnwrap(PineCompletionFixture("color.r|").item("red"))
        XCTAssertEqual(red.kind, .constant)
        XCTAssertEqual(red.style, .identifier)
        XCTAssertTrue(PineCompletionFixture("plot(1, style = plot.style_l|)").labels().isEmpty == false)
    }

    func testNamespacesAreOfferedAsNamespaces() throws {
        let ta = try XCTUnwrap(PineCompletionFixture("x = t|").item("ta"))
        XCTAssertEqual(ta.kind, .namespace)
        XCTAssertEqual(ta.style, .namespace)
        let globals = PineCompletionFixture("x = |", explicit: true).labels()
        XCTAssertFalse(globals.contains("rsi"), "members are not globals")
    }

    func testNothingUnimplementedIsOffered() {
        let labels = PineCompletionFixture("x = |", explicit: true).labels()
        for name in ["fixnan", "plotarrow", "plotbar", "weekofyear"] { XCTAssertFalse(labels.contains(name), name) }
        XCTAssertFalse(PineCompletionFixture("syminfo.|").labels().contains("session"))
    }

    // MARK: Unknown bases

    func testUnknownAndValueBasesOfferNothing() {
        XCTAssertEqual(PineCompletionFixture("somethingUnknown.|").labels(), [])
        XCTAssertEqual(PineCompletionFixture("close.|").labels(), [])
        XCTAssertEqual(PineCompletionFixture("ta.nothing.|").labels(), [])
        XCTAssertEqual(PineCompletionFixture("f().|").labels(), [])
    }

    // MARK: User symbols

    func testUserVariable() throws {
        let fixture = PineCompletionFixture("myAverage = ta.sma(close, 20)\nplot(myA|)")
        let item = try XCTUnwrap(fixture.item("myAverage"))
        XCTAssertEqual(item.kind, .variable)
        XCTAssertEqual(item.origin, .user)
        XCTAssertEqual(item.tier, .scriptGlobal)
    }

    func testSeveralVariablesRankByName() {
        let source = """
            fastLength = input.int(10)
            slowLength = input.int(20)
            fastEMA = ta.ema(close, fastLength)
            plot(fast|)
            """
        XCTAssertEqual(PineCompletionFixture(source).labels(), ["fastEMA", "fastLength"])
    }

    func testUserFunction() throws {
        let fixture = PineCompletionFixture("myAverage(source, length) =>\n    ta.sma(source, length)\n\nvalue = myA|")
        let item = try XCTUnwrap(fixture.item("myAverage"))
        XCTAssertEqual(item.kind, .function)
        XCTAssertEqual(item.style, .callable)
        XCTAssertEqual(item.origin, .user)
        XCTAssertEqual(item.detail, "myAverage(source, length)")
        XCTAssertEqual(item.signatures.first?.parameters.map(\.name), ["source", "length"])
    }

    func testParametersRankAheadOfGlobals() throws {
        let fixture = PineCompletionFixture("myFunc(source, length) =>\n    ta.sma(s|)")
        let items = fixture.items()
        XCTAssertEqual(items.first?.label, "source")
        XCTAssertEqual(items.first?.kind, .parameter)
        XCTAssertEqual(items.first?.tier, .innermostScope)
        XCTAssertTrue(items.dropFirst().contains { $0.origin == .builtin })
    }

    func testLocalsAreVisibleOnlyInsideTheirFunction() {
        let source = "foo() =>\n    fooLocal = close\n    fooLo|\n\nbar() =>\n    barLocal = close\n"
        let inside = PineCompletionFixture(source)
        XCTAssertTrue(inside.labels().contains("fooLocal"))
        XCTAssertEqual(inside.item("fooLocal")?.kind, .local)
        let afterFoo = PineCompletionFixture("foo() =>\n    fooLocal = close\n\nbarLo|\nbar() =>\n    barLocal = 1")
        XCTAssertFalse(afterFoo.labels().contains("fooLocal"))
        let inFoo = PineCompletionFixture(source.replacingOccurrences(of: "fooLo|", with: "barLo|"))
        XCTAssertFalse(inFoo.labels().contains("barLocal"))
        XCTAssertFalse(PineCompletionFixture("foo() =>\n    fooLocal = close\n\nfooLo|").labels().contains("fooLocal"))
    }

    func testUserDeclarationHidesTheBuiltinOfTheSameName() throws {
        let fixture = PineCompletionFixture("close = 123\nplot(clo|)")
        let matches = fixture.items().filter { $0.label == "close" }
        XCTAssertEqual(matches.count, 1)
        XCTAssertEqual(matches.first?.origin, .user)
        XCTAssertEqual(matches.first?.kind, .variable)
    }

    func testAUserVariableHidesABuiltinNamespace() {
        XCTAssertEqual(PineCompletionFixture("ta = 1\nta.|").labels(), [])
        let labels = PineCompletionFixture("ta = 1\nplot(t|)").items().filter { $0.label == "ta" }
        XCTAssertEqual(labels.count, 1)
        XCTAssertEqual(labels.first?.origin, .user)
    }

    func testLoopVariablesAndShadowing() throws {
        let source = "value = 1\nfoo(value) =>\n    for i = 0 to 3\n        va|"
        let items = PineCompletionFixture(source).items().filter { $0.label == "value" }
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.kind, .parameter)
        XCTAssertEqual(PineCompletionFixture("for i = 0 to 3\n    i|").item("i")?.kind, .local)
    }

    // MARK: Member ranking

    func testUserNamesNeverLandBetweenMembers() {
        let fixture = PineCompletionFixture("rsi = 1\nrunning = 2\nta.r|")
        let labels = fixture.labels()
        XCTAssertEqual(labels, labels.sorted { $0.lowercased() < $1.lowercased() })
        XCTAssertFalse(labels.contains("running"))
        XCTAssertTrue(labels.contains("rsi"))
        XCTAssertEqual(fixture.item("rsi")?.origin, .builtin)
    }

    func testRankingPutsExactAndCaseMatchesFirst() {
        let source = "Close = 1\nclosed = 2\nplot(close|)"
        let labels = PineCompletionFixture(source).labels()
        XCTAssertEqual(Array(labels.prefix(3)), ["close", "Close", "closed"], "exact, same case first")
    }

    func testResultsHaveUniqueLabels() {
        let labels = PineCompletionFixture("x = |", explicit: true).labels()
        XCTAssertEqual(labels.count, Set(labels).count)
    }

    // MARK: Keywords and types

    func testKeywordsAppearOnlyWhereTheyBelong() {
        XCTAssertTrue(PineCompletionFixture("i|").labels().contains("if"))
        XCTAssertTrue(PineCompletionFixture("im|").labels().contains("import"))
        XCTAssertFalse(PineCompletionFixture("x = i|").labels().contains("if"))
        XCTAssertTrue(PineCompletionFixture("x = tr|").labels().contains("true"))
        XCTAssertFalse(PineCompletionFixture("tr|").labels().contains("true"))
    }

    func testTypesAtStatementStartAndAfterVar() {
        XCTAssertEqual(PineCompletionFixture("fl|").item("float")?.kind, .type)
        let afterVar = PineCompletionFixture("var |", explicit: true)
        XCTAssertTrue(afterVar.items().allSatisfy { $0.kind == .type })
        XCTAssertTrue(afterVar.labels().contains("float"))
        XCTAssertEqual(PineCompletionFixture("type Point\n    float x\nPo|").item("Point")?.kind, .type)
    }

    // MARK: Declarations and incomplete code

    func testNamingADeclarationOffersNothing() {
        XCTAssertEqual(PineCompletionFixture("float my|").labels(), [])
        XCTAssertEqual(PineCompletionFixture("for |", explicit: true).labels(), [])
    }

    func testCommentsAndStringsOfferNothing() {
        XCTAssertEqual(PineCompletionFixture("// ta.rs|").labels(), [])
        XCTAssertEqual(PineCompletionFixture("x = \"ta.rs|\"").labels(), [])
        XCTAssertEqual(PineCompletionFixture("label.new(text=\"ta.rs|\")").labels(), [])
    }

    func testIncompleteSourceStillCompletes() {
        XCTAssertTrue(PineCompletionFixture("x = ta.rs|\nif\nfoo(").labels().contains("rsi"))
        XCTAssertTrue(PineCompletionFixture("foo(\nx = 1\ny = cl|").labels().contains("close"))
        XCTAssertTrue(PineCompletionFixture("x = 1 +\nplot(clo|").labels().contains("close"))
        XCTAssertTrue(PineCompletionFixture("a = (1\nb = 2\nplot(|)", explicit: true).labels().contains("b"))
    }

    // MARK: Libraries

    func testImportedLibraryMembers() throws {
        let fixture = PineCompletionFixture("import user/TestLib/1 as test\n\nx = test.|")
        let items = fixture.items(libraries: testLibrary)
        XCTAssertEqual(items.map(\.label), ["Band", "myEMA", "ratio"])
        let ema = try XCTUnwrap(items.first { $0.label == "myEMA" })
        XCTAssertEqual(ema.kind, .libraryMember)
        XCTAssertEqual(ema.origin, .library(alias: "test"))
        XCTAssertEqual(ema.detail, "myEMA(source, length)")
        XCTAssertEqual(ema.style, .callable)
        XCTAssertEqual(items.first { $0.label == "ratio" }?.style, .identifier)
        XCTAssertEqual(items.first { $0.label == "Band" }?.style, .namespace)
    }

    func testImportedMemberPrefix() {
        let fixture = PineCompletionFixture("import user/TestLib/1 as lib\nx = lib.my|")
        XCTAssertEqual(fixture.labels(libraries: testLibrary), ["myEMA"])
    }

    func testAnAliasIsOfferedAndAnUnknownLibraryHasNoMembers() throws {
        let fixture = PineCompletionFixture("import user/TestLib/1\nx = Test|")
        XCTAssertEqual(fixture.item("TestLib", libraries: testLibrary)?.kind, .library)
        XCTAssertEqual(PineCompletionFixture("import user/Missing/1 as m\nx = m.|").labels(libraries: testLibrary), [])
        XCTAssertEqual(PineCompletionFixture("import user/TestLib/1 as m\nx = m.|").labels(), [], "no libraries known")
    }

    func testExportedTypeOffersItsConstructor() {
        let fixture = PineCompletionFixture("import user/TestLib/1 as lib\nx = lib.Band.|")
        XCTAssertEqual(fixture.labels(libraries: testLibrary), ["new"])
    }

    func testImportPathCompletion() {
        XCTAssertEqual(PineCompletionFixture("import |", explicit: true).labels(libraries: testLibrary), ["user"])
        XCTAssertEqual(PineCompletionFixture("import user/|").labels(libraries: testLibrary), ["TestLib"])
        XCTAssertEqual(PineCompletionFixture("import user/Te|").labels(libraries: testLibrary), ["TestLib"])
        XCTAssertEqual(PineCompletionFixture("import user/Nope|").labels(libraries: testLibrary), [])
    }

    // MARK: User types and enums

    func testUserTypeFieldsAndMethods() {
        let source = """
            type Point
                float x
                float y = 0
            method scaled(Point this, float k) =>
                this.x * k
            p = Point.new(1, 2)
            plot(p.|)
            """
        let items = PineCompletionFixture(source).items()
        XCTAssertEqual(Set(items.map(\.label)), ["x", "y", "scaled"])
        XCTAssertEqual(items.first { $0.label == "x" }?.kind, .field)
        XCTAssertEqual(items.first { $0.label == "scaled" }?.detail, "scaled(k)", "the receiver is not written")
    }

    func testAnAnnotatedVariableAndAParameterCompleteTheirFields() {
        XCTAssertEqual(
            PineCompletionFixture("type Point\n    float x\nPoint q = na\nplot(q.|)").labels(), ["x"])
        XCTAssertEqual(
            PineCompletionFixture("type Point\n    float x\nf(Point p) =>\n    p.|").labels(), ["x"])
    }

    func testTypeOffersNewAndEnumOffersMembers() {
        XCTAssertEqual(PineCompletionFixture("type Point\n    float x\nPoint.|").labels(), ["new"])
        let enumeration = PineCompletionFixture("enum Mode\n    fast\n    slow\nx = Mode.|")
        XCTAssertEqual(enumeration.labels(), ["fast", "slow"])
        XCTAssertEqual(enumeration.item("fast")?.kind, .enumMember)
    }

    // MARK: Named arguments

    func testNamedArgumentsOfTheCallUnderTheCaret() throws {
        let fixture = PineCompletionFixture("plot(close, ti|)")
        let title = try XCTUnwrap(fixture.item("title"))
        XCTAssertEqual(title.kind, .argumentName)
        XCTAssertEqual(title.style, .argumentName)
        XCTAssertEqual(title.detail, "title: const string = \"\"")
        XCTAssertFalse(fixture.labels().contains("series"), "the first parameter is taken by the positional close")
    }

    func testSuppliedNamesAreNotOfferedAgain() {
        XCTAssertFalse(PineCompletionFixture("plot(close, title = \"x\", ti|)").labels().contains("title"))
        XCTAssertTrue(PineCompletionFixture("plot(close, title = \"x\", co|)").labels().contains("color"))
    }

    func testNoNamedArgumentsAfterAnExpressionHasStarted() {
        XCTAssertFalse(PineCompletionFixture("plot(close + ti|)").labels().contains("title"))
        XCTAssertFalse(PineCompletionFixture("plot(close, title = ti|)").items().contains { $0.kind == .argumentName })
    }

    func testNamedArgumentsOfUserFunctionsAndConstructors() {
        let function = PineCompletionFixture("foo(alpha, beta = 2) =>\n    alpha\nfoo(1, be|)")
        XCTAssertTrue(function.labels().contains("beta"))
        let constructor = PineCompletionFixture("type Point\n    float x\n    float y\np = Point.new(1, y|)")
        XCTAssertTrue(constructor.labels().contains("y"))
    }

    // MARK: Triggers

    func testTriggerPolicy() {
        func opens(_ marked: String, typed: String) -> Bool {
            PineCompletionTrigger.opens(for: .typed(typed), in: PineCompletionFixture(marked).context)
        }
        XCTAssertTrue(opens("ta.|", typed: "."))
        XCTAssertTrue(opens("ta.r|", typed: "r"))
        XCTAssertFalse(opens("c|", typed: "c"), "one letter is below the threshold")
        XCTAssertTrue(opens("cl|", typed: "l"))
        XCTAssertFalse(opens("x = |", typed: " "))
        XCTAssertFalse(opens("// ta.r|", typed: "r"))
        XCTAssertFalse(opens("plot(|)", typed: "("))
        XCTAssertTrue(opens("import user/|", typed: "/"))
        XCTAssertTrue(PineCompletionTrigger.opens(for: .explicit, in: PineCompletionFixture("x = |").context))
        XCTAssertFalse(PineCompletionTrigger.opens(for: .explicit, in: PineCompletionFixture("// x|").context))
        XCTAssertFalse(PineCompletionTrigger.opens(for: .deleted, in: PineCompletionFixture("cl|").context))
    }

    func testAnAutomaticListOfTheWordJustTypedIsNotShown() {
        let fixture = PineCompletionFixture("x = bar_index|")
        XCTAssertFalse(
            PineCompletionTrigger.shouldPresent(fixture.items(), for: .typed("x"), in: fixture.context))
        XCTAssertTrue(
            PineCompletionTrigger.shouldPresent(fixture.items(), for: .explicit, in: fixture.context))
    }
}
