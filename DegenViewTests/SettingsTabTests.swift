import XCTest

@testable import DegenView

final class SettingsTabTests: XCTestCase {
    /// The selected tab is persisted (and written by other screens) under these raw values.
    func testTabRawValuesAreStable() {
        XCTAssertEqual(
            SettingsTab.allCases.map(\.rawValue), ["appearance", "alpaca", "coinMarketCap", "notifications"])
        XCTAssertEqual(SettingsTab(rawValue: "alpaca"), .alpaca)
    }

    func testEveryTabHasATitleAndIcon() {
        for tab in SettingsTab.allCases {
            XCTAssertFalse(tab.title.isEmpty)
            XCTAssertFalse(tab.systemImage.isEmpty)
        }
    }
}
