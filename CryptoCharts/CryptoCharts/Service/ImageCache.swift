import AppKit

/// Memory + disk image cache shared across the app. `NSCache` keeps hot images cheap,
/// while the URL-keyed files prevent resolved artwork being downloaded every launch.
final class ImageCache: @unchecked Sendable {
    static let shared = ImageCache()

    private let cache = NSCache<NSURL, NSImage>()
    private let diskDirectory: URL
    private let diskLock = NSLock()

    private init() {
        cache.countLimit = 300
        diskDirectory = AppSupport.directory.appendingPathComponent("icon_images", isDirectory: true)
        try? FileManager.default.createDirectory(
            at: diskDirectory,
            withIntermediateDirectories: true
        )
    }

    /// Synchronous memory/disk hit — returns nil on miss. Disk hits are promoted to
    /// memory so a re-scrolled row only pays for decoding once per launch.
    func cachedImage(for url: URL) -> NSImage? {
        if let image = cache.object(forKey: url as NSURL) { return image }

        let data: Data? = withDiskLock {
            try? Data(contentsOf: diskURL(for: url), options: .mappedIfSafe)
        }
        guard let data, let image = NSImage(data: data) else { return nil }
        cache.setObject(image, forKey: url as NSURL)
        return image
    }

    /// Returns a locally cached image when possible; otherwise downloads it and writes
    /// the original bytes atomically before returning it.
    func image(for url: URL) async -> NSImage? {
        if let hit = cachedImage(for: url) { return hit }

        guard let (data, response) = try? await AppSupport.defaultSession.data(from: url),
              let http = response as? HTTPURLResponse,
              http.statusCode == 200,
              http.mimeType?.hasPrefix("image/") == true,
              let image = NSImage(data: data) else { return nil }

        withDiskLock {
            try? data.write(to: diskURL(for: url), options: .atomic)
        }
        cache.setObject(image, forKey: url as NSURL)
        return image
    }

    /// Stable across processes, unlike Swift's randomized `hashValue`.
    private func diskURL(for url: URL) -> URL {
        let hash = url.absoluteString.utf8.reduce(UInt64(14_695_981_039_346_656_037)) {
            ($0 ^ UInt64($1)) &* 1_099_511_628_211
        }
        return diskDirectory.appendingPathComponent(String(hash, radix: 16))
    }

    private func withDiskLock<T>(_ work: () -> T) -> T {
        diskLock.lock()
        defer { diskLock.unlock() }
        return work()
    }
}
