import XCTest

@testable import DegenView

@MainActor
final class ScriptManagerViewModelTests: XCTestCase {
    private var defaults: UserDefaults!
    private var suite: String!

    override func setUp() {
        suite = "ScriptManagerViewModelTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    func testSessionRoundTrips() {
        let session = ScriptManagerSession(isOpen: true, anchorTabID: UUID(), wasSelected: true)
        session.save(to: defaults)
        XCTAssertEqual(ScriptManagerSession.load(from: defaults), session)
    }

    func testSessionIsNilWhenNothingSaved() {
        XCTAssertNil(ScriptManagerSession.load(from: defaults))
    }

    func testSelectionBeforeFirstLoadDoesNotOverwriteStoredID() {
        let stored = UUID()
        defaults.set(stored.uuidString, forKey: ScriptManagerViewModel.selectionKey)
        let model = ScriptManagerViewModel(defaults: defaults)
        model.selection = nil
        XCTAssertEqual(defaults.string(forKey: ScriptManagerViewModel.selectionKey), stored.uuidString)
    }
}
