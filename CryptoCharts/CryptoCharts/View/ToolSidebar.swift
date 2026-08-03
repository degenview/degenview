import SwiftUI

/// Vertical strip of drawing tools down the left edge of the window.
///
/// Sits outside the chart column, so the card-height math in `ContentView` is
/// unaffected and the strip spans the empty state too. One tool today; the layout is
/// a stack so more can be added under it.
struct ToolSidebar: View {
    let activeTool: ChartTool
    let onSelect: (ChartTool) -> Void

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 4) {
                ToolButton(
                    icon: "line.diagonal",
                    label: "Trend line",
                    isActive: activeTool == .trendLine
                ) {
                    onSelect(.trendLine)
                }
                Spacer(minLength: 0)
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
        .help(label)
    }
}

#Preview {
    ToolSidebar(activeTool: .trendLine, onSelect: { _ in })
        .frame(height: 300)
}
