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
            VStack(spacing: 4) {
                ToolButton(
                    icon: "plus",
                    label: "Crosshair",
                    isActive: activeTool == .crosshair
                ) {
                    onSelect(.crosshair)
                }
                ToolButton(
                    icon: "line.diagonal",
                    label: "Trend line",
                    isActive: activeTool == .trendLine
                ) {
                    onSelect(.trendLine)
                }
                ToolButton(
                    icon: "ruler",
                    label: "Ruler",
                    isActive: activeTool == .ruler
                ) {
                    onSelect(.ruler)
                }
                Spacer(minLength: 0)
                bottomControls()
                Button {
                    openWindow(id: "alerts")
                } label: {
                    Image(systemName: "bell")
                        .font(.system(size: 13, weight: .medium))
                        .frame(width: 26, height: 26)
                        .contentShape(RoundedRectangle(cornerRadius: 5))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("View Price Alerts")
                .sidebarTooltip("View Price Alerts")
            }
            .padding(.vertical, 6)
            .frame(width: UI.toolSidebarWidth)
            .background(.regularMaterial)

            Divider()
        }
    }
}

private struct ToolButton: View {
    let icon: String
    let label: String
    let isActive: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: 13, weight: .medium))
                .frame(width: 26, height: 26)
                .foregroundStyle(isActive ? Color.white : Color.primary)
                .background(
                    RoundedRectangle(cornerRadius: 5)
                        .fill(isActive ? Color.accentColor : Color.clear)
                )
                .contentShape(RoundedRectangle(cornerRadius: 5))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .sidebarTooltip(label)
    }
}

private struct SidebarTooltip: ViewModifier {
    let text: String
    @State private var isPresented = false
    @State private var presentationTask: Task<Void, Never>?

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
                        .padding(.horizontal, 14)
                        .padding(.vertical, 9)
                        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
                        .overlay {
                            RoundedRectangle(cornerRadius: 8)
                                .stroke(.separator.opacity(0.35), lineWidth: 0.5)
                        }
                        .shadow(color: .black.opacity(0.14), radius: 7, y: 3)
                        .offset(x: 34)
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
