import XCTest

@testable import DegenView

/// Completion is an editor operation: it reads the cached analysis and the builtin tables, and
/// never compiles, runs, lexes again, or touches the network or the database.
final class PineCompletionPerformanceTests: XCTestCase {
    /// A script of a few thousand lines with many functions, locals and calls.
    private static let largeSource: String = {
        var lines = ["//@version=6", "indicator(\"Large\", overlay = true)"]
        for index in 0..<300 {
            lines.append("level\(index) = input.int(\(index + 1), \"Level \(index)\")")
            lines.append("smooth\(index)(source, length) =>")
            lines.append("    average = ta.sma(source, length + \(index))")
            lines.append("    spread = ta.stdev(source, length) * \(index % 7 + 1)")
            lines.append("    if average > spread")
            lines.append("        average - spread")
            lines.append("    else")
            lines.append("        average + spread")
            lines.append("line\(index) = smooth\(index)(close, level\(index))")
            lines.append("plot(line\(index), \"Line \(index)\", color.new(color.blue, \(index % 90)))")
        }
        lines.append("result = ")
        return lines.joined(separator: "\n")
    }()

    /// The large script with `tail` typed after `result = `, analysed once.
    private struct Request {
        let analysis: PineEditorAnalysisSnapshot
        let selection: NSRange
        let cache: PineEditorAnalysisCache

        init(_ tail: String) {
            let fixture = PineEditorFixture(
                PineCompletionPerformanceTests.largeSource.replacingOccurrences(
                    of: "result = ", with: "result = \(tail)"))
            cache = PineEditorAnalysisCache()
            analysis = cache.analysis(for: fixture.text)
            selection = fixture.selection
        }

        var labels: [String] {
            let context = PineCompletionContext(analysis: analysis, selection: selection, explicit: false)
            return PineCompletionEngine.complete(context, analysis: analysis).map(\.label)
        }
    }

    func testCompletionNeverLexesCompilesOrBuildsAgain() {
        let request = Request("smooth29|")
        let lexes = PineLexicalSnapshot.buildCount
        for _ in 0..<20 { XCTAssertFalse(request.labels.isEmpty) }
        XCTAssertEqual(PineLexicalSnapshot.buildCount, lexes)
        XCTAssertEqual(request.cache.buildCount, 1)
    }

    func testGlobalNamespaceAndLocalPrefixesOnALargeScript() {
        let global = Request("smooth29|")
        let member = Request("ta.r|")
        let local = Request("level2|")
        measure {
            for request in [global, member, local] { _ = request.labels }
        }
        XCTAssertTrue(global.labels.contains("smooth291"))
        XCTAssertTrue(member.labels.contains("rsi"))
        XCTAssertTrue(local.labels.contains("level2"))
        XCTAssertTrue(local.labels.contains("level299"))
    }

    func testAnalysingALargeScriptIsOneLexAndOneIndex() {
        let cache = PineEditorAnalysisCache()
        let lexes = PineLexicalSnapshot.buildCount
        _ = cache.analysis(for: Self.largeSource)
        XCTAssertLessThanOrEqual(PineLexicalSnapshot.buildCount - lexes, 1)
        measure { _ = PineEditorAnalysisCache().analysis(for: Self.largeSource + "\n") }
    }

    /// The editor's analysis and completion files may not reach for the compiler, the runtime, the
    /// network or the database. A source-level guard, because those calls have no counters.
    func testEditorAnalysisFilesNeverCompileRunOrReachOut() throws {
        let editor = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("DegenView/Pine/Editor")
        let names = try FileManager.default.contentsOfDirectory(atPath: editor.path).filter {
            $0.hasPrefix("PineCompletion") && !$0.hasPrefix("PineCompletionPanel")
                || $0.hasPrefix("PineSignature") || $0.hasPrefix("PineSourceSymbolIndex")
                || $0.hasPrefix("PineStatementSplitter") || $0.hasPrefix("PineEditorAnalysis")
                || $0.hasPrefix("PineCallSite") || $0.hasPrefix("PineLibraryExport")
        }
        XCTAssertGreaterThan(names.count, 12)
        let forbidden = [
            "PineCompiler.compile", "PineRuntimeSession(", "PineLexer(", "URLSession", "AppDatabase", "GRDB",
            "FileManager", "UserDefaults",
        ]
        for name in names where name.hasSuffix(".swift") {
            let text = try String(contentsOf: editor.appendingPathComponent(name), encoding: .utf8)
            for word in forbidden {
                XCTAssertFalse(text.contains(word), "\(name) must not use \(word)")
            }
        }
    }
}
