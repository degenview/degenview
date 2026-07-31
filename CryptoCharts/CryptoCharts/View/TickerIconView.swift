import SwiftUI

/// A coin icon that always occupies the same slot.
///
/// Every failure mode — no URL resolved, image still loading, image failed to load —
/// falls back to a letter monogram, so headers stay aligned whether or not artwork
/// exists for the coin.
struct TickerIconView: View {
    let symbol: String
    let url: URL?

    var body: some View {
        Group {
            if let url {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image.resizable().scaledToFit()
                    default:
                        monogram
                    }
                }
            } else {
                monogram
            }
        }
        .frame(width: Icon.size, height: Icon.size)
        .clipShape(Circle())
    }

    // MARK: - Fallback

    private var monogram: some View {
        Circle()
            .fill(background)
            .overlay {
                Text(initials)
                    .font(.system(size: Icon.size * 0.42, weight: .bold, design: .rounded))
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
