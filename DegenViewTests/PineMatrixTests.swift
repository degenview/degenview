import XCTest

@testable import DegenView

/// `matrix<T>`: construction, access, growing and shrinking, and summaries.
final class PineMatrixTests: XCTestCase {
    private func bars(_ count: Int) -> [KlineData] {
        (0..<count).map { i in
            let close = Double(i + 1)
            return .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: close, highPrice: close + 1,
                lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
    }

    private func firstValues(_ body: String) throws -> [Double?] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(1)).output.plots.map {
            $0.values[0]
        }
    }

    private func runtimeError(_ body: String) -> String? {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        do {
            _ = try PineRuntimeSession(program: program).evaluate(bars: bars(1))
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    func testNewGetSetAndDimensions() throws {
        let result = try firstValues(
            """
            matrix<float> m = matrix.new<float>(2, 3, 1.5)
            m.set(1, 2, 9.0)
            matrix.set(m, 0, 0, 4.0)
            plot(m.rows())
            plot(m.columns())
            plot(m.get(0, 0))
            plot(m.get(1, 2))
            plot(matrix.get(m, 0, 1))
            plot(m.elements_count())
            """)
        XCTAssertEqual(result, [2, 3, 4, 9, 1.5, 6])
    }

    func testAddAndRemoveRowsAndColumns() throws {
        let result = try firstValues(
            """
            matrix<float> m = matrix.new<float>(2, 2, 0.0)
            m.set(0, 0, 1.0)
            m.set(1, 1, 4.0)
            m.add_col(0)
            m.add_row(2, array.from(7.0, 8.0, 9.0))
            plot(m.rows())
            plot(m.columns())
            plot(na(m.get(0, 0)) ? 1 : 0)
            plot(m.get(0, 1))
            plot(m.get(2, 2))
            m.remove_col()
            m.remove_row(0)
            plot(m.rows())
            plot(m.columns())
            """)
        XCTAssertEqual(result, [3, 3, 1, 1, 9, 2, 2])
    }

    func testRowAndColumnReturnArraysAndChainWithMethods() throws {
        let result = try firstValues(
            """
            matrix<float> m = matrix.new<float>(2, 3, 0.0)
            for r = 0 to 1
                for c = 0 to 2
                    m.set(r, c, r * 10 + c)
            plot(m.row(1).avg())
            plot(array.size(m.col(2)))
            plot(array.get(m.col(2), 1))
            """)
        XCTAssertEqual(result, [11, 2, 12])
    }

    func testFillCopyTransposeAndSummaries() throws {
        let result = try firstValues(
            """
            matrix<float> m = matrix.new<float>(2, 3, 2.0)
            m.set(0, 1, 8.0)
            copy = m.copy()
            copy.fill(0.0)
            t = m.transpose()
            plot(m.get(0, 1))
            plot(copy.get(0, 1))
            plot(t.rows())
            plot(t.get(1, 0))
            plot(matrix.avg(m))
            plot(matrix.max(m))
            plot(matrix.min(m))
            """)
        XCTAssertEqual(result, [8, 0, 3, 8, 3, 8, 2])
    }

    func testOutOfBoundsAndNaMatricesAreRuntimeErrors() {
        XCTAssertEqual(
            runtimeError("matrix<float> m = matrix.new<float>(1, 1, 0.0)\nplot(m.get(1, 0))"), "PINE4010")
        XCTAssertEqual(
            runtimeError("matrix<float> m = matrix.new<float>(1, 1, 0.0)\nm.add_row(5)\nplot(close)"), "PINE4010")
        XCTAssertEqual(runtimeError("matrix<float> m = na\nplot(matrix.rows(m))"), "PINE4028")
    }

    func testMatrixTypesParseAsVariablesParametersAndFields() {
        let program = compile(
            """
            type Grid
                matrix<float> cells

            var matrix<float> shared = matrix.new<float>(2, 2, 0.0)
            f(matrix<int> m) => m.rows()
            Grid g = Grid.new(matrix.new<float>(1, 1, 0.0))
            plot(g.cells.rows() + shared.rows())
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }
}
