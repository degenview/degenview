import Foundation

/// What the About page shows about the running build, read from a bundle so tests can inject one.
struct AppInfo {
    static let repositoryURL = URL(string: "https://github.com/degenview/degenview")!
    static let donationURL = URL(string: "https://buymeacoffee.com/degenview")!
    static let licenseName = "GPL-3.0-only"
    static let tagline = "Crypto candlestick charts for macOS"

    let name: String
    let version: String
    let build: String
    let copyright: String
    /// When the executable was linked; `nil` if the file can't be inspected.
    let buildDate: Date?

    init(bundle: Bundle = .main) {
        func string(_ key: String) -> String? {
            let value = bundle.object(forInfoDictionaryKey: key) as? String
            return value?.isEmpty == false ? value : nil
        }
        name = string("CFBundleDisplayName") ?? string("CFBundleName") ?? "DegenView"
        version = string("CFBundleShortVersionString") ?? "1.0"
        build = string("CFBundleVersion") ?? ""
        copyright =
            string("NSHumanReadableCopyright")
            ?? "Copyright © 2026 Nico Oelgart. Licensed under \(Self.licenseName)."
        buildDate = bundle.executableURL
            .flatMap { try? FileManager.default.attributesOfItem(atPath: $0.path)[.modificationDate] as? Date }
    }

    /// `1.0 (42)`, or just `1.0` when the build number is missing or equals the version.
    var versionLabel: String {
        build.isEmpty || build == version ? version : "\(version) (\(build))"
    }

    /// The build number when it says something the version doesn't.
    var distinctBuild: String? {
        build.isEmpty || build == version ? nil : build
    }

    /// The header's second line: `Version 1.0 - 7/10/2026` (date in the user's locale, no time).
    var headerSubtitle: String {
        var line = "Version \(version)"
        if let buildDate {
            line += " - \(buildDate.formatted(date: .numeric, time: .omitted))"
        }
        return line
    }

    /// One line for bug reports: `DegenView 1.0 (1) · built Oct 7, 2026 at 10:00 AM`.
    var clipboardSummary: String {
        var line = "\(name) \(versionLabel)"
        if let buildDate {
            line += " · built \(buildDate.formatted(date: .abbreviated, time: .shortened))"
        }
        return line
    }
}
