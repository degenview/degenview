import XCTest

@testable import DegenView

final class ChartSettingsSheetTests: XCTestCase {
    func testDecimalModeRoundTripsEveryPrecision() {
        XCTAssertEqual(ChartSettingsSheet.DecimalMode.from(nil), .auto)
        XCTAssertNil(ChartSettingsSheet.DecimalMode.auto.intValue)
        for places in 0...8 {
            let mode = ChartSettingsSheet.DecimalMode.from(places)
            XCTAssertEqual(mode.intValue, places)
        }
    }

    func testAnOutOfRangePrecisionFallsBackToAuto() {
        XCTAssertEqual(ChartSettingsSheet.DecimalMode.from(9), .auto)
        XCTAssertEqual(ChartSettingsSheet.DecimalMode.from(-1), .auto)
    }

    func testSettingsOfferNoTickerTab() {
        XCTAssertEqual(ChartSettingsSheet.Tab.allCases.map(\.rawValue), ["Indicators", "Scripts", "Appearance"])
    }
}
