import Foundation

/// Drives the About page's update indicator. Runs one check at a time; a second request while one
/// is in flight is ignored.
@MainActor
final class UpdateCheckViewModel: ObservableObject {
    @Published private(set) var status: UpdateStatus = .idle

    private let checker: UpdateChecker
    private let currentVersion: String
    private let now: () -> Date

    init(
        checker: UpdateChecker = PlaceholderUpdateChecker(),
        currentVersion: String,
        now: @escaping () -> Date = Date.init
    ) {
        self.checker = checker
        self.currentVersion = currentVersion
        self.now = now
    }

    var isChecking: Bool { status == .checking }

    func check() async {
        guard !isChecking else { return }
        status = .checking
        do {
            switch try await checker.check(currentVersion: currentVersion) {
            case .upToDate:
                status = .upToDate(checkedAt: now())
            case .available(let version, let url):
                status = .available(version: version, releaseURL: url)
            }
        } catch is CancellationError {
            status = .idle
        } catch {
            status = .failed(message: error.localizedDescription)
        }
    }
}
