import XCTest

@testable import DegenView

final class BitcoinPowerLawTests: XCTestCase {
    func testPublishedDefaultsAndBands() throws {
        let config = BitcoinPowerLawConfig.default
        XCTAssertEqual(config.intercept, -16.493)
        XCTAssertEqual(config.exponent, 5.688)
        XCTAssertEqual(config.lowerMultiplier, pow(10, -0.4), accuracy: 1e-12)
        XCTAssertEqual(config.upperMultiplier, pow(10, 0.4), accuracy: 1e-12)

        let model = try XCTUnwrap(config.price(days: 5_000))
        let bands = try XCTUnwrap(config.bandPrices(days: 5_000))
        XCTAssertEqual(bands.lower, model * config.lowerMultiplier, accuracy: 1e-10)
        XCTAssertEqual(bands.upper, model * config.upperMultiplier, accuracy: 1e-10)
    }

    func testGenesisDayAndTenCalendarYearProjection() {
        XCTAssertEqual(BitcoinPowerLawModel.daysSinceGenesis(BitcoinPowerLawModel.genesisDate), 0)
        XCTAssertNil(BitcoinPowerLawConfig.default.price(days: 0))

        let start = BitcoinPowerLawModel.utcCalendar.date(from: DateComponents(year: 2024, month: 2, day: 29))!
        let end = BitcoinPowerLawModel.projectionEnd(from: start)
        let parts = BitcoinPowerLawModel.utcCalendar.dateComponents([.year, .month, .day], from: end)
        XCTAssertEqual(parts.year, 2034)
        XCTAssertEqual(parts.month, 2)
        XCTAssertEqual(parts.day, 28)
    }

    func testLogCoordinatesRoundTripAndRangesContainData() throws {
        let history = [
            BitcoinDailyClose(date: Date(timeIntervalSince1970: 1_278_374_400), close: 0.07),
            BitcoinDailyClose(date: Date(timeIntervalSince1970: 1_700_000_000), close: 36_000),
        ]
        let plot = try XCTUnwrap(
            BitcoinPowerLawPlot(
                history: history, config: .default,
                now: Date(timeIntervalSince1970: 1_700_000_000)))
        let days = BitcoinPowerLawModel.daysSinceGenesis(history[1].date)
        let point = try XCTUnwrap(plot.point(days: days, price: history[1].close))
        let value = plot.value(at: point)
        XCTAssertEqual(value.days, days, accuracy: 1e-8)
        XCTAssertEqual(value.price, history[1].close, accuracy: 1e-8)
        XCTAssertTrue(plot.yRange.contains(log10(history[0].close)))
        XCTAssertTrue(plot.yRange.contains(log10(history[1].close)))
    }

    func testParsingFiltersInvalidPricesSortsAndReplacesDuplicates() throws {
        let json =
            #"{"data":{"ohlc":[{"timestamp":"20","close":"2"},{"timestamp":"10","close":"1"},{"timestamp":"20","close":"3"},{"timestamp":"30","close":"0"},{"timestamp":"40","close":"oops"}]}}"#
        let parsed = try BitcoinHistoryService.parse(Data(json.utf8))
        XCTAssertEqual(parsed.map(\.date.timeIntervalSince1970), [10, 20])
        XCTAssertEqual(parsed.map(\.close), [1, 3])
    }

    func testConfigCodableAndLegacyTickerConfig() throws {
        let config = BitcoinPowerLawConfig(
            intercept: -15, exponent: 5,
            lowerMultiplier: 0.5, upperMultiplier: 2)
        XCTAssertEqual(
            try JSONDecoder().decode(
                BitcoinPowerLawConfig.self,
                from: JSONEncoder().encode(config)), config)

        let legacy = #"{"symbol":"BTC","source":"Binance","scripts":[]}"#
        let ticker = try JSONDecoder().decode(TickerConfig.self, from: Data(legacy.utf8))
        XCTAssertNil(ticker.bitcoinPowerLaw)
    }

    func testValidationRejectsNonFiniteAndNonPositiveParameters() {
        XCTAssertFalse(BitcoinPowerLawConfig(intercept: .infinity).isValid)
        XCTAssertFalse(BitcoinPowerLawConfig(exponent: 0).isValid)
        XCTAssertFalse(BitcoinPowerLawConfig(lowerMultiplier: -1).isValid)
        XCTAssertFalse(BitcoinPowerLawConfig(upperMultiplier: 0).isValid)
    }
}
