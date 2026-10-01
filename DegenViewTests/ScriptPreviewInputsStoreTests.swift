import XCTest

@testable import DegenView

@MainActor
final class ScriptPreviewInputsStoreTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ScriptPreviewInputsStoreTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func makeStore() -> ScriptPreviewInputsStore {
        ScriptPreviewInputsStore(store: JSONStore(filename: "inputs.json", directory: directory))
    }

    func testValuesSurviveANewInstance() {
        let script = UUID()
        let values: [String: PineInputValue] = [
            "len": .int(5), "mult": .float(2.5), "on": .bool(false), "tint": .color(0xFF00_00FF),
            "src": .source("high"), "mode": .string("fast"),
        ]
        makeStore().setInputs(values, for: script)

        XCTAssertEqual(makeStore().inputs(for: script), values)
    }

    func testEmptyInputsForgetTheScript() {
        let script = UUID()
        let store = makeStore()
        store.setInputs(["len": .int(5)], for: script)
        store.setInputs([:], for: script)

        XCTAssertEqual(makeStore().inputs(for: script), [:])
    }

    func testPruneDropsDeletedScripts() {
        let kept = UUID()
        let deleted = UUID()
        let store = makeStore()
        store.setInputs(["a": .int(1)], for: kept)
        store.setInputs(["b": .int(2)], for: deleted)

        store.prune(keeping: [kept])

        let reloaded = makeStore()
        XCTAssertEqual(reloaded.inputs(for: kept), ["a": .int(1)])
        XCTAssertEqual(reloaded.inputs(for: deleted), [:])
    }
}
