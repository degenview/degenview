import XCTest

@testable import DegenView

/// Guards that the catalog keeps up with what the compiler and runtime actually implement.
final class PineSymbolCatalogTests: XCTestCase {
    func testEveryRuntimeFunctionNameIsKnown() {
        for name in PineRuntimeSession.exactHandlers.keys {
            if let head = name.split(separator: ".").first, name.contains(".") {
                XCTAssertTrue(PineSymbolCatalog.namespaces.contains(String(head)), name)
            } else {
                XCTAssertTrue(PineSymbolCatalog.functions.contains(name), name)
            }
        }
        for name in PineRuntimeSession.visualNames {
            XCTAssertTrue(PineSymbolCatalog.functions.contains(name), name)
        }
        for entry in PineRuntimeSession.namespaceHandlers {
            XCTAssertTrue(PineSymbolCatalog.namespaces.contains(String(entry.prefix.dropLast())))
        }
    }

    func testEveryRuntimeValueNameIsKnown() {
        for name in PineBarFlags.names {
            XCTAssertNotNil(PineBarFlags().value(named: name), name)
            XCTAssertTrue(PineSymbolCatalog.variables.contains(name), name)
        }
        for name in PineTime.timeframeNames {
            XCTAssertNotNil(PineTime.timeframe(name, barSeconds: 60), name)
            XCTAssertTrue(PineSymbolCatalog.variables.contains(name), name)
        }
        for name in PineTime.partNames {
            XCTAssertTrue(PineSymbolCatalog.variables.contains(name), name)
        }
        for name in PineBuiltinTypes.floatSeries.union(PineBuiltinTypes.intSeries) {
            XCTAssertTrue(PineSymbolCatalog.variables.contains(name), name)
        }
    }

    func testEveryConstantTableEntryIsKnown() {
        for name in PineBuiltins.colors.keys {
            XCTAssertTrue(PineSymbolCatalog.constants.contains(name), name)
        }
        for name in PineBuiltins.constants.keys {
            XCTAssertTrue(PineSymbolCatalog.constants.contains(name), name)
        }
        for name in PineSize.pineNames + PineLineStyle.pineNames + PineMarkerShape.pineNames {
            XCTAssertTrue(PineSymbolCatalog.constants.contains(name), name)
        }
        XCTAssertTrue(PineSymbolCatalog.constants.contains("size.tiny"))
        XCTAssertTrue(PineSymbolCatalog.constants.contains("line.style_dotted"))
    }

    func testKindsDoNotOverlap() {
        XCTAssertTrue(PineSymbolCatalog.variables.isDisjoint(with: PineSymbolCatalog.constants))
        XCTAssertTrue(
            PineSymbolCatalog.reservedWords.isDisjoint(with: PineSymbolCatalog.variables))
    }

    func testParserWordsAreKnown() {
        XCTAssertEqual(PineSymbolCatalog.qualifiers, ["const", "input", "simple", "series"])
        XCTAssertTrue(PineSymbolCatalog.objectTypes.isSuperset(of: ["line", "label", "box", "table", "array"]))
    }
}
