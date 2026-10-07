import XCTest

@testable import DegenView

/// Script-defined `type`s: parsing, construction, fields, mutation and methods.
final class PineUserTypeTests: XCTestCase {
    private func bars(_ closes: [Double], spacing: TimeInterval = 60) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * spacing), openPrice: c,
                highPrice: c + 1, lowPrice: c - 1, closePrice: c, volume: Double(i + 1))
        }
    }

    private func compile(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func codes(_ program: PineCompiledProgram) -> [String] { program.diagnostics.map(\.code) }

    // MARK: - Parsing

    func testTypeDeclarationParsesFieldsDefaultsAndNestedTypes() throws {
        let program = compile(
            """
            type Point
                float x = 0.0
                float y

            type Zone
                Point origin
                array<float> history
                string label = "zone"
                int bars
                Zone next

            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let declarations = program.statements.compactMap { statement -> (String, [PineTypeField])? in
            if case .typeDeclaration(let name, let fields, _) = statement { return (name, fields) }
            return nil
        }
        XCTAssertEqual(declarations.map(\.0), ["Point", "Zone"])
        XCTAssertEqual(declarations[1].1.map(\.name), ["origin", "history", "label", "bars", "next"])
        XCTAssertEqual(declarations[1].1.map(\.type), [.object, .array, .string, .int, .object])
        XCTAssertNotNil(declarations[1].1[2].defaultValue)
    }

    func testDeclaredTypeNameWorksInDeclarationsAndParameters() {
        let program = compile(
            """
            type Zone
                float top

            var Zone z = na
            f(Zone zone, array<Zone> all) =>
                zone.top
            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }

    func testTypeAndEnumRemainUsableAsVariableNames() {
        XCTAssertTrue(compile("type = 1\nenum = type + 1\nplot(enum)").isValid)
    }

    func testAnUndeclaredTypeNameIsNotADeclaration() {
        XCTAssertFalse(compile("Zone z = na\nplot(close)").isValid)
    }

    func testFieldWithoutANameIsADiagnostic() {
        let program = compile("type Zone\n    float\nplot(close)")
        XCTAssertTrue(codes(program).contains("PINE2015"), "\(codes(program))")
    }

    func testTypeWithoutFieldsBlockIsADiagnostic() {
        XCTAssertTrue(codes(compile("type Zone\nplot(close)")).contains("PINE2002"))
    }

    func testExportedTypeIsAcceptedInALibrary() {
        let source = "//@version=6\nlibrary(\"L\")\nexport type Zone\n    float top\n"
        XCTAssertTrue(PineCompiler.compile(source: source).isValid)
        XCTAssertTrue(codes(compile("export type Zone\n    float top\n")).contains("PINE3037"))
    }

    // MARK: - Objects

    private func values(_ body: String, bars series: [Double] = [1]) throws -> [[Double?]] {
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

    func testConstructorTakesPositionalNamedAndDefaultFields() throws {
        let plots = try values(
            """
            type Zone
                float top
                float bottom = 2.0 * 3
                int bars = 4
                string label

            a = Zone.new(1.5, 2.5, 3)
            b = Zone.new(bars = 9, top = 7.0)
            c = Zone.new()
            plot(a.top)
            plot(a.bottom)
            plot(a.bars)
            plot(b.top)
            plot(b.bottom)
            plot(b.bars)
            plot(na(c.top) ? 1 : 0)
            plot(c.bars)
            plot(na(c.label) ? 1 : 0)
            """)
        XCTAssertEqual(plots.map { $0[0] }, [1.5, 2.5, 3, 7, 6, 9, 1, 4, 1])
    }

    func testVarObjectIsCreatedOnceAndPersists() throws {
        let plots = try values(
            """
            type Anchor
                int born

            var Anchor anchor = Anchor.new(bar_index)
            plot(anchor.born)
            """, bars: [1, 2, 3])
        XCTAssertEqual(plots, [[0, 0, 0]])
    }

    func testNestedObjectsAndHandlesAreReachableThroughFieldPaths() throws {
        let plots = try values(
            """
            type Point
                float x
                float y

            type Marker
                Point origin
                line ln

            m = Marker.new(Point.new(1.0, 2.0), line.new(0, 1.0, 1, 2.0))
            m.ln.set_y2(9.0)
            plot(m.origin.y)
            """)
        XCTAssertEqual(plots, [[2]])
        let program = compile(
            """
            type Marker
                line ln

            m = Marker.new(line.new(0, 1.0, 1, 2.0))
            m.ln.set_y2(9.0)
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.lines.first?.y2, 9)
    }

    func testObjectsStoredInAnArrayComeBackAsObjects() throws {
        let plots = try values(
            """
            type Zone
                float top

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, Zone.new(5.0))
            array.push(zones, Zone.new(8.0))
            Zone last = array.get(zones, 1)
            plot(last.top)
            plot(array.size(zones))
            """)
        XCTAssertEqual(plots, [[8], [2]])
    }

    func testReadingAFieldOfNaIsARuntimeError() {
        let error = runtimeError(
            """
            type Zone
                float top

            Zone z = na
            plot(z.top)
            """)
        XCTAssertEqual(error, "PINE4018")
    }

    func testUnknownFieldAndUnknownConstructorArgumentAreRuntimeErrors() {
        let header = "type Zone\n    float top\n\n"
        XCTAssertEqual(runtimeError(header + "z = Zone.new(1.0)\nplot(z.bottom)"), "PINE4019")
        XCTAssertEqual(runtimeError(header + "z = Zone.new(bottom = 1.0)\nplot(close)"), "PINE4019")
    }

    func testMethodsOnNonObjectsAndUnknownMembersStillFail() {
        let header = "type Zone\n    float top\n\n"
        XCTAssertEqual(runtimeError(header + "z = Zone.new(1.0)\nplot(z.nothing())"), "PINE4007")
    }

    // MARK: - Field assignment

    func testFieldAssignmentAndCompoundAssignmentUpdateTheObject() throws {
        let plots = try values(
            """
            type Counter
                int n
                float total = 1.0

            var Counter c = Counter.new(0)
            c.n += 1
            c.total *= 2.0
            c.total := c.total + 0.5
            plot(c.n)
            plot(c.total)
            """, bars: [1, 2, 3])
        XCTAssertEqual(plots, [[1, 2, 3], [2.5, 5.5, 11.5]])
    }

    func testObjectsAreSharedByReferenceAndCopyIsIndependent() throws {
        let plots = try values(
            """
            type Zone
                float top

            a = Zone.new(1.0)
            b = a
            b.top := 5.0
            c = a.copy()
            c.top := 9.0
            plot(a.top)
            plot(c.top)
            """)
        XCTAssertEqual(plots, [[5], [9]])
    }

    func testMutationThroughAnArrayElementIsVisibleEverywhere() throws {
        let plots = try values(
            """
            type Zone
                float top

            z = Zone.new(1.0)
            var array<Zone> zones = array.new<Zone>()
            array.push(zones, z)
            Zone same = array.get(zones, 0)
            same.top := 9.0
            plot(z.top)
            """)
        XCTAssertEqual(plots, [[9]])
    }

    func testNestedFieldAssignment() throws {
        let plots = try values(
            """
            type Point
                float x

            type Marker
                Point origin

            m = Marker.new(Point.new(1.0))
            m.origin.x := 4.0
            plot(m.origin.x)
            """)
        XCTAssertEqual(plots, [[4]])
    }

    func testAssigningAnUnknownFieldOrThroughNaIsARuntimeError() {
        let header = "type Zone\n    float top\n\n"
        XCTAssertEqual(runtimeError(header + "z = Zone.new(1.0)\nz.bottom := 1.0\nplot(close)"), "PINE4019")
        XCTAssertEqual(runtimeError(header + "Zone z = na\nz.top := 1.0\nplot(close)"), "PINE4018")
        XCTAssertEqual(runtimeError(header + "x = 1.0\nx.top := 1.0\nplot(close)"), "PINE4020")
    }

    func testRealtimeTicksRollBackFieldMutations() throws {
        typealias F = PineExecutionFixtures
        let controller = F.controller(
            """
            type Counter
                int n

            var Counter c = Counter.new(0)
            c.n += 1
            plot(c.n)
            """)
        let closes = (0..<5).map { Double($0) + 1 }
        _ = F.update(controller.rebuild(bars: F.history(closes), live: false))
        // Three ticks of realtime bar 5: each runs on the committed state, so n is 6 every time.
        let updates = [105.0, 106, 107].enumerated().compactMap { offset, close in
            F.update(controller.ingest(F.stream(F.bar(5, open: 105, close: close, closed: offset == 2))))
        }
        XCTAssertEqual(updates.map { F.last($0.output) }, [6, 6, 6])
        let next = try XCTUnwrap(F.update(controller.ingest(F.stream(F.bar(6, open: 107, close: 108)))))
        XCTAssertEqual(F.last(next.output), 7)
    }

    func testAssignmentTargetsThatAreNotFieldsStayOrdinarySyntaxErrors() {
        XCTAssertTrue(compile("ta.ema(close, 3) = 1\nplot(close)").diagnostics.contains { $0.severity == .error })
        XCTAssertTrue(compile("x = ta.ema(close, 3)\nplot(x)").isValid)
    }

    // MARK: - Field access on expressions

    func testFieldReadAfterACallAndInsideAnExpression() throws {
        let plots = try values(
            """
            type Zone
                float top
                int side

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, Zone.new(5.0, 1))
            array.push(zones, Zone.new(8.0, -1))
            n = array.size(zones)
            plot(array.get(zones, n - 1).top)
            plot(array.get(zones, 0).side == 1 and array.get(zones, 1).side == -1 ? 1 : 0)
            plot(array.get(zones, 0).top * 2 + array.get(zones, 1).top)
            """)
        XCTAssertEqual(plots, [[8], [1], [18]])
    }

    func testFieldAssignmentThroughACallResult() throws {
        let plots = try values(
            """
            type Zone
                float top

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, Zone.new(5.0))
            array.get(zones, 0).top := 7.0
            array.get(zones, 0).top += 1.0
            plot(array.get(zones, 0).top)
            """)
        XCTAssertEqual(plots, [[8]])
    }

    func testFieldReadOfNaFromACallIsARuntimeError() {
        let error = runtimeError(
            """
            type Zone
                float top

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, na)
            plot(array.get(zones, 0).top)
            """)
        XCTAssertEqual(error, "PINE4018")
    }

    func testAMemberOnALiteralIsNotAnObjectField() {
        XCTAssertEqual(runtimeError("plot((1.5).top)"), "PINE4018")
    }

    // MARK: - Methods

    func testMethodReceivesTheObjectAndCanMutateIt() throws {
        let plots = try values(
            """
            type Counter
                int n

            method bump(Counter this, int by = 1) =>
                this.n += by

            var Counter c = Counter.new(0)
            c.bump()
            c.bump(by = 2)
            plot(c.n)
            """, bars: [1, 2])
        XCTAssertEqual(plots, [[3, 6]])
    }

    func testMethodReturnsAValueAndWorksOnAnArrayElement() throws {
        let plots = try values(
            """
            type Zone
                float top
                float bottom

            method height(Zone this) =>
                this.top - this.bottom

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, Zone.new(10.0, 4.0))
            Zone first = array.get(zones, 0)
            plot(first.height())
            plot(height(first))
            """)
        XCTAssertEqual(plots, [[6], [6]])
    }

    func testAMethodWinsOnlyForReceiversItsFirstParameterAccepts() throws {
        let plots = try values(
            """
            type Box
                float v

            method size(Box this) => this.v * 100

            b = Box.new(2.0)
            arr = array.from(1.0, 2.0, 3.0)
            plot(b.size())
            plot(arr.size())
            """)
        XCTAssertEqual(plots, [[200], [3]], "the array still gets the builtin size()")
    }

    func testAMethodCanBeDefinedForSeveralReceiverTypes() throws {
        let plots = try values(
            """
            type Circle
                float r

            type Square
                float side

            method area(Circle this) => this.r * 3
            method area(Square this) => this.side * this.side
            method area(array<float> this) => array.sum(this)
            method area(float this) => this * 2

            plot(Circle.new(2.0).area())
            plot(Square.new(4.0).area())
            plot(array.from(1.0, 2.0).area())
            plot((1.5).area())
            plot(area(Circle.new(1.0)))
            plot(area(Square.new(3.0)))
            """)
        XCTAssertEqual(plots, [[6], [16], [3], [3], [3], [9]])
    }

    func testDefiningAMethodTwiceForTheSameReceiverIsStillAnError() {
        let program = compile(
            """
            type Zone
                float top

            method grow(Zone this) => this.top += 1
            method grow(Zone this) => this.top += 2
            plot(close)
            """)
        XCTAssertTrue(codes(program).contains("PINE3024"))
        let functions = compile("f(float x) => x\nf(int x) => x\nplot(close)")
        XCTAssertTrue(codes(functions).contains("PINE3024"), "only methods may be overloaded")
    }

    func testNoDefinitionForTheReceiverIsAnError() {
        XCTAssertEqual(
            runtimeError(
                """
                type A
                    float v

                type B
                    float v

                method f(A this) => this.v
                method f(B this) => this.v + 1
                plot(f(5.0))
                """), "PINE4029")
    }

    func testMethodsOnEnumsAndPrimitivesPickByType() throws {
        let plots = try values(
            """
            enum Mode
                fast
                slow

            method label(Mode this) => this == Mode.fast ? 1 : 2
            method label(string this) => 10

            plot(Mode.slow.label())
            plot("x".label())
            """)
        XCTAssertEqual(plots, [[2], [10]])
    }

    func testExportedMethodIsAcceptedInALibrary() {
        let source =
            "//@version=6\nlibrary(\"L\")\nexport type Zone\n    float top\n\nexport method grow(Zone this) =>\n    this.top += 1\n"
        XCTAssertTrue(PineCompiler.compile(source: source).isValid)
    }

    // MARK: - Methods on call results

    func testMethodOnACallResultCallsUserMethodsAndBuiltins() throws {
        let plots = try values(
            """
            type Counter
                int n

            method bump(Counter this, int by = 1) =>
                this.n += by

            var array<Counter> counters = array.new<Counter>()
            array.push(counters, Counter.new(0))
            array.get(counters, 0).bump()
            array.get(counters, 0).bump(by = 4)
            var array<line> lines = array.new<line>()
            array.push(lines, line.new(0, 1.0, 1, 2.0))
            array.get(lines, 0).set_y2(9.0)
            plot(array.get(counters, 0).n)
            plot(array.get(lines, 0).get_y2())
            """)
        XCTAssertEqual(plots, [[5], [9]])
    }

    func testMethodOnAFieldOfACallResult() throws {
        let plots = try values(
            """
            type Holder
                array<float> xs

            var map<int, Holder> holders = map.new<int, Holder>()
            holders.put(1, Holder.new(array.from(3.0, 4.0)))
            plot(holders.get(1).xs.get(1))
            holders.get(1).xs.push(8.0)
            plot(holders.get(1).xs.size())
            """)
        XCTAssertEqual(plots, [[4], [3]])
    }

    func testMethodOnANaOrUnsupportedReceiverIsARuntimeError() {
        let header = "type Zone\n    float top\n\nvar array<Zone> zones = array.new<Zone>()\n"
        let grow = header + "array.push(zones, na)\narray.get(zones, 0).grow()\nplot(close)"
        XCTAssertEqual(runtimeError(grow), "PINE4018")
        XCTAssertEqual(runtimeError("plot((1.5).foo())"), "PINE4007")
        XCTAssertEqual(
            runtimeError(header + "array.push(zones, Zone.new(1.0))\narray.get(zones, 0).grow()\nplot(close)"),
            "PINE4007")
    }
}
