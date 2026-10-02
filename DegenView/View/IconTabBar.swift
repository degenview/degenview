import SwiftUI

/// A section switcher (portfolio tabs, Add Chart sources): a pill-shaped segmented control with an icon and title per
/// segment. Built by hand because the system segmented control draws neither icons beside text
/// nor a size beyond "regular".
struct IconTabBar<Value: Hashable>: View {
    struct Item: Identifiable {
        let value: Value
        let title: String
        let systemImage: String
        /// A tally shown after the title; nil or zero draws nothing.
        var count: Int?
        var id: String { title }
    }

    let items: [Item]
    @Binding var selection: Value
    /// Smaller type and padding, for sheets where six segments share one row.
    var isCompact = false
    @Namespace private var highlight

    var body: some View {
        HStack(spacing: 2) {
            ForEach(items) { item in
                segment(item)
            }
        }
        .padding(3)
        .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 11, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Section")
    }

    private func segment(_ item: Item) -> some View {
        let isSelected = item.value == selection
        return Button {
            withAnimation(.easeOut(duration: 0.18)) { selection = item.value }
        } label: {
            HStack(spacing: isCompact ? 5 : 7) {
                Image(systemName: item.systemImage)
                    .font(.system(size: isCompact ? 12 : 14, weight: .medium))
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                Text(item.title)
                    .font(.system(size: isCompact ? 12.5 : 14, weight: isSelected ? .semibold : .medium))
                    .foregroundStyle(isSelected ? Color.primary : .secondary)
                    .lineLimit(1)
                if let count = item.count, count > 0 {
                    Text("\(count)")
                        .font(.system(size: isCompact ? 10.5 : 11.5, weight: .semibold).monospacedDigit())
                        .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 1)
                        .background(
                            (isSelected ? Color.accentColor : Color.secondary).opacity(0.15), in: Capsule())
                }
            }
            .padding(.horizontal, isCompact ? 11 : 16)
            .padding(.vertical, isCompact ? 6 : 8)
            .contentShape(Rectangle())
            .background {
                if isSelected {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Color(nsColor: .controlBackgroundColor))
                        .shadow(color: .black.opacity(0.12), radius: 2, y: 1)
                        .overlay {
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .strokeBorder(.separator.opacity(0.6))
                        }
                        .matchedGeometryEffect(id: "selection", in: highlight)
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(item.title)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
