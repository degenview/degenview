import Foundation

/// Generic JSON-backed persistence for any Codable type.
/// Reads/writes a single JSON file in the DegenView Application Support directory.
final class JSONStore<T: Codable> {
    private let storageURL: URL

    /// - Parameter filename: JSON filename (e.g. "tickers.json").
    init(filename: String) {
        storageURL = AppSupport.directory.appendingPathComponent(filename)
    }

    /// Load and decode the stored value. Returns `nil` if the file doesn't exist or decoding fails.
    func load() -> T? {
        guard FileManager.default.fileExists(atPath: storageURL.path),
              let data = try? Data(contentsOf: storageURL)
        else { return nil }

        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            #if DEBUG
            print("[JSONStore] Failed to decode \(storageURL.lastPathComponent): \(error.localizedDescription)")
            #endif
            return nil
        }
    }

    /// Encode and write the value atomically to disk. Failures are logged.
    func save(_ value: T) {
        do {
            let data = try JSONEncoder().encode(value)
            try data.write(to: storageURL, options: .atomic)
        } catch {
            #if DEBUG
            print("[JSONStore] Failed to save \(storageURL.lastPathComponent): \(error.localizedDescription)")
            #endif
        }
    }
}
