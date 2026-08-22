import AppKit

/// In-memory image cache shared across the app. `NSCache` is thread-safe and evicts
/// automatically under memory pressure, so images are never written to disk.
final class ImageCache: @unchecked Sendable {
    static let shared = ImageCache()

    private let cache = NSCache<NSURL, NSImage>()

    private init() {
        cache.countLimit = 300
    }

    /// Synchronous cache hit — returns nil on miss.
    func cachedImage(for url: URL) -> NSImage? {
        cache.object(forKey: url as NSURL)
    }

    /// Returns the image from cache if present, otherwise fetches, caches, and returns it.
    func image(for url: URL) async -> NSImage? {
        if let hit = cachedImage(for: url) { return hit }
        guard let (data, _) = try? await URLSession.shared.data(from: url),
              let image = NSImage(data: data) else { return nil }
        cache.setObject(image, forKey: url as NSURL)
        return image
    }
}
