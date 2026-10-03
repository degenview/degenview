import SwiftUI

/// Choose when a replay begins: a date and time inside the loaded chart, with shortcuts for common
/// starting points and a preview of the bar it will snap to.
struct ReplayStartSheet: View {
    /// What the loaded chart covers. Nil while it has no data.
    let range: ClosedRange<Date>?
    /// The chart bar a given date falls on.
    let snap: (Date) -> Date?
    let onStart: (Date) -> Void
    let onCancel: () -> Void

    @State private var date: Date

    init(
        range: ClosedRange<Date>?, initial: Date?, snap: @escaping (Date) -> Date?,
        onStart: @escaping (Date) -> Void, onCancel: @escaping () -> Void
    ) {
        self.range = range
        self.snap = snap
        self.onStart = onStart
        self.onCancel = onCancel
        let midpoint = range.map {
            $0.lowerBound.addingTimeInterval($0.upperBound.timeIntervalSince($0.lowerBound) / 2)
        }
        let seed = initial ?? midpoint ?? Date()
        _date = State(initialValue: range.map { min(max(seed, $0.lowerBound), $0.upperBound) } ?? seed)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            SheetHeader(
                systemImage: "clock.arrow.circlepath", title: "Start Replay",
                subtitle: "Pick the moment the chart rewinds to.")
            presets
            picker
            snapPreview
            HStack {
                Spacer()
                Button("Cancel", role: .cancel, action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button("Start Replay") { onStart(date) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(range == nil)
            }
        }
        .padding(20)
        .frame(width: 400)
    }

    // MARK: Pieces

    private var presets: some View {
        HStack(spacing: 6) {
            preset("Start of data", fraction: 0)
            preset("25%", fraction: 0.25)
            preset("50%", fraction: 0.5)
            preset("75%", fraction: 0.75)
            Button {
                date = random()
            } label: {
                Label("Random", systemImage: "dice")
                    .font(.system(size: 11.5, weight: .medium))
                    .padding(.horizontal, 9)
                    .padding(.vertical, 4)
                    .background(
                        Color.primary.opacity(0.06),
                        in: RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(range == nil)
        }
    }

    private func preset(_ title: String, fraction: Double) -> some View {
        Button {
            guard let range else { return }
            date = range.lowerBound.addingTimeInterval(
                range.upperBound.timeIntervalSince(range.lowerBound) * fraction)
        } label: {
            Text(title)
                .font(.system(size: 11.5, weight: .medium))
                .padding(.horizontal, 9)
                .padding(.vertical, 4)
                .background(
                    Color.primary.opacity(0.06),
                    in: RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(range == nil)
    }

    @ViewBuilder
    private var picker: some View {
        if let range {
            DatePicker("Replay starts at", selection: $date, in: range)
                .datePickerStyle(.graphical)
                .labelsHidden()
        } else {
            Text("Load a chart with data first.")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var snapPreview: some View {
        if let bar = snap(date) {
            Label {
                Text("Snaps to the bar at \(ReplayStyle.fullText(bar))")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            } icon: {
                Image(systemName: "scope").foregroundStyle(ReplayStyle.accent)
            }
        }
    }

    private func random() -> Date {
        guard let range else { return date }
        // Leave the last fifth for something to replay, as the menu's Random Bar does.
        let span = range.upperBound.timeIntervalSince(range.lowerBound)
        return range.lowerBound.addingTimeInterval(Double.random(in: 0...(span * 0.8)))
    }
}
