import AppKit
import SwiftUI
import XCTest

@testable import DegenView

/// The sidebar's geometry depends on padding SwiftUI's `List` adds around every row. These host the real sidebar in an
/// invisible window and measure it, so a system that changes that padding fails here instead of silently
/// misaligning the logos, the flag and the column titles.
@MainActor
final class WatchlistListMetricsTests: XCTestCase {
    private var window: NSWindow!
    private var host: NSView!
    private var outline: NSOutlineView!
    private var store: WatchlistStore!

    private func item(_ symbol: String) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: .binance, symbol: symbol), name: symbol, label: symbol)
    }

    private func find(_ view: NSView) -> NSOutlineView? {
        if let outline = view as? NSOutlineView { return outline }
        for sub in view.subviews { if let found = find(sub) { return found } }
        return nil
    }

    override func setUp() async throws {
        store = WatchlistStore(database: try AppDatabase.makeInMemory())
        let list = try XCTUnwrap(store.favorites)
        try store.addInstrument(item("AAA"), to: list.id)
        try store.addInstrument(item("BBB"), to: list.id)
        try store.setFlag(.red, for: InstrumentID(source: .binance, symbol: "AAA"))
        let coordinator = WatchlistQuoteCoordinator(
            provider: LiveWatchlistQuoteProvider.shared, book: WatchlistQuoteBook(), runsLoop: false)
        let viewModel = WatchlistSidebarViewModel(store: store, quotes: WatchlistQuoteBook(), coordinator: coordinator)
        let sidebar = WatchlistSidebar(
            viewModel: viewModel,
            actions: WatchlistInstrumentActions(open: { _ in }, addChart: { _ in }, openInNewTab: { _ in }),
            isWindowVisible: false, focusedMarket: nil, onAddSymbol: {})
        let hosting = NSHostingView(rootView: sidebar.frame(width: 300, height: 400))
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 400), styleMask: [.titled], backing: .buffered,
            defer: false)
        window.contentView = hosting
        window.alphaValue = 0
        window.orderBack(nil)
        host = hosting
        for _ in 0..<20 {
            hosting.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
        outline = try XCTUnwrap(find(hosting))
    }

    override func tearDown() async throws {
        window?.orderOut(nil)
    }

    func testTheListPadsEachRowByTheAmountTheMetricsAssume() {
        let cell = outline.frameOfCell(atColumn: 0, row: 0)
        XCTAssertEqual(cell.minX, WatchlistMetrics.listCellLeading, accuracy: 0.6)
        XCTAssertEqual(outline.bounds.width - cell.maxX, WatchlistMetrics.listCellTrailing, accuracy: 0.6)
    }

    func testRowContentLandsOnTheLeadingAndTrailingInsets() {
        let cell = outline.frameOfCell(atColumn: 0, row: 0)
        let content = cell.minX + WatchlistMetrics.rowLeadingInset
        let end = cell.maxX - WatchlistMetrics.rowTrailingInset
        XCTAssertEqual(content, WatchlistMetrics.leadingInset, accuracy: 0.6, "logos start on the leading inset")
        XCTAssertEqual(outline.bounds.width - end, WatchlistMetrics.trailingInset, accuracy: 0.6, "values end on the trailing inset")
    }

    /// The colour at (x, mid-height) of a row, drawn from its layer tree.
    private func colour(row: Int, x: Int) throws -> NSColor {
        let rowView = try XCTUnwrap(outline.rowView(atRow: row, makeIfNecessary: false))
        let width = Int(rowView.bounds.width)
        let height = Int(rowView.bounds.height)
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let context = try XCTUnwrap(
            CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        rowView.layer?.render(in: context)
        let image = try XCTUnwrap(context.makeImage())
        let rep = NSBitmapImageRep(cgImage: image)
        return try XCTUnwrap(rep.colorAt(x: x, y: height / 2)?.usingColorSpace(.sRGB))
    }

    func testAFlagTouchesTheLeftBorderOfItsRow() throws {
        let flagged = try colour(row: 0, x: 1)
        XCTAssertGreaterThan(flagged.redComponent, 0.7, "the flag's red is drawn on the border: \(flagged)")
        XCTAssertLessThan(flagged.greenComponent, 0.5)
        let plain = try colour(row: 1, x: 1)
        XCTAssertFalse(plain.redComponent > 0.7 && plain.greenComponent < 0.5, "an unflagged row has no flag: \(plain)")
    }

    func testAFlagStaysVisibleOnASelectedRow() async throws {
        outline.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(30))
        }
        let flagged = try colour(row: 0, x: 1)
        XCTAssertGreaterThan(flagged.redComponent, 0.7, "selected row: \(flagged)")
        XCTAssertLessThan(flagged.greenComponent, 0.5)
    }
}
