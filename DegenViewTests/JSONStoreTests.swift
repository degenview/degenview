import XCTest
@testable import DegenView

final class JSONStoreTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("JSONStoreTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    func testMissingAndRoundTripAreDistinguished() throws {
        let store = JSONStore<[Int]>(filename: "values.json", directory: directory)
        if case .missing = store.loadResult() {} else { XCTFail("Expected a missing file") }

        try store.saveThrowing([1, 2, 3])
        guard case .value(let value) = store.loadResult() else { return XCTFail("Expected decoded data") }
        XCTAssertEqual(value, [1, 2, 3])
    }

    func testCorruptFileIsPreservedBeforeRecovery() throws {
        let instant = Date(timeIntervalSince1970: 1_777_777_777)
        let store = JSONStore<[Int]>(filename: "tabs.json", directory: directory, now: { instant })
        let original = Data("not-json".utf8)
        try original.write(to: store.storageURL)

        guard case .quarantined(let backupURL) = store.loadResult() else {
            return XCTFail("Expected corrupt data to be quarantined")
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.storageURL.path))
        XCTAssertEqual(try Data(contentsOf: backupURL), original)
        XCTAssertTrue(backupURL.lastPathComponent.hasPrefix("tabs.corrupt-"))

        try store.saveThrowing([42])
        XCTAssertEqual(store.load(), [42])
    }

    func testQuarantineNameCollisionGetsNumericSuffix() throws {
        let instant = Date(timeIntervalSince1970: 1_777_777_777)
        let first = JSONStore<[Int]>(filename: "tabs.json", directory: directory, now: { instant })
        try Data("bad-one".utf8).write(to: first.storageURL)
        guard case .quarantined(let firstURL) = first.loadResult() else { return XCTFail() }

        try Data("bad-two".utf8).write(to: first.storageURL)
        guard case .quarantined(let secondURL) = first.loadResult() else { return XCTFail() }
        XCTAssertNotEqual(firstURL, secondURL)
        XCTAssertTrue(secondURL.deletingPathExtension().lastPathComponent.hasSuffix("-1"))
    }

    @MainActor
    func testTabsStoreQuarantinesCorruptionBeforeWritingRecoveredSession() throws {
        let store = JSONStore<TabsSnapshot>(
            filename: "tabs.json",
            directory: directory,
            now: { Date(timeIntervalSince1970: 1_777_777_777) }
        )
        let original = Data("broken-session".utf8)
        try original.write(to: store.storageURL)
        let suiteName = "JSONStoreTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!

        let tabsStore = TabsStore(store: store, userDefaults: defaults, supportDirectory: directory)

        XCTAssertEqual(tabsStore.tabs.count, 1)
        XCTAssertNotNil(store.load())
        let backups = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.hasPrefix("tabs.corrupt-") }
        XCTAssertEqual(backups.count, 1)
        XCTAssertEqual(try Data(contentsOf: backups[0]), original)
        defaults.removePersistentDomain(forName: suiteName)
    }
}
