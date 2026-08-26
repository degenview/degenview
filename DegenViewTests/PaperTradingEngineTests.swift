import XCTest

@testable import DegenView

final class PaperTradingEngineTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_700_000_000)
    private var instrument: PaperInstrument {
        .init(
            key: "test:XYZ", symbol: "XYZ", displayName: "XYZ", source: .alpaca, assetClass: .stock,
            quoteCurrency: .USD, tickSize: 1, minimumQuantity: 1, quantityIncrement: 1, contractMultiplier: 1,
            pointValue: 1)
    }

    private func makeEngine(
        balance: Decimal = 100_000, commission: PaperCommissionConfiguration = .none,
        leverage: Decimal = 1
    ) async throws -> (PaperTradingEngine, UUID) {
        let engine = PaperTradingEngine(quoteMaximumAge: 30, now: { self.now })
        var settings = PaperAccountSettings()
        settings.commission = commission
        settings.leverage = .init(
            stocks: leverage, crypto: leverage, forex: leverage, futures: leverage, prediction: leverage)
        let id = try await engine.createAccount(name: "Test PAPER", initialBalance: balance, settings: settings)
        return (engine, id)
    }

    private func quote(_ engine: PaperTradingEngine, bid: Decimal? = nil, ask: Decimal? = nil, last: Decimal? = nil)
        async throws
    {
        try await engine.process(.init(instrumentKey: instrument.key, bid: bid, ask: ask, last: last, timestamp: now))
    }

    private func request(
        _ account: UUID, side: PaperOrderSide, type: PaperOrderType = .market,
        quantity: Decimal = 10, limit: Decimal? = nil, stop: Decimal? = nil,
        takeProfit: Decimal? = nil, stopLoss: Decimal? = nil
    ) -> PaperOrderRequest {
        .init(
            accountID: account, instrument: instrument, side: side, type: type, quantity: quantity,
            limitPrice: limit, stopPrice: stop, takeProfit: takeProfit, stopLoss: stopLoss)
    }

    func testMarketBuyUsesAskAndLongPnLUsesBid() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        _ = try await engine.submit(request(id, side: .buy))
        var state = await engine.snapshot()
        XCTAssertEqual(state.fills.last?.price, 100)
        XCTAssertEqual(state.fills.last?.priceSource, .bidAsk)
        try await quote(engine, bid: 102, ask: 103)
        state = await engine.snapshot()
        let pnl = await engine.unrealizedPnL(try XCTUnwrap(state.positions.first))
        XCTAssertEqual(pnl, 20)
    }

    func testMarketSellUsesBidAndShortPnLUsesAsk() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        _ = try await engine.submit(request(id, side: .sell))
        var state = await engine.snapshot()
        XCTAssertEqual(state.fills.last?.price, 99)
        try await quote(engine, bid: 96, ask: 97)
        state = await engine.snapshot()
        let pnl = await engine.unrealizedPnL(try XCTUnwrap(state.positions.first))
        XCTAssertEqual(pnl, 20)
    }

    func testLastPriceFallbackIsAudited() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, last: 100)
        _ = try await engine.submit(request(id, side: .buy))
        let state = await engine.snapshot()
        XCTAssertEqual(state.fills.last?.priceSource, .lastPriceFallback)
    }

    func testLimitBuyAndSell() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        let buy = try await engine.submit(request(id, side: .buy, type: .limit, limit: 98))
        var state = await engine.snapshot()
        XCTAssertEqual(state.orders.first { $0.id == buy }?.status, .working)
        try await quote(engine, bid: 97, ask: 98)
        state = await engine.snapshot()
        XCTAssertEqual(state.orders.first { $0.id == buy }?.status, .filled)
        let sell = try await engine.submit(request(id, side: .sell, type: .limit, limit: 105))
        try await quote(engine, bid: 105, ask: 106)
        state = await engine.snapshot()
        XCTAssertEqual(state.orders.first { $0.id == sell }?.status, .filled)
    }

    func testStopsAndStopLimitActivation() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        let buyStop = try await engine.submit(request(id, side: .buy, type: .stop, stop: 101))
        try await quote(engine, bid: 100, ask: 101)
        var state = await engine.snapshot()
        XCTAssertEqual(state.orders.first { $0.id == buyStop }?.status, .filled)
        let sellStop = try await engine.submit(request(id, side: .sell, type: .stop, stop: 98))
        try await quote(engine, bid: 98, ask: 99)
        state = await engine.snapshot()
        XCTAssertEqual(state.orders.first { $0.id == sellStop }?.status, .filled)
        let stopLimit = try await engine.submit(request(id, side: .buy, type: .stopLimit, limit: 101, stop: 102))
        try await quote(engine, bid: 102, ask: 103)
        state = await engine.snapshot()
        var order = try XCTUnwrap(state.orders.first { $0.id == stopLimit })
        XCTAssertTrue(order.stopTriggered)
        XCTAssertEqual(order.status, .working)
        try await quote(engine, bid: 100, ask: 101)
        state = await engine.snapshot()
        order = try XCTUnwrap(state.orders.first { $0.id == stopLimit })
        XCTAssertEqual(order.status, .filled)
    }

    func testWeightedAverageReductionAndReversal() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 10, ask: 10)
        _ = try await engine.submit(request(id, side: .buy, quantity: 100))
        try await quote(engine, bid: 13, ask: 13)
        _ = try await engine.submit(request(id, side: .buy, quantity: 50))
        var state = await engine.snapshot()
        var position = try XCTUnwrap(state.positions.first)
        XCTAssertEqual(position.quantity, 150)
        XCTAssertEqual(position.averageEntryPrice, 11)
        try await quote(engine, bid: 14, ask: 14)
        _ = try await engine.submit(request(id, side: .sell, quantity: 50))
        state = await engine.snapshot()
        position = try XCTUnwrap(state.positions.first)
        XCTAssertEqual(position.quantity, 100)
        XCTAssertEqual(position.averageEntryPrice, 11)
        XCTAssertEqual(position.realizedGrossPnL, 150)
        try await quote(engine, bid: 22, ask: 22)
        _ = try await engine.submit(request(id, side: .sell, quantity: 150))
        state = await engine.snapshot()
        position = try XCTUnwrap(state.positions.first)
        XCTAssertEqual(position.signedQuantity, -50)
        XCTAssertEqual(position.averageEntryPrice, 22)
    }

    func testCommissionModels() async throws {
        for (configuration, expected) in [
            (PaperCommissionConfiguration.fixedPerOrder(3), Decimal(3)), (.percentage(1), 10), (.perContract(2), 20),
        ] {
            let (engine, id) = try await makeEngine(commission: configuration)
            try await quote(engine, bid: 100, ask: 100)
            _ = try await engine.submit(request(id, side: .buy))
            let state = await engine.snapshot()
            XCTAssertEqual(state.fills.last?.commission, expected)
        }
    }

    func testMarginAndInsufficientFunds() async throws {
        let (engine, id) = try await makeEngine(balance: 100, leverage: 2)
        try await quote(engine, bid: 10, ask: 10)
        await XCTAssertThrowsErrorAsync { _ = try await engine.submit(self.request(id, side: .buy, quantity: 30)) }
        _ = try await engine.submit(request(id, side: .buy, quantity: 20))
        let metrics = await engine.metrics(accountID: id)
        XCTAssertEqual(metrics.positionMargin, 100)
        _ = try await engine.submit(request(id, side: .sell, quantity: 20))
        let state = await engine.snapshot()
        XCTAssertTrue(state.positions.isEmpty, "Risk-reducing orders must not require fresh margin")
    }

    func testCancelModifyAndInvalidTransition() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        let order = try await engine.submit(request(id, side: .buy, type: .limit, limit: 90))
        try await engine.modify(order, changes: .init(quantity: 20, limitPrice: 91))
        var state = await engine.snapshot()
        var saved = try XCTUnwrap(state.orders.first { $0.id == order })
        XCTAssertEqual(saved.originalQuantity, 20)
        XCTAssertEqual(saved.limitPrice, 91)
        try await engine.cancel(order)
        state = await engine.snapshot()
        saved = try XCTUnwrap(state.orders.first { $0.id == order })
        XCTAssertEqual(saved.status, .canceled)
        await XCTAssertThrowsErrorAsync { try await engine.modify(order, changes: .init(limitPrice: 92)) }
    }

    func testBracketOCO() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        _ = try await engine.submit(request(id, side: .buy, takeProfit: 110, stopLoss: 90))
        var children = await engine.snapshot().orders.filter { $0.role != .entry }
        XCTAssertEqual(children.count, 2)
        try await quote(engine, bid: 110, ask: 111)
        children = await engine.snapshot().orders.filter { $0.role != .entry }
        XCTAssertEqual(children.filter { $0.status == .filled }.count, 1)
        XCTAssertEqual(children.filter { $0.status == .canceled }.count, 1)
        let state = await engine.snapshot()
        XCTAssertTrue(state.positions.isEmpty)
    }

    func testResetAndCodableRestoration() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        _ = try await engine.submit(request(id, side: .buy))
        let data = try JSONEncoder().encode(await engine.snapshot())
        let restored = try JSONDecoder().decode(PaperTradingSnapshot.self, from: data)
        XCTAssertEqual(restored.positions.count, 1)
        XCTAssertEqual(restored.fills.count, 1)
        try await engine.resetAccount(id, currency: .EUR, initialBalance: 50_000, settings: .init())
        let reset = await engine.snapshot()
        XCTAssertTrue(reset.positions.isEmpty)
        XCTAssertTrue(reset.orders.isEmpty)
        XCTAssertTrue(reset.fills.isEmpty)
        XCTAssertEqual(reset.accounts.first?.cashBalance, 50_000)
    }

    func testPaperServiceNeverInvokesLiveAdapter() async throws {
        let (engine, id) = try await makeEngine()
        try await quote(engine, bid: 99, ask: 100)
        let paper = PaperTradingExecutionService(engine: engine)
        let live = LiveExecutionSpy()
        _ = try await paper.submit(request(id, side: .buy))
        let liveCount = await live.invocationCount
        let state = await engine.snapshot()
        XCTAssertEqual(paper.environment, .paper)
        XCTAssertEqual(liveCount, 0)
        XCTAssertEqual(state.fills.count, 1)
    }

    func testPaperTradingCurrencyAndPercentageFormatting() {
        let locale = Locale(identifier: "en_US")
        XCTAssertEqual(
            PaperTradingFormatter.money(Decimal(string: "1234.567")!, currency: .USD, locale: locale), "$1,234.57")
        XCTAssertEqual(
            PaperTradingFormatter.money(Decimal(string: "1234.567")!, currency: .JPY, locale: locale), "¥1,235")
        XCTAssertEqual(
            PaperTradingFormatter.percent(Decimal(string: "0.23349063089384617193604934")!, locale: locale), "23.35%")
        XCTAssertEqual(
            PaperTradingFormatter.money(Decimal(string: "-0.004")!, currency: .USD, locale: locale), "-$0.00")
    }

    func testPaperTradingInstrumentPrecisionFormatting() {
        var precise = instrument
        precise.tickSize = Decimal(string: "0.00001")!
        precise.quantityIncrement = Decimal(string: "0.00000001")!
        let locale = Locale(identifier: "en_US")
        XCTAssertEqual(
            PaperTradingFormatter.price(Decimal(string: "1.23456")!, instrument: precise, locale: locale), "1.23456")
        XCTAssertEqual(
            PaperTradingFormatter.quantity(Decimal(string: "0.12345678")!, instrument: precise, locale: locale),
            "0.12345678")
    }
}

private actor LiveExecutionSpy: TradingExecutionService {
    let environment: TradingExecutionEnvironment = .live
    private(set) var invocationCount = 0
    func submit(_ order: PaperOrderRequest) async throws -> PaperOrderID {
        invocationCount += 1
        return UUID()
    }
    func cancel(_ orderID: PaperOrderID) async throws { invocationCount += 1 }
    func modify(_ orderID: PaperOrderID, changes: PaperOrderChanges) async throws { invocationCount += 1 }
}

private func XCTAssertThrowsErrorAsync(
    _ expression: () async throws -> Void, file: StaticString = #filePath, line: UInt = #line
) async {
    do {
        try await expression()
        XCTFail("Expected error", file: file, line: line)
    } catch {}
}
