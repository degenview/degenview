import AppKit
import XCTest

@testable import DegenView

@MainActor
final class WindowTabIconTests: XCTestCase {
    private func attachmentCount(_ string: NSAttributedString) -> Int {
        var count = 0
        string.enumerateAttribute(.attachment, in: NSRange(location: 0, length: string.length)) { value, _, _ in
            if value != nil { count += 1 }
        }
        return count
    }

    func testEachTabKindHasItsOwnIcon() {
        XCTAssertEqual(WindowTabIcon(kind: .charts), .charts)
        XCTAssertEqual(WindowTabIcon(kind: .portfolio), .portfolio)
        XCTAssertEqual(Set(WindowTabIcon.allCases.map(\.symbolName)).count, WindowTabIcon.allCases.count)
        for icon in WindowTabIcon.allCases {
            XCTAssertNotNil(NSImage(systemSymbolName: icon.symbolName, accessibilityDescription: nil), icon.symbolName)
        }
    }

    func testTheTitleIsPrecededByExactlyOneIcon() {
        let label = WindowTabIcon.charts.attributedTitle("BTC")

        XCTAssertEqual(attachmentCount(label), 1)
        XCTAssertTrue(label.string.hasSuffix("BTC"))
        XCTAssertTrue(label.string.hasPrefix("\u{FFFC}"), "The attachment leads")
    }

    func testAnEmptyTitleStillShowsTheIcon() {
        let label = WindowTabIcon.portfolio.attributedTitle("")

        XCTAssertEqual(attachmentCount(label), 1)
        XCTAssertEqual(label.string.trimmingCharacters(in: .whitespaces), "\u{FFFC}")
    }

    func testRenamingAWindowKeepsItsIcon() {
        let window = NSWindow(
            contentRect: .init(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled], backing: .buffered,
            defer: false)
        window.title = "Unnamed"
        WindowTabDecorator.decorate(window, with: .charts)
        XCTAssertEqual(attachmentCount(window.tab.attributedTitle ?? NSAttributedString()), 1)

        window.title = "Altcoins"

        let label = window.tab.attributedTitle ?? NSAttributedString()
        XCTAssertEqual(attachmentCount(label), 1)
        XCTAssertTrue(label.string.hasSuffix("Altcoins"))
    }

    func testWindowsDecorateIndependently() {
        func makeWindow(_ title: String, _ icon: WindowTabIcon) -> NSWindow {
            let window = NSWindow(
                contentRect: .init(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled], backing: .buffered,
                defer: false)
            window.title = title
            WindowTabDecorator.decorate(window, with: icon)
            return window
        }
        let charts = makeWindow("Unnamed", .charts)
        let portfolio = makeWindow("Portfolio", .portfolio)

        charts.title = "Majors"

        XCTAssertTrue((charts.tab.attributedTitle?.string ?? "").hasSuffix("Majors"))
        XCTAssertTrue((portfolio.tab.attributedTitle?.string ?? "").hasSuffix("Portfolio"))
    }
}
