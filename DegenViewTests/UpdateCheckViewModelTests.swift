import XCTest

@testable import DegenView

@MainActor
final class UpdateCheckViewModelTests: XCTestCase {
    private struct StubChecker: UpdateChecker {
        let result: Result<UpdateCheckResult, Error>
        func check(currentVersion: String) async throws -> UpdateCheckResult { try result.get() }
    }

    private struct Boom: LocalizedError {
        var errorDescription: String? { "offline" }
    }

    private let date = Date(timeIntervalSince1970: 1_000)

    private func model(_ result: Result<UpdateCheckResult, Error>) -> UpdateCheckViewModel {
        UpdateCheckViewModel(checker: StubChecker(result: result), currentVersion: "1.0", now: { self.date })
    }

    func testStartsIdle() {
        XCTAssertEqual(model(.success(.upToDate)).status, .idle)
    }

    func testUpToDateRecordsCheckTime() async {
        let vm = model(.success(.upToDate))
        await vm.check()
        XCTAssertEqual(vm.status, .upToDate(checkedAt: date))
    }

    func testAvailableCarriesVersionAndURL() async {
        let url = AppInfo.repositoryURL
        let vm = model(.success(.available(version: "2.0", releaseURL: url)))
        await vm.check()
        XCTAssertEqual(vm.status, .available(version: "2.0", releaseURL: url))
    }

    func testFailureSurfacesMessage() async {
        let vm = model(.failure(Boom()))
        await vm.check()
        XCTAssertEqual(vm.status, .failed(message: "offline"))
    }

    func testPlaceholderReportsUpToDate() async throws {
        let result = try await PlaceholderUpdateChecker(delay: .zero).check(currentVersion: "1.0")
        XCTAssertEqual(result, .upToDate)
    }
}
