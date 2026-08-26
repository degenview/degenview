import Foundation

enum JSONStoreLoadResult<Value> {
    case missing
    case value(Value)
    case quarantined(backupURL: URL)
    case unreadable(Error)
}

/// Generic JSON-backed persistence for any Codable type.
/// Reads/writes a single JSON file in the DegenView Application Support directory.
final class JSONStore<T: Codable> {
    let storageURL: URL
    private let now: () -> Date

    init(
        filename: String,
        directory: URL = AppSupport.directory,
        now: @escaping () -> Date = Date.init
    ) {
        storageURL = directory.appendingPathComponent(filename)
        self.now = now
    }

    /// Distinguishes an absent file from data that exists but cannot safely be used.
    /// Malformed JSON is moved aside before recovery can write a replacement.
    func loadResult() -> JSONStoreLoadResult<T> {
        guard FileManager.default.fileExists(atPath: storageURL.path) else { return .missing }

        let data: Data
        do {
            data = try Data(contentsOf: storageURL)
        } catch {
            return .unreadable(error)
        }

        do {
            return .value(try JSONDecoder().decode(T.self, from: data))
        } catch {
            do {
                let backupURL = try quarantineURL()
                try FileManager.default.moveItem(at: storageURL, to: backupURL)
                #if DEBUG
                    print(
                        "[JSONStore] Quarantined invalid \(storageURL.lastPathComponent) at \(backupURL.lastPathComponent)"
                    )
                #endif
                return .quarantined(backupURL: backupURL)
            } catch {
                #if DEBUG
                    print(
                        "[JSONStore] Could not quarantine \(storageURL.lastPathComponent): \(error.localizedDescription)"
                    )
                #endif
                return .unreadable(error)
            }
        }
    }

    /// Compatibility convenience for caches and non-critical stores.
    func load() -> T? {
        guard case .value(let value) = loadResult() else { return nil }
        return value
    }

    func saveThrowing(_ value: T) throws {
        let data = try JSONEncoder().encode(value)
        try data.write(to: storageURL, options: .atomic)
    }

    /// Encode and write the value atomically to disk. Failures are logged.
    func save(_ value: T) {
        do {
            try saveThrowing(value)
        } catch {
            #if DEBUG
                print("[JSONStore] Failed to save \(storageURL.lastPathComponent): \(error.localizedDescription)")
            #endif
        }
    }

    private func quarantineURL() throws -> URL {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyyMMdd-HHmmss"

        let extensionName = storageURL.pathExtension
        let stem = storageURL.deletingPathExtension().lastPathComponent
        let timestamp = formatter.string(from: now())
        let directory = storageURL.deletingLastPathComponent()
        var suffix = 0

        while true {
            let collisionSuffix = suffix == 0 ? "" : "-\(suffix)"
            let filename =
                "\(stem).corrupt-\(timestamp)\(collisionSuffix)"
                + (extensionName.isEmpty ? "" : ".\(extensionName)")
            let candidate = directory.appendingPathComponent(filename)
            if !FileManager.default.fileExists(atPath: candidate.path) { return candidate }
            suffix += 1
        }
    }
}
