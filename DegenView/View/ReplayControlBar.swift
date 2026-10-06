import SwiftUI

/// The strip above the charts while a replay is active: status, transport, speed and resolution,
/// the timeline, the cursor's clock and the way back to live. While picking a start it swaps the
/// transport for a hint. On a narrow window the timeline drops to a second row.
struct ReplayControlBar: View {
    @ObservedObject var engine: ReplayEngine
    let onChangeStart: () -> Void
    let onCancelSelection: () -> Void
    let onReturnToLive: () -> Void
    var availableIntervals: [ReplayInterval] = [.automatic, .chartBar]
    /// The interval Auto resolved to for the primary chart, when it has granular data.
    var resolvedInterval: ReplayInterval?
    let onIntervalChanged: (ReplayInterval) -> Void
    var isPreparing = false
    var notice: String?
    let onDismissNotice: () -> Void

    var body: some View {
        VStack(spacing: 6) {
            if engine.status == .selectingStart {
                selectingRow
            } else {
                ViewThatFits(in: .horizontal) {
                    wideRow
                    narrowRows
                }
            }
            if let notice {
                ReplayNoticeChip(message: notice, onDismiss: onDismissNotice)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(.ultraThinMaterial)
        .background(ReplayStyle.accent.opacity(0.06))
        .overlay(alignment: .bottom) { ReplayStyle.accent.opacity(0.35).frame(height: 1) }
        .overlay(alignment: .top) {
            if isPreparing {
                ProgressView().progressViewStyle(.linear).tint(ReplayStyle.accent)
                    .scaleEffect(x: 1, y: 0.5).frame(height: 2)
            }
        }
    }

    // MARK: Rows

    private var wideRow: some View {
        HStack(spacing: 10) {
            ReplayStatusChip(status: engine.status, isPreparing: isPreparing)
            ReplayTransportControls(engine: engine, isPreparing: isPreparing)
            pickers
            ReplayScrubber(engine: engine, isDisabled: isPreparing)
                .frame(minWidth: 140)
            ReplayClockReadout(date: engine.currentTimestamp)
            actions
        }
    }

    private var narrowRows: some View {
        VStack(spacing: 6) {
            HStack(spacing: 8) {
                ReplayStatusChip(status: engine.status, isPreparing: isPreparing)
                ReplayTransportControls(engine: engine, isPreparing: isPreparing)
                Spacer(minLength: 4)
                ReplayClockReadout(date: engine.currentTimestamp, isCompact: true)
            }
            HStack(spacing: 8) {
                pickers
                ReplayScrubber(engine: engine, isDisabled: isPreparing)
                    .frame(minWidth: 60)
                actions
            }
        }
    }

    private var selectingRow: some View {
        HStack(spacing: 10) {
            ReplayStatusChip(status: .selectingStart)
            Image(systemName: "cursorarrow.click.2").foregroundStyle(ReplayStyle.accent)
            Text("Click a candle to start the replay")
                .font(.system(size: 12, weight: .medium))
                .lineLimit(1)
            Text("Esc to cancel")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 8)
            Button("Cancel", action: onCancelSelection)
                .controlSize(.small)
                .help("Cancel (Esc)")
                .accessibilityLabel("Cancel choosing a replay start")
        }
    }

    // MARK: Pieces

    private var pickers: some View {
        HStack(spacing: 6) {
            ReplayPickerMenu(
                caption: "Speed", valueText: (engine.session?.playbackSpeed ?? .normal).rawValue,
                options: ReplaySpeed.allCases, selection: engine.session?.playbackSpeed,
                optionTitle: { $0.rawValue }, help: "Playback speed",
                onSelect: engine.setSpeed)
            ReplayPickerMenu(
                caption: "Bars", valueText: intervalText,
                options: availableIntervals, selection: engine.session?.replayInterval,
                optionTitle: { $0.rawValue },
                help: "Replay resolution. Auto picks the finest history that fits; 1 bar steps whole chart bars.",
                onSelect: onIntervalChanged, isDisabled: isPreparing)
        }
    }

    private var intervalText: String {
        let chosen = engine.session?.replayInterval ?? .automatic
        guard chosen == .automatic, let resolvedInterval else { return chosen.rawValue }
        return "Auto · \(resolvedInterval.rawValue)"
    }

    private var actions: some View {
        HStack(spacing: 2) {
            ReplayIconButton(systemImage: "scope", label: "Choose a new starting point", action: onChangeStart)
            Button(action: onReturnToLive) {
                HStack(spacing: 5) {
                    Image(systemName: "dot.radiowaves.left.and.right").font(.system(size: 10, weight: .bold))
                    Text("Live").font(.system(size: 11.5, weight: .semibold))
                }
                .foregroundStyle(ReplayStyle.accent)
                .padding(.horizontal, 9)
                .padding(.vertical, 4)
                .overlay(
                    Capsule().stroke(ReplayStyle.accent.opacity(0.55), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .help("Leave replay and return to the live market")
            .accessibilityLabel("Return to live market")
            ReplayIconButton(systemImage: "xmark", label: "Close Replay", action: onReturnToLive)
        }
    }
}
