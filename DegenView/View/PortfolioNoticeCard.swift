import SwiftUI

/// A tinted callout for errors, failures and warnings, with an optional scrolling list of lines.
struct PortfolioNoticeCard: View {
    let systemImage: String
    let tint: Color
    let title: String
    var detail: String?
    var lines: [String] = []
    var maxListHeight: CGFloat = 110
    private static let inlineLimit = 3

    private var list: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                Text(line).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: systemImage)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(tint)
                .padding(.top, 1)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(title).font(.subheadline.weight(.semibold))
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
                if !lines.isEmpty {
                    // A short list takes its own height; only a long one scrolls.
                    if lines.count > Self.inlineLimit {
                        ScrollView { list }.frame(maxHeight: maxListHeight)
                    } else {
                        list
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(tint.opacity(0.25)))
        .accessibilityElement(children: .combine)
    }
}
