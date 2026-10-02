import XCTest

@testable import DegenView

final class KeychainPolicyTests: XCTestCase {
    func testPolicyIsDisabledUnderXCTest() {
        XCTAssertTrue(KeychainPolicy.isDisabled)
    }

    func testCoinMarketCapStoreReportsNothingSaved() {
        XCTAssertNil(CoinMarketCapCredentialStore.apiKey)
        XCTAssertFalse(CoinMarketCapCredentialStore.isConfigured)
    }

    func testAlpacaStoreReportsNothingSaved() {
        XCTAssertFalse(AlpacaCredentialsStore.credentials.isConfigured)
        XCTAssertFalse(AlpacaCredentialsStore.isConfigured)
    }
}
