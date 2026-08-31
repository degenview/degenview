import XCTest

@testable import DegenView

final class CoinMarketCapTests: XCTestCase {
    func testKeylessRequestUsesPublicAPIAndNoHeader() {
        let request = CoinMarketCapClient.makeRequest(path: "/v3/fear-and-greed/latest", apiKey: nil)
        XCTAssertEqual(
            request.url?.absoluteString, "https://pro-api.coinmarketcap.com/public-api/v3/fear-and-greed/latest")
        XCTAssertNil(request.value(forHTTPHeaderField: "X-CMC_PRO_API_KEY"))
    }

    func testAuthenticatedRequestUsesProRootAndHeader() {
        let request = CoinMarketCapClient.makeRequest(path: "/v3/fear-and-greed/latest", apiKey: "abc123")
        XCTAssertEqual(request.url?.absoluteString, "https://pro-api.coinmarketcap.com/v3/fear-and-greed/latest")
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-CMC_PRO_API_KEY"), "abc123")
        XCTAssertFalse(request.url!.path.contains("public-api"))
    }

    func testAltcoinLatestDecodingAndRegime() throws {
        let json =
            #"{"data":{"altcoin_index":24,"altcoin_marketcap":1234567890,"snapshot_time":"2026-08-30T00:00:00Z","yearly_high":87,"yearly_high_date":"2026-03-14","yearly_low":14,"yearly_low_date":"2026-06-22"},"status":{"error_code":0}}"#
            .data(using: .utf8)!
        let envelope = try JSONDecoder().decode(CMCEnvelope<AltcoinSeasonLatest>.self, from: json)
        XCTAssertEqual(envelope.data.altcoinIndex, 24)
        XCTAssertEqual(envelope.data.classification, "Bitcoin Season")
    }

    func testAltcoinRegimeBoundaries() throws {
        for value in [0.0, 24, 25] { XCTAssertEqual(try latest(value).classification, "Bitcoin Season") }
        for value in [26.0, 50, 74] { XCTAssertEqual(try latest(value).classification, "Neutral") }
        for value in [75.0, 76, 100] { XCTAssertEqual(try latest(value).classification, "Altcoin Season") }
    }

    func testFearGreedLatestAndUnixTimestamp() throws {
        let latestJSON =
            #"{"data":{"value":77,"value_classification":"Greed","update_time":"2026-08-30T12:00:00Z"},"status":{"error_code":0}}"#
            .data(using: .utf8)!
        let latest = try JSONDecoder().decode(CMCEnvelope<FearAndGreedLatest>.self, from: latestJSON).data
        XCTAssertEqual(latest.value, 77)
        XCTAssertEqual(latest.valueClassification, "Greed")
        XCTAssertEqual(CMCDateParser.parse("1726617600")?.timeIntervalSince1970, 1_726_617_600)
    }

    func testStatusAcceptsLiveStringErrorCode() throws {
        let json =
            #"{"data":{"value":75,"update_time":"2026-08-31T16:38:10.020Z","value_classification":"Greed"},"status":{"timestamp":"2026-08-31T16:48:42.327Z","error_code":"0","error_message":"","elapsed":2,"credit_count":1}}"#
            .data(using: .utf8)!
        let envelope = try JSONDecoder().decode(CMCEnvelope<FearAndGreedLatest>.self, from: json)
        XCTAssertEqual(envelope.status.errorCode, 0)
        XCTAssertEqual(envelope.data.value, 75)
    }

    func testCMCConfigRoundTripsWithoutCredential() throws {
        let config = TickerConfig(
            symbol: CoinMarketCapChartType.fearAndGreedHistorical.rawValue, source: .coinMarketCap,
            coinMarketCapChart: .init(type: .fearAndGreedHistorical, fearGreedRange: .oneYear))
        let data = try JSONEncoder().encode(config)
        XCTAssertFalse(String(decoding: data, as: UTF8.self).localizedCaseInsensitiveContains("apiKey"))
        let restored = try JSONDecoder().decode(TickerConfig.self, from: data)
        XCTAssertEqual(restored.coinMarketCapChart?.type, .fearAndGreedHistorical)
        XCTAssertEqual(restored.coinMarketCapChart?.fearGreedRange, .oneYear)
    }

    private func latest(_ value: Double) throws -> AltcoinSeasonLatest {
        let json =
            "{\"altcoin_index\":\(value),\"altcoin_marketcap\":0,\"snapshot_time\":\"2026-08-30T00:00:00Z\",\"yearly_high\":100,\"yearly_high_date\":\"2026-01-01\",\"yearly_low\":0,\"yearly_low_date\":\"2026-01-02\"}"
            .data(using: .utf8)!
        return try JSONDecoder().decode(AltcoinSeasonLatest.self, from: json)
    }
}
