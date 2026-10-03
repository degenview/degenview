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

    private func script(_ name: String, favorite: Bool) -> LocalScript {
        LocalScript(
            id: UUID(), name: name, type: .indicator, source: "", latestRevisionID: nil,
            createdAt: Date(), modifiedAt: Date(), lastOpenedAt: nil, isFavorite: favorite,
            compileRecord: nil)
    }

    func testFavoritedScriptSelectsOnlyTheClickedRow() throws {
        let model = ScriptManagerViewModel(defaults: defaults)
        let starred = script("Starred", favorite: true)
        model.scripts = [starred, script("Other", favorite: false)]
        let rows = model.groups.flatMap(\.rows).filter { $0.script.id == starred.id }
        XCTAssertEqual(rows.count, 2)

        let typeRow = try XCTUnwrap(rows.last)
        model.selectRow(typeRow.id)
        XCTAssertEqual(model.selection, starred.id)
        XCTAssertEqual(model.selectedRowID, typeRow.id)

        let favoritesRow = try XCTUnwrap(rows.first)
        model.selectRow(favoritesRow.id)
        XCTAssertEqual(model.selectedRowID, favoritesRow.id)
    }

    func testSelectedRowFollowsScriptWhenItsRowDisappears() throws {
        let model = ScriptManagerViewModel(defaults: defaults)
        var starred = script("Starred", favorite: true)
        model.scripts = [starred]
        let favoritesRow = try XCTUnwrap(model.groups.first?.rows.first)
        XCTAssertTrue(favoritesRow.id.hasPrefix("favorites#"))
        model.selectRow(favoritesRow.id)

        starred.isFavorite = false
        model.scripts = [starred]
        model.query = "star"  // any change re-reconciles
        XCTAssertEqual(model.selectedRowID, model.groups.first?.rows.first?.id)
        XCTAssertFalse(try XCTUnwrap(model.selectedRowID).hasPrefix("favorites#"))
    }

    func testSelectingByScriptHighlightsOneRow() {
        let model = ScriptManagerViewModel(defaults: defaults)
        let starred = script("Starred", favorite: true)
        model.scripts = [starred]
        model.selection = starred.id
        XCTAssertEqual(model.selectedRowID, model.groups.flatMap(\.rows).first?.id)
    }
}
