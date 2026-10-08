import SwiftUI

/// The ways to begin a replay, shown in the bar before a start is chosen: one filled primary
/// action (click a candle) and a segmented group for the rest. It sheds its labels, then
/// collapses into a menu, as the bar narrows.
struct ReplayStartOptions: View {
    let onSelectOnChart: () -> Void
    let onChooseDate: () -> Void
    let onRandomBar: () -> Void
    let onFirstBar: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            layout(labels: true)
            layout(labels: false)
            menu
        }
    }

    // MARK: Layouts

    private func layout(labels: Bool) -> some View {
        HStack(spacing: 8) {
            primary(showsTitle: labels)
            if labels {
                Text("or")
                    .font(.system(size: 10.5))
                    .foregroundStyle(.tertiary)
            }
            HStack(spacing: 0) {
                segment(
                    "Date & Time", systemImage: "calendar", showsTitle: labels,
                    help: "Pick a date and time to start from", action: onChooseDate)
                divider
                segment(
                    "Random", systemImage: "dice", showsTitle: labels,
                    help: "Start from a random bar, leaving room to play forward", action: onRandomBar)
                divider
                segment(
                    "First Bar", systemImage: "backward.end", showsTitle: labels,
                    help: "Start from the first bar the chart has loaded", action: onFirstBar)
            }
            .background(
                Color.primary.opacity(0.06),
                in: RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous)
            )
            .clipShape(RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous))
        }
        .fixedSize()
    }

    private var menu: some View {
        Menu {
            Button("Select Bar on Chart", systemImage: "cursorarrow.click.2", action: onSelectOnChart)
            Divider()
            Button("Date & Time…", systemImage: "calendar", action: onChooseDate)
            Button("Random Bar", systemImage: "dice", action: onRandomBar)
            Button("First Bar", systemImage: "backward.end", action: onFirstBar)
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "play.fill").font(.system(size: 9, weight: .bold))
                Text("Start").font(.system(size: 11.5, weight: .semibold))
                Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold))
            }
            .foregroundStyle(.white)
            .padding(.horizontal, 11)
            .padding(.vertical, 5)
            .background(Capsule().fill(ReplayStyle.accent))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Choose where the replay starts")
        .accessibilityLabel("Start replay")
    }

    // MARK: Pieces

    private func primary(showsTitle: Bool) -> some View {
        Button(action: onSelectOnChart) {
            HStack(spacing: 6) {
                Image(systemName: "cursorarrow.click.2").font(.system(size: 11, weight: .semibold))
                if showsTitle {
                    Text("Select Bar on Chart").font(.system(size: 11.5, weight: .semibold))
                } else {
                    Text("Select").font(.system(size: 11.5, weight: .semibold))
                }
            }
        }
        .buttonStyle(PrimaryStyle())
        .help("Click a candle on a chart to start the replay there")
        .accessibilityLabel("Select bar on chart")
    }

    private func segment(
        _ title: String, systemImage: String, showsTitle: Bool, help: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Image(systemName: systemImage)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(ReplayStyle.accent)
                if showsTitle {
                    Text(title).font(.system(size: 11.5, weight: .medium))
                }
            }
        }
        .buttonStyle(SegmentStyle())
        .help(help)
        .accessibilityLabel(title)
    }

    private var divider: some View {
        Color.primary.opacity(0.1).frame(width: 1, height: 14)
    }

    // MARK: Styles

    /// The filled accent capsule: lifts on hover, sinks on press, dims when disabled.
    private struct PrimaryStyle: ButtonStyle {
        @Environment(\.isEnabled) private var isEnabled
        @State private var isHovering = false

        func makeBody(configuration: Configuration) -> some View {
            configuration.label
                .foregroundStyle(.white)
                .padding(.horizontal, 12)
                .padding(.vertical, 5)
                .background(
                    Capsule().fill(ReplayStyle.accent.opacity(configuration.isPressed ? 0.8 : isHovering ? 1 : 0.92))
                )
                .overlay(Capsule().strokeBorder(.white.opacity(0.18), lineWidth: 0.5))
                .scaleEffect(configuration.isPressed ? 0.97 : 1)
                .opacity(isEnabled ? 1 : 0.4)
                .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
                .animation(.easeOut(duration: 0.12), value: isHovering)
                .onHover { isHovering = $0 }
                .contentShape(Capsule())
        }
    }

    /// One cell of the segmented group: a soft highlight on hover, a stronger one on press.
    private struct SegmentStyle: ButtonStyle {
        @Environment(\.isEnabled) private var isEnabled
        @State private var isHovering = false

        func makeBody(configuration: Configuration) -> some View {
            configuration.label
                .foregroundStyle(.primary)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Color.primary.opacity(configuration.isPressed ? 0.14 : isHovering ? 0.08 : 0))
                .opacity(isEnabled ? 1 : 0.4)
                .animation(.easeOut(duration: 0.1), value: isHovering)
                .onHover { isHovering = $0 }
                .contentShape(Rectangle())
        }
    }
}
