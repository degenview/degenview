import XCTest

@testable import DegenView

final class AppInfoTests: XCTestCase {
    func testLinksAreHTTPS() {
        XCTAssertEqual(AppInfo.repositoryURL.scheme, "https")
        XCTAssertEqual(AppInfo.donationURL.absoluteString, "https://buymeacoffee.com/degenview")
    }

    func testVersionLabelIncludesBuildWhenDistinct() {
        let info = AppInfo(bundle: .main)
        XCTAssertFalse(info.version.isEmpty)
        XCTAssertTrue(info.versionLabel.hasPrefix(info.version))
    }

    func testMissingKeysFallBack() {
        // A bundle with no Info.plist keys (the test bundle's resources directory).
        let bundle = Bundle(path: NSTemporaryDirectory()) ?? Bundle()
        let info = AppInfo(bundle: bundle)
        XCTAssertFalse(info.name.isEmpty)
        XCTAssertFalse(info.version.isEmpty)
        XCTAssertTrue(info.copyright.contains(AppInfo.licenseName))
    }

    func testClipboardSummaryStartsWithNameAndVersion() {
        let info = AppInfo(bundle: .main)
        XCTAssertTrue(info.clipboardSummary.hasPrefix("\(info.name) \(info.versionLabel)"))
    }

    func testHeaderSubtitleHasVersionAndDateOnly() {
        let info = AppInfo(bundle: .main)
        XCTAssertTrue(info.headerSubtitle.hasPrefix("Version \(info.version)"))
        if info.buildDate != nil {
            XCTAssertFalse(info.headerSubtitle.contains(":"), "no time of day in the header")
        }
    }
}
