import XCTest

@testable import DegenView

/// Guards that the editor's signature table and member lists agree with what the compiler and
/// runtime actually implement: nothing is offered that DegenView cannot run.
final class PineSymbolMetadataTests: XCTestCase {
    /// Builtins that are callable but have no signature of their own: handle-type casts.
    private static let unsignedFunctions: Set<String> = ["line", "label", "box", "table"]

    func testEveryEntryParses() {
        XCTAssertEqual(PineSymbolMetadata.rejectedLines, [])
    }

    func testEntryNamesBelongToTheCatalog() {
        for name in PineSymbolMetadata.table.keys {
            if let head = name.split(separator: ".").first, name.contains(".") {
                XCTAssertTrue(PineSymbolCatalog.namespaces.contains(String(head)), name)
            } else {
                XCTAssertTrue(PineSymbolCatalog.functions.contains(name), name)
            }
            XCTAssertFalse(PineSymbolCatalog.isUnimplementedFunction(name), name)
        }
        for name in PineSymbolMetadata.variableSummaries.keys {
            let known = PineSymbolCatalog.variables.contains(name) || PineSymbolCatalog.constants.contains(name)
            XCTAssertTrue(known, name)
        }
    }

    func testEveryRuntimeFunctionHasASignature() {
        for name in PineRuntimeSession.exactHandlers.keys where !Self.unsignedFunctions.contains(name) {
            XCTAssertNotNil(PineSymbolMetadata.table[name], "\(name) needs a signature entry")
        }
        for name in PineTA.supported {
            XCTAssertNotNil(PineSymbolMetadata.table[name], "\(name) needs a signature entry")
        }
    }

    func testEveryCatalogNameIsListedUnderItsNamespace() {
        var names = PineSymbolCatalog.variables.union(PineSymbolCatalog.constants)
        names.formUnion(PineRuntimeSession.exactHandlers.keys)
        names.formUnion(PineSymbolMetadata.table.keys)
        for name in names where !PineSymbolCatalog.isUnimplementedVariable(name) {
            guard !PineSymbolCatalog.isUnimplementedFunction(name) else { continue }
            let parts = name.split(separator: ".").map(String.init)
            let parent = parts.dropLast().joined(separator: ".")
            let label = parts.last ?? name
            XCTAssertTrue(
                PineSymbolCatalog.members(of: parent).contains { $0.label == label },
                "\(name) is missing from members(of: \"\(parent)\")")
            XCTAssertTrue(PineSymbolCatalog.isCompletable(name) || PineSymbolCatalog.kind(of: name) == nil)
        }
    }

    func testUnimplementedNamesAreNeverOffered() {
        var reserved = ["fixnan", "plotarrow", "plotbar", "weekofyear", "session.ismarket", "syminfo.session"]
        reserved.append("syminfo.description")
        for name in reserved {
            XCTAssertFalse(PineSymbolCatalog.isCompletable(name), name)
            let parts = name.split(separator: ".").map(String.init)
            let parent = parts.dropLast().joined(separator: ".")
            XCTAssertFalse(
                PineSymbolCatalog.members(of: parent).contains { $0.label == parts.last },
                name)
        }
    }

    /// Every signature names something the runtime really dispatches: calling it never reports an
    /// unknown function. Other errors (a missing argument) are fine; this is only the "can DegenView
    /// run this name" question.
    func testNoSignatureNamesAnUnsupportedFunction() {
        let skipped: Set<String> = ["indicator", "strategy", "library"]
        for name in PineSymbolMetadata.table.keys.sorted() where !skipped.contains(name) {
            let header = name.hasPrefix("strategy.") ? "strategy(\"T\")" : "indicator(\"T\")"
            let program = PineCompiler.compile(source: "//@version=6\n\(header)\nx = \(name)()")
            guard program.isValid else { continue }
            do {
                _ = try PineRuntimeSession(program: program).evaluate(bars: PineExecutionFixtures.history([1, 2, 3]))
            } catch {
                XCTAssertNotEqual((error as? PineDiagnostic)?.code, "PINE4007", "\(name) is not implemented")
            }
        }
    }

    func testReturnTypesAgreeWithTheTypeChecker() {
        for (name, signatures) in PineSymbolMetadata.table {
            let inferred = PineBuiltinTypes.call(name, arguments: [], values: [], qualifier: nil)
            guard let type = inferred.type else { continue }
            let word = String(describing: type)
            XCTAssertTrue(
                signatures.contains { $0.returns.contains(word) },
                "\(name) returns \(signatures.map(\.returns)) but the type checker says \(word)")
        }
    }

    func testSignatureShape() throws {
        let rsi = try XCTUnwrap(PineSymbolMetadata.table["ta.rsi"]?.first)
        XCTAssertEqual(rsi.label, "ta.rsi(source, length) → series float")
        XCTAssertEqual(rsi.parameters.map(\.type), ["series float", "simple int"])
        XCTAssertEqual(rsi.requiredCount, 2)
        XCTAssertNotNil(rsi.summary)

        let change = try XCTUnwrap(PineSymbolMetadata.table["ta.change"]?.first)
        XCTAssertEqual(change.parameters.last?.defaultValue, "1")
        XCTAssertEqual(change.requiredCount, 1)

        let macd = try XCTUnwrap(PineSymbolMetadata.table["ta.macd"]?.first)
        XCTAssertEqual(macd.returns, "[series float, series float, series float]")
    }

    func testOverloadsStayAsSeparateSignatures() {
        XCTAssertEqual(PineSymbolMetadata.table["ta.highest"]?.count, 2)
        XCTAssertEqual(PineSymbolMetadata.table["timestamp"]?.count, 2)
        XCTAssertEqual(PineSymbolMetadata.table["math.round"]?.map(\.returns), ["series int", "series float"])
    }

    func testMembersOfANamespace() {
        let ta = PineSymbolCatalog.members(of: "ta")
        XCTAssertEqual(ta.first { $0.label == "rsi" }?.kind, .function)
        XCTAssertEqual(ta.first { $0.label == "tr" }?.kind, .variable, "a variable and a function: the variable wins")
        XCTAssertTrue(ta.allSatisfy { $0.qualifiedName.hasPrefix("ta.") })
        XCTAssertEqual(ta.map(\.label), ta.map(\.label).sorted())
        XCTAssertFalse(ta.contains { $0.label == "close" })

        let color = PineSymbolCatalog.members(of: "color")
        XCTAssertEqual(color.first { $0.label == "red" }?.kind, .constant)
        XCTAssertEqual(color.first { $0.label == "new" }?.kind, .function)

        XCTAssertEqual(PineSymbolCatalog.members(of: "chart").first { $0.label == "point" }?.kind, .namespace)
        XCTAssertEqual(
            PineSymbolCatalog.members(of: "chart.point").map(\.label),
            ["from_index", "from_time", "new", "now"])
        XCTAssertEqual(PineSymbolCatalog.members(of: "nothing"), [])
        XCTAssertEqual(PineSymbolCatalog.members(of: "close"), [])
    }

    func testRootMembers() {
        let root = PineSymbolCatalog.members(of: "")
        func kind(_ label: String) -> PineCatalogSymbolKind? { root.first { $0.label == label }?.kind }
        XCTAssertEqual(kind("close"), .variable)
        XCTAssertEqual(kind("time"), .variable)
        XCTAssertEqual(kind("nz"), .function)
        XCTAssertEqual(kind("plot"), .function, "plot stays callable although plot.style_* exist")
        XCTAssertEqual(kind("hline"), .function)
        XCTAssertEqual(kind("ta"), .namespace)
        XCTAssertEqual(kind("color"), .namespace)
        XCTAssertNil(kind("log"), "a namespace with no members is not offered")
        XCTAssertNil(kind("fixnan"))
        XCTAssertNil(kind("rsi"), "namespace members are not globals")
    }

    func testAddingACatalogNameSurfacesItWithoutAnotherList() {
        // The members come from the same tables the runtime dispatches on.
        for name in PineRuntimeSession.exactHandlers.keys where name.hasPrefix("ta.") {
            XCTAssertTrue(PineSymbolCatalog.members(of: "ta").contains { $0.qualifiedName == name }, name)
        }
        for name in PineBuiltins.colors.keys {
            XCTAssertTrue(PineSymbolCatalog.members(of: "color").contains { $0.qualifiedName == name }, name)
        }
    }

    func testKeywordAndTypeSetsComeFromTheCatalog() {
        XCTAssertTrue(PineSymbolCatalog.statementKeywords.contains("if"))
        XCTAssertTrue(PineSymbolCatalog.typeNames.isSuperset(of: PineSymbolCatalog.objectTypes))
        XCTAssertTrue(PineSymbolCatalog.typeNames.contains("float"))
    }
}
