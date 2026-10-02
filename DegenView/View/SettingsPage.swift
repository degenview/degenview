import SwiftUI

/// One page of the app's Settings window: the sheet-style header, scrolling content, and an
/// optional footer bar pinned under it for actions (Save, Test Connection).
struct SettingsPage<Content: View, Footer: View>: View {
    let systemImage: String
    let title: String
    let subtitle: String
    @ViewBuilder let content: Content
    @ViewBuilder let footer: Footer

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    SheetHeader(systemImage: systemImage, title: title, subtitle: subtitle)
                    content
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if Footer.self != EmptyView.self {
                Divider()
                footer
                    .padding(.horizontal, 24)
                    .padding(.vertical, 14)
            }
        }
    }
}

extension SettingsPage where Footer == EmptyView {
    init(systemImage: String, title: String, subtitle: String, @ViewBuilder content: () -> Content) {
        self.init(
            systemImage: systemImage, title: title, subtitle: subtitle, content: content, footer: { EmptyView() })
    }
}

/// A titled group inside a settings page: heading, optional description, then the cards.
struct SettingsSection<Content: View>: View {
    let title: String
    var subtitle: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.headline)
                if let subtitle {
                    Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                }
            }
            content
        }
    }
}

/// The quiet rounded card settings content sits in.
struct SettingsCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) { content }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(.separator.opacity(0.45), lineWidth: 1)
            }
    }
}
