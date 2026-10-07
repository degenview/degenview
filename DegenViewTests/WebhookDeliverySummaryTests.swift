import XCTest

@testable import DegenView

final class WebhookDeliverySummaryTests: XCTestCase {
    private func record(
        _ state: WebhookDeliveryState, status: Int? = nil, duration: TimeInterval? = nil,
        error: WebhookDeliveryError? = nil
    ) -> WebhookDeliveryRecord {
        WebhookDeliveryRecord(
            id: UUID(), endpointID: UUID(), source: .price, eventID: UUID(), timestamp: Date(), state: state,
            statusCode: status, duration: duration, error: error)
    }

    func testCountsDeliveredOfAttempted() {
        let summary = WebhookDeliverySummary(records: [
            record(.delivered, status: 200, duration: 0.143),
            record(.delivered, status: 204, duration: 0.05),
            record(.failed, status: 401, duration: 0.098, error: .httpFailure),
        ])
        XCTAssertEqual(summary.title, "Webhooks 2/3 delivered")
        XCTAssertEqual(summary.tone, .warning)
    }

    func testEveryOutcomeToneAndDetail() {
        XCTAssertEqual(WebhookDeliverySummary(records: [record(.delivered, status: 200)]).tone, .good)
        XCTAssertEqual(WebhookDeliverySummary(records: [record(.failed, error: .timeout)]).tone, .bad)
        XCTAssertEqual(WebhookDeliverySummary(records: [record(.pending)]).title, "Webhooks sending…")
        XCTAssertEqual(
            WebhookDeliverySummary.detail(for: record(.delivered, status: 200, duration: 0.143)), "HTTP 200 · 143 ms")
        XCTAssertEqual(
            WebhookDeliverySummary.detail(for: record(.failed, status: 401, duration: 0.098, error: .httpFailure)),
            "HTTP 401 · 98 ms")
        XCTAssertEqual(WebhookDeliverySummary.detail(for: record(.pending)), "Sending…")
        XCTAssertTrue(WebhookDeliverySummary(records: []).isEmpty)
    }
}
