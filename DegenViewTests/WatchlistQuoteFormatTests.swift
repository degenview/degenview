import XCTest

@testable import DegenView

final class WatchlistQuoteFormatTests: XCTestCase {
    func testMissingValuesAreADashNeverZero() {
        let empty = WatchlistQuote()
        XCTAssertEqual(WatchlistQuoteFormat.last(nil, source: .binance), "–")
        XCTAssertEqual(WatchlistQuoteFormat.last(empty, source: .binance), "–")
        XCTAssertEqual(WatchlistQuoteFormat.change(empty, source: .binance), "–")
        XCTAssertEqual(WatchlistQuoteFormat.percent(empty), "–")
        XCTAssertEqual(WatchlistQuoteFormat.volume(empty), "–")
    }

    func testPercentIsSignedWithTwoDecimals() {
        XCTAssertEqual(WatchlistQuoteFormat.percent(WatchlistQuote(changePercent: 2.4)), "+2.40%")
        XCTAssertEqual(WatchlistQuoteFormat.percent(WatchlistQuote(changePercent: -0.7)), "-0.70%")
    }

    func testPricesUseTheSourceScale() {
        let price = WatchlistQuote(last: 0.66)
        let probability = WatchlistQuoteFormat.last(price, source: .polymarket)
        XCTAssertTrue(probability.hasPrefix("66"), probability)
        XCTAssertTrue(probability.hasSuffix("%"), probability)
        XCTAssertFalse(WatchlistQuoteFormat.last(price, source: .binance).hasSuffix("%"))
    }

    func testVolumeIsCompactAndOnlyCurrencyGetsADollarSign() {
        // Whole numbers, so the check does not depend on the machine's decimal separator.
        XCTAssertEqual(
            WatchlistQuoteFormat.volume(WatchlistQuote(volume: 3_000_000, volumeKind: .quoteCurrency)), "$3M")
        XCTAssertEqual(WatchlistQuoteFormat.volume(WatchlistQuote(volume: 4_000, volumeKind: .shares)), "4K")
        XCTAssertEqual(WatchlistQuoteFormat.volume(WatchlistQuote(volume: 12, volumeKind: .base)), "12")
    }

    func testDirectionDoesNotDependOnColourAlone() {
        XCTAssertEqual(WatchlistQuoteFormat.direction(WatchlistQuote(changePercent: 1)).glyph, "▲")
        XCTAssertEqual(WatchlistQuoteFormat.direction(WatchlistQuote(changePercent: -1)).glyph, "▼")
        XCTAssertEqual(WatchlistQuoteFormat.direction(WatchlistQuote(changePercent: 0)).glyph, "")
        XCTAssertEqual(WatchlistQuoteFormat.direction(nil).glyph, "")
    }

    func testSpokenLabelMentionsStaleness() {
        let stale = WatchlistQuote(last: 100, changePercent: 1.5, freshness: .stale)
        let spoken = WatchlistQuoteFormat.spoken(name: "Bitcoin", quote: stale, source: .binance)
        XCTAssertTrue(spoken.contains("up"))
        XCTAssertTrue(spoken.contains("Stale"))
        XCTAssertEqual(WatchlistQuoteFormat.spoken(name: "Bitcoin", quote: nil, source: .binance), "Bitcoin, no quote")
    }
}
