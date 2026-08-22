import SwiftUI
import AppKit

/// A coin icon that always occupies the same slot.
///
/// Every failure mode — no URL resolved, image still loading, image failed to load —
/// falls back to a letter monogram, so headers stay aligned whether or not artwork
/// exists for the coin.
struct TickerIconView: View {
    let symbol: String
    let url: URL?
    var size: CGFloat = Icon.size

    @State private var loadedImage: NSImage?

    init(symbol: String, url: URL?, size: CGFloat = Icon.size) {
        self.symbol = symbol
        self.url = url
        self.size = size
        // Seed from cache so re-scrolled cells show the image instantly.
        _loadedImage = State(initialValue: url.flatMap { ImageCache.shared.cachedImage(for: $0) })
    }

    var body: some View {
        Group {
            if let img = loadedImage {
                Image(nsImage: img)
                    .resizable()
                    .scaledToFit()
            } else {
                monogram
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .task(id: url) {
            guard let url else { loadedImage = nil; return }
            loadedImage = await ImageCache.shared.image(for: url)
        }
    }

    // MARK: - Fallback

    private var monogram: some View {
        Circle()
            .fill(background)
            .overlay {
                Text(initials)
                    .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .minimumScaleFactor(0.6)
                    .padding(1)
            }
    }

    private var initials: String {
        let letters = symbol.filter { $0.isLetter || $0.isNumber }
        return String(letters.prefix(2)).uppercased()
    }

    /// Hue derived from the symbol so a coin keeps the same color across launches.
    /// `hashValue` is seeded per-process and would not, hence the explicit sum.
    private var background: Color {
        let seed = symbol.uppercased().unicodeScalars.reduce(0) { $0 &+ Int($1.value) &* 31 }
        return Color(hue: Double(seed % 360) / 360, saturation: 0.55, brightness: 0.7)
    }
}

#Preview {
    HStack(spacing: 8) {
        TickerIconView(symbol: "BTC", url: nil)
        TickerIconView(symbol: "WIF", url: nil)
        TickerIconView(symbol: "BONK", url: nil)
        TickerIconView(symbol: "5zpyutJu9ee6jFymDGoK", url: nil)
    }
    .padding()
}
