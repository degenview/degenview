import XCTest

@testable import DegenView

/// Runs real-world TradingView scripts through the engine. The sources are third-party and
/// never committed: `tools/pine-corpus/fetch.py` caches them in `.pine-corpus/`, and the test
/// skips when that cache is missing. `expectations.json` records what each script is known to
/// produce, so a regression and a newly supported script both show up as a mismatch.
final class PineCorpusTests: XCTestCase {
    private struct ManifestEntry: Decodable {
        let slug: String
        let name: String
        let kind: String
        let status: String
    }

    private struct Expectation: Codable, Equatable {
        /// `ok`, `compile-error`, or `runtime-error`.
        var status: String
        /// Error diagnostic codes, sorted and unique. Empty for `ok`.
        var codes: [String]
    }

    /// Written when `PINE_CORPUS_REPORT` names a file: the observed outcomes, plus each error's
    /// line and message for triage.
    private struct Report: Encodable {
        let expectations: [String: Expectation]
        let details: [String: [String]]
    }

    private static let barCount = 1_000
    private static let repositoryRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
    private static let corpusDirectory = repositoryRoot.appendingPathComponent("DegenViewTests/PineCorpus")

    private var cacheDirectory: URL {
        ProcessInfo.processInfo.environment["PINE_CORPUS_DIR"].map { URL(fileURLWithPath: $0) }
            ?? Self.repositoryRoot.appendingPathComponent(".pine-corpus")
    }

    func testCorpusMatchesExpectations() throws {
        let manifest = try JSONDecoder().decode(
            [ManifestEntry].self,
            from: Data(contentsOf: Self.corpusDirectory.appendingPathComponent("manifest.json")))
        let expectations = try JSONDecoder().decode(
            [String: Expectation].self,
            from: Data(contentsOf: Self.corpusDirectory.appendingPathComponent("expectations.json")))
        let fetched = manifest.filter { $0.status == "fetched" }
        try XCTSkipIf(
            !FileManager.default.fileExists(atPath: cacheDirectory.path),
            "No corpus cache; run tools/pine-corpus/fetch.py")

        let bars = Self.syntheticBars(count: Self.barCount)
        var observed: [String: Expectation] = [:]
        var details: [String: [String]] = [:]
        var table: [String] = []
        for entry in fetched {
            let url = cacheDirectory.appendingPathComponent("\(entry.kind)/\(entry.slug).pine")
            let source = try String(contentsOf: url, encoding: .utf8)
            let start = Date()
            let (outcome, messages) = Self.run(source, bars: bars)
            let milliseconds = Int(Date().timeIntervalSince(start) * 1000)
            observed[entry.slug] = outcome
            details[entry.slug] = messages
            table.append(
                "\(entry.slug.padding(toLength: 10, withPad: " ", startingAt: 0)) "
                    + "\(entry.kind.padding(toLength: 9, withPad: " ", startingAt: 0)) "
                    + "\(outcome.status.padding(toLength: 14, withPad: " ", startingAt: 0)) "
                    + "\(String(milliseconds).padding(toLength: 6, withPad: " ", startingAt: 0))ms "
                    + outcome.codes.joined(separator: ","))
        }
        print("Pine corpus (\(fetched.count) scripts, \(Self.barCount) bars)\n" + table.joined(separator: "\n"))
        if let path = ProcessInfo.processInfo.environment["PINE_CORPUS_REPORT"] {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(Report(expectations: observed, details: details))
                .write(to: URL(fileURLWithPath: path))
        }

        for entry in fetched {
            let expected = expectations[entry.slug] ?? Expectation(status: "ok", codes: [])
            XCTAssertEqual(
                observed[entry.slug], expected, "\(entry.slug) (\(entry.name)) differs from expectations.json")
        }
    }

    /// Compiles `source` and, when it is valid, runs it over `bars` with default inputs.
    private static func run(_ source: String, bars: [KlineData]) -> (Expectation, [String]) {
        let program = PineCompiler.compile(source: source)
        let compileErrors = errors(in: program.diagnostics)
        guard compileErrors.isEmpty else {
            return (.init(status: "compile-error", codes: codes(of: compileErrors)), describe(compileErrors))
        }
        do {
            let result = try PineRuntimeSession(program: program).evaluate(bars: bars)
            let runtimeErrors = errors(in: result.diagnostics)
            return runtimeErrors.isEmpty
                ? (.init(status: "ok", codes: []), [])
                : (.init(status: "runtime-error", codes: codes(of: runtimeErrors)), describe(runtimeErrors))
        } catch let diagnostic as PineDiagnostic {
            return (.init(status: "runtime-error", codes: [diagnostic.code]), describe([diagnostic]))
        } catch {
            return (.init(status: "runtime-error", codes: ["\(type(of: error))"]), ["\(error)"])
        }
    }

    private static func errors(in diagnostics: [PineDiagnostic]) -> [PineDiagnostic] {
        diagnostics.filter { $0.severity == .error }
    }

    private static func codes(of diagnostics: [PineDiagnostic]) -> [String] {
        Array(Set(diagnostics.map(\.code))).sorted()
    }

    private static func describe(_ diagnostics: [PineDiagnostic]) -> [String] {
        diagnostics.map { "\($0.range.start.line):\($0.range.start.column) \($0.code) \($0.message)" }
    }

    /// A deterministic random walk (fixed-seed LCG), so a failure reproduces on every machine.
    private static func syntheticBars(count: Int, spacing: TimeInterval = 3_600) -> [KlineData] {
        var state: UInt64 = 0x2545_F491_4F6C_DD1D
        func next() -> Double {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Double(state >> 11) / Double(1 << 53)
        }
        var close = 100.0
        return (0..<count).map { index in
            let open = close
            close = max(1, open * (1 + (next() - 0.5) * 0.04))
            let high = max(open, close) * (1 + next() * 0.01)
            let low = min(open, close) * (1 - next() * 0.01)
            return KlineData(
                openTime: Date(timeIntervalSince1970: 1_700_000_000 + Double(index) * spacing),
                openPrice: open, highPrice: high, lowPrice: low, closePrice: close,
                volume: 100 + next() * 900, isClosed: true)
        }
    }
}
