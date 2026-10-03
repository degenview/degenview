import SwiftUI

/// Vertical strip of drawing tools down the left edge of the window.
///
/// Sits outside the chart column, so the card-height math in `ContentView` is
/// unaffected and the strip spans the empty state too. The layout is a stack, so more
/// tools can be added under the ones already here.
struct ToolSidebar<BottomControls: View>: View {
    @Environment(\.openWindow) private var openWindow

    let activeTool: ChartTool
    let onSelect: (ChartTool) -> Void
    @ViewBuilder let bottomControls: () -> BottomControls

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 6) {
                SidebarIconButton(
                    icon: "plus",
                    label: "Crosshair",
                    isActive: activeTool == .crosshair
                ) {
                    onSelect(.crosshair)
                }
                SidebarIconButton(
                    icon: "line.diagonal",
                    label: "Trend line",
                    isActive: activeTool == .trendLine
                ) {
                    onSelect(.trendLine)
                }
                SidebarIconButton(
                    icon: "point.3.connected.trianglepath.dotted",
                    label: "Fib Retracement",
                    isActive: activeTool == .fibonacciRetracement
                ) {
                    onSelect(.fibonacciRetracement)
                }
                SidebarIconButton(
                    icon: "ruler",
                    label: "Ruler",
                    isActive: activeTool == .ruler
                ) {
                    onSelect(.ruler)
                }
                Spacer(minLength: 0)
                bottomControls()
                SidebarGroupDivider()
                SidebarIconButton(
                    icon: "bell",
                    label: "View Price Alerts"
                ) {
                    openWindow(id: "alerts")
                }
            }
            .padding(.horizontal, (UI.toolSidebarWidth - UI.toolButtonSize) / 2)
            .padding(.vertical, 10)
            .frame(width: UI.toolSidebarWidth)
            .background(.regularMaterial)

            Divider()
        }
    }
}

/// Short inset rule between button groups.
struct SidebarGroupDivider: View {
    var body: some View {
        Capsule()
            .fill(.separator)
            .frame(width: 22, height: 1)
            .padding(.vertical, 2)
            .accessibilityHidden(true)
    }
}

/// How an active control is drawn: a lit accent tile for a tool that is armed, or a
/// tinted glyph on a soft wash for a toggle (panel open, replay running).
enum SidebarActiveStyle {
    case filled
    case tinted
}

/// The tile every tool-strip control shares, so buttons and menus read as one set.
///
/// Hover is tracked here rather than in a `ButtonStyle` so a `Menu` label — which has no
/// style hook — gets the same treatment. `isPressed` is fed in by `SidebarIconButton`.
struct SidebarIconLabel: View {
    @Environment(\.isEnabled) private var isEnabled
    @State private var isHovering = false

    let icon: String
    var isActive = false
    var tint: Color = .accentColor
    var activeStyle: SidebarActiveStyle = .filled
    var isPressed = false
    /// Hover reported by a parent, for a control (a `Menu`) whose label never sees the pointer.
    var hoverOverride: Bool?

    private var hovering: Bool { hoverOverride ?? isHovering }
    private var isLit: Bool { isActive && activeStyle == .filled }
    private var tile: RoundedRectangle {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
    }

    var body: some View {
        Image(systemName: icon)
            .font(.system(size: UI.toolIconSize, weight: isActive ? .semibold : .medium))
            .foregroundStyle(glyph)
            .frame(width: UI.toolButtonSize, height: UI.toolButtonSize)
            .background(tile.fill(fill))
            .overlay {
                if isLit {
                    tile.strokeBorder(
                        LinearGradient(
                            colors: [.white.opacity(0.28), .white.opacity(0)],
                            startPoint: .top,
                            endPoint: .center
                        ),
                        lineWidth: 1
                    )
                }
            }
            .shadow(color: isLit ? tint.opacity(0.35) : .clear, radius: 5, y: 2)
            .scaleEffect(isPressed ? 0.94 : 1)
            .opacity(isEnabled ? 1 : 0.35)
            .contentShape(tile)
            .onHover { isHovering = $0 && isEnabled }
            .animation(.easeOut(duration: 0.12), value: hovering)
            .animation(.easeOut(duration: 0.12), value: isPressed)
            .animation(.easeOut(duration: 0.16), value: isActive)
    }

    private var glyph: Color {
        if isLit { return .white }
        if isActive { return tint }
        return hovering || isPressed ? .primary : .primary.opacity(0.75)
    }

    private var fill: Color {
        if isLit { return tint.opacity(isPressed ? 0.85 : 1) }
        if isActive { return tint.opacity(isPressed ? 0.26 : hovering ? 0.22 : 0.16) }
        if isPressed { return .primary.opacity(0.14) }
        return hovering ? .primary.opacity(0.08) : .clear
    }
}

/// A tool-strip button: the shared tile, a tooltip, and press feedback.
struct SidebarIconButton: View {
    let icon: String
    let label: String
    /// Hover text when it should differ from the accessibility label.
    var tooltip: String?
    var isActive = false
    var tint: Color = .accentColor
    var activeStyle: SidebarActiveStyle = .filled
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Color.clear
        }
        .buttonStyle(
            SidebarButtonStyle(icon: icon, isActive: isActive, tint: tint, activeStyle: activeStyle)
        )
        .accessibilityLabel(label)
        .accessibilityAddTraits(isActive ? .isSelected : [])
        .sidebarTooltip(tooltip ?? label)
    }
}

/// A tool-strip menu: same tile, size and hover as `SidebarIconButton`.
///
/// Plain button style, not `.borderlessButton` — that style adds its own padding and
/// shrinks the tile below the other controls. Hover is read on the menu itself, since an
/// AppKit-backed menu doesn't forward it to its label.
struct SidebarMenu<Content: View>: View {
    @Environment(\.isEnabled) private var isEnabled
    @State private var isHovering = false

    let icon: String
    let label: String
    var tooltip: String?
    var isActive = false
    var tint: Color = .accentColor
    var activeStyle: SidebarActiveStyle = .filled
    @ViewBuilder let content: () -> Content

    var body: some View {
        Menu(content: content) {
            SidebarIconLabel(
                icon: icon,
                isActive: isActive,
                tint: tint,
                activeStyle: activeStyle,
                hoverOverride: isHovering && isEnabled
            )
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .frame(width: UI.toolButtonSize, height: UI.toolButtonSize)
        .contentShape(Rectangle())
        .onHover { isHovering = $0 }
        .accessibilityLabel(label)
        .accessibilityAddTraits(isActive ? .isSelected : [])
        .sidebarTooltip(tooltip ?? label)
    }
}

private struct SidebarButtonStyle: ButtonStyle {
    let icon: String
    let isActive: Bool
    let tint: Color
    let activeStyle: SidebarActiveStyle

    func makeBody(configuration: Configuration) -> some View {
        SidebarIconLabel(
            icon: icon,
            isActive: isActive,
            tint: tint,
            activeStyle: activeStyle,
            isPressed: configuration.isPressed
        )
    }
}

private struct SidebarTooltip: ViewModifier {
    let text: String
    @State private var isPresented = false
    @State private var presentationTask: Task<Void, Never>?

    /// Clears the strip's trailing edge with a small gap.
    private var tooltipOffset: CGFloat {
        (UI.toolSidebarWidth + UI.toolButtonSize) / 2 + 8
    }

    func body(content: Content) -> some View {
        content
            .onHover { isHovering in
                presentationTask?.cancel()
                if isHovering {
                    presentationTask = Task {
                        try? await Task.sleep(for: .milliseconds(250))
                        guard !Task.isCancelled else { return }
                        await MainActor.run { isPresented = true }
                    }
                } else {
                    isPresented = false
                }
            }
            .overlay(alignment: .leading) {
                if isPresented {
                    Text(text)
                        .font(.callout)
                        .fixedSize()
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
                        .overlay {
                            RoundedRectangle(cornerRadius: 8)
                                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
                        }
                        .shadow(color: .black.opacity(0.14), radius: 7, y: 3)
                        .offset(x: tooltipOffset)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }
            }
            .zIndex(isPresented ? 1 : 0)
            .onDisappear {
                presentationTask?.cancel()
            }
    }
}

extension View {
    func sidebarTooltip(_ text: String) -> some View {
        modifier(SidebarTooltip(text: text))
    }
}

#Preview {
    ToolSidebar(activeTool: .trendLine, onSelect: { _ in }) {
        EmptyView()
    }
    .frame(height: 300)
}
