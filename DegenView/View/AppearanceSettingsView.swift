import SwiftUI

/// Settings ▸ Appearance: the app theme, picked from small previews.
struct AppearanceSettingsView: View {
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    var body: some View {
        SettingsPage(
            systemImage: "paintbrush.fill", title: "Appearance",
            subtitle: "Choose how DegenView looks across every window and chart."
        ) {
            SettingsSection(title: "Theme", subtitle: "System follows your Mac's light or dark setting.") {
                HStack(spacing: 12) {
                    ForEach(AppTheme.allCases) { theme in
                        ThemeTile(theme: theme, isSelected: appTheme == theme) { appTheme = theme }
                    }
                }
            }
        }
    }
}

/// One theme as a miniature window in that theme (System shows both halves), with its name.
private struct ThemeTile: View {
    let theme: AppTheme
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: 10) {
                preview
                    .frame(height: 92)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
                HStack(spacing: 6) {
                    Image(systemName: theme.icon).foregroundStyle(isSelected ? Color.accentColor : .secondary)
                    Text(theme.rawValue).font(.subheadline.weight(.semibold))
                    Spacer(minLength: 0)
                    Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(isSelected ? Color.accentColor : .secondary.opacity(0.6))
                }
            }
            .padding(10)
            .background(fill, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(
                        isSelected ? Color.accentColor : Color.primary.opacity(0.12), lineWidth: isSelected ? 1.5 : 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .accessibilityLabel("\(theme.rawValue) theme")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var fill: Color {
        if isSelected { return Color.accentColor.opacity(0.1) }
        return isHovered ? Color.primary.opacity(0.07) : Color.primary.opacity(0.03)
    }

    @ViewBuilder private var preview: some View {
        switch theme {
        case .light: MiniWindow(isDark: false)
        case .dark: MiniWindow(isDark: true)
        case .system:
            HStack(spacing: 0) {
                MiniWindow(isDark: false)
                MiniWindow(isDark: true)
            }
        }
    }
}

/// A stylised window: title bar, a rising line chart and two cards, in fixed light or dark colors.
private struct MiniWindow: View {
    let isDark: Bool

    private var background: Color { isDark ? Color(white: 0.13) : Color(white: 0.96) }
    private var bar: Color { isDark ? Color(white: 0.2) : Color(white: 0.88) }
    private var card: Color { isDark ? Color(white: 0.2) : .white }

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .topLeading) {
                background
                bar.frame(height: 14)
                HStack(spacing: 3) {
                    ForEach([Color.red, .yellow, .green], id: \.self) { color in
                        Circle().fill(color.opacity(0.85)).frame(width: 5, height: 5)
                    }
                }
                .padding(.leading, 6)
                .padding(.top, 4.5)
                RoundedRectangle(cornerRadius: 4).fill(card)
                    .frame(width: geometry.size.width - 16, height: 34)
                    .offset(x: 8, y: 22)
                    .overlay(alignment: .topLeading) {
                        Path { path in
                            let width = geometry.size.width - 16
                            path.move(to: CGPoint(x: 4, y: 28))
                            path.addLine(to: CGPoint(x: width * 0.3, y: 20))
                            path.addLine(to: CGPoint(x: width * 0.5, y: 24))
                            path.addLine(to: CGPoint(x: width * 0.75, y: 10))
                            path.addLine(to: CGPoint(x: width - 4, y: 14))
                        }
                        .stroke(Color.green, style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
                        .offset(x: 8, y: 22)
                    }
                HStack(spacing: 4) {
                    RoundedRectangle(cornerRadius: 3).fill(card)
                    RoundedRectangle(cornerRadius: 3).fill(card)
                }
                .frame(width: geometry.size.width - 16, height: 22)
                .offset(x: 8, y: 62)
            }
        }
    }
}
