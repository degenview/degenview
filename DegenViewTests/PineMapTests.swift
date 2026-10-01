import XCTest

@testable import DegenView

/// `map<K, V>`: construction, access, ordering, iteration, persistence and rollback.
final class PineMapTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private func bars(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: 1)
        }
    }

    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
    }

    private func plots(_ body: String, bars series: [Double] = [1]) throws -> [[Double?]] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(series)).output.plots.map(\.values)
    }

    private func runtimeError(_ body: String) -> String? {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        do {
            _ = try PineRuntimeSession(program: program).evaluate(bars: bars([1]))
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    // MARK: - Operations

    func testPutGetContainsRemoveAndSize() throws {
        let result = try plots(
            """
            var map<string, float> m = map.new<string, float>()
            map.put(m, "a", 1.5)
            m.put("b", 2.5)
            m.put("a", 9.0)
            plot(m.get("a"))
            plot(m.get("b"))
            plot(na(m.get("zzz")) ? 1 : 0)
            plot(m.contains("b") ? 1 : 0)
            plot(m.remove("b"))
            plot(m.contains("b") ? 1 : 0)
            plot(m.size())
            """)
        XCTAssertEqual(result.map { $0[0] }, [9, 2.5, 1, 1, 2.5, 0, 1])
    }

    func testKeysAndValuesKeepInsertionOrderAndOverwritesKeepTheirSlot() throws {
        let result = try plots(
            """
            var map<int, float> m = map.new<int, float>()
            m.put(30, 3.0)
            m.put(10, 1.0)
            m.put(20, 2.0)
            m.put(10, 1.5)
            keys = m.keys()
            vals = m.values()
            plot(array.get(keys, 0))
            plot(array.get(keys, 1))
            plot(array.get(keys, 2))
            plot(array.get(vals, 1))
            """)
        XCTAssertEqual(result.map { $0[0] }, [30, 10, 20, 1.5])
    }

    func testWholeFloatAndIntKeysAreTheSameKey() throws {
        let result = try plots(
            """
            var map<float, string> m = map.new<float, string>()
            m.put(2, "two")
            m.put(2.5, "half")
            plot(str.length(m.get(2.0)))
            plot(str.length(m.get(2.5)))
            plot(m.size())
            """)
        XCTAssertEqual(result.map { $0[0] }, [3, 4, 2])
    }

    func testBoolAndColorKeys() throws {
        let result = try plots(
            """
            var map<bool, int> flags = map.new<bool, int>()
            flags.put(true, 1)
            flags.put(false, 0)
            var map<color, int> shades = map.new<color, int>()
            shades.put(color.red, 5)
            plot(flags.get(true))
            plot(shades.get(color.red))
            """)
        XCTAssertEqual(result.map { $0[0] }, [1, 5])
    }

    func testClearCopyAndPutAll() throws {
        let result = try plots(
            """
            var map<int, int> a = map.new<int, int>()
            a.put(1, 10)
            copy = a.copy()
            copy.put(2, 20)
            merged = map.new<int, int>()
            merged.put(0, 5)
            merged.put_all(copy)
            plot(a.size())
            plot(copy.size())
            plot(merged.size())
            a.clear()
            plot(a.size())
            """)
        XCTAssertEqual(result.map { $0[0] }, [1, 2, 3, 0])
    }

    func testForInOverPairsFollowsInsertionOrder() throws {
        let result = try plots(
            """
            var map<int, float> m = map.new<int, float>()
            m.put(3, 30.0)
            m.put(1, 10.0)
            float keySum = 0.0
            float firstValue = na
            for [k, v] in m
                keySum += k
                if na(firstValue)
                    firstValue := v
            plot(keySum)
            plot(firstValue)
            """)
        XCTAssertEqual(result.map { $0[0] }, [4, 30])
    }

    // MARK: - Types and fields

    func testMapsHoldObjectsAndLiveInFields() throws {
        let result = try plots(
            """
            type Level
                float price

            type Book
                map<int, Level> levels

            Book book = Book.new(map.new<int, Level>())
            book.levels.put(7, Level.new(42.0))
            Level found = book.levels.get(7)
            plot(found.price)
            plot(book.levels.size())
            """)
        XCTAssertEqual(result.map { $0[0] }, [42, 1])
    }

    func testNestedGenericTypesParse() {
        let program = compile(
            """
            var map<string, array<float>> series = map.new<string, array<float>>()
            map<int, chart.point> points = map.new<int, chart.point>()
            f(map<string, float> m) => m.size()
            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }

    // MARK: - Errors

    func testUsingANaMapOrAnNaKeyIsARuntimeError() {
        XCTAssertEqual(runtimeError("map<int, int> m = na\nplot(map.get(m, 1))"), "PINE4026")
        XCTAssertEqual(
            runtimeError("var map<int, int> m = map.new<int, int>()\nm.put(na, 1)\nplot(close)"), "PINE4027")
    }

    // MARK: - State

    func testMapPersistsAcrossBarsAndRollsBackWithRealtimeTicks() throws {
        let controller = F.controller(
            """
            var map<int, int> seen = map.new<int, int>()
            seen.put(bar_index, 1)
            plot(seen.size())
            """)
        _ = F.update(controller.rebuild(bars: F.history([1, 2, 3]), live: false))
        let ticks = [(4.0, false), (5.0, false), (6.0, true)].compactMap { close, closed in
            F.update(controller.ingest(F.stream(F.bar(3, open: 4, close: close, closed: closed))))
        }
        XCTAssertEqual(ticks.map { F.last($0.output) }, [4, 4, 4], "each tick starts from three committed keys")
        let next = try XCTUnwrap(F.update(controller.ingest(F.stream(F.bar(4, open: 6, close: 7)))))
        XCTAssertEqual(F.last(next.output), 5)
    }
}
