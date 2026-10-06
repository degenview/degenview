import SwiftUI

/// Style editor for the selected brush stroke: colour, width, opacity, lock, delete.
///
/// A menu pick is one undo step. The opacity slider writes live without history and
/// records a single step when the drag ends, like the Fibonacci settings.
struct BrushEditor: View {
    @ObservedObject var viewModel: ChartViewModel
    let brushID: UUID
    let onDismiss: () -> Void

    private var stroke: BrushDrawing? {
        viewModel.brushes.first { $0.id == brushID }
    }

    var body: some View {
        if let stroke {
            HStack(spacing: 8) {
                colorMenu(stroke)
                widthMenu(stroke)
                opacitySlider(stroke)

                Button {
                    var updated = stroke
                    updated.isLocked.toggle()
                    viewModel.updateBrush(updated)
                } label: {
                    Image(systemName: stroke.isLocked ? "lock.fill" : "lock.open")
                        .font(.caption.bold())
                }
                .buttonStyle(.plain)
                .foregroundStyle(stroke.isLocked ? Color.accentColor : .secondary)
                .accessibilityLabel(stroke.isLocked ? "Unlock brush stroke" : "Lock brush stroke")
                .help(stroke.isLocked ? "Unlock" : "Lock")

                Button {
                    _ = viewModel.removeBrush(id: brushID)
                } label: {
                    Image(systemName: "trash")
                        .font(.caption.bold())
                }
                .buttonStyle(.plain)
                .foregroundStyle(.red)
                .accessibilityLabel("Delete brush stroke")
                .help("Delete brush stroke")

                Button(action: onDismiss) {
                    Image(systemName: "xmark")
                        .font(.caption.bold())
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .accessibilityLabel("Close brush editor")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .stroke(.secondary.opacity(0.25), lineWidth: 1)
            }
            .shadow(radius: 4, y: 2)
        }
    }

    private func colorMenu(_ stroke: BrushDrawing) -> some View {
        Menu {
            ForEach(TrendLineColor.allCases, id: \.self) { option in
                Button {
                    var updated = stroke
                    updated.color = option
                    viewModel.updateBrush(updated)
                } label: {
                    HStack {
                        Circle().fill(option.color).frame(width: 10, height: 10)
                        Text(option.title)
                    }
                }
            }
        } label: {
            HStack(spacing: 6) {
                Circle().fill(stroke.color.color).frame(width: 10, height: 10)
                Text(stroke.color.title)
                Image(systemName: "chevron.down")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .frame(minWidth: 78)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
        }
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .accessibilityLabel("Brush color")
    }

    private func widthMenu(_ stroke: BrushDrawing) -> some View {
        Menu {
            ForEach(BrushStyle.widths, id: \.self) { width in
                Button {
                    var updated = stroke
                    updated.lineWidth = width
                    viewModel.updateBrush(updated)
                } label: {
                    Text("\(Int(width)) pt")
                }
            }
        } label: {
            HStack(spacing: 6) {
                Capsule()
                    .fill(stroke.color.color)
                    .frame(width: 30, height: max(1, CGFloat(stroke.lineWidth)))
                    .frame(height: 10)
                Text("\(Int(stroke.lineWidth))")
                    .monospacedDigit()
                Image(systemName: "chevron.down")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
        }
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .accessibilityLabel("Brush width")
    }

    private func opacitySlider(_ stroke: BrushDrawing) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "circle.lefthalf.filled")
                .font(.caption)
                .foregroundStyle(.secondary)
            Slider(
                value: Binding(
                    get: { stroke.opacity },
                    set: { value in
                        var updated = stroke
                        updated.opacity = value
                        viewModel.updateBrush(updated, recordUndo: false)
                    }
                ),
                in: 0.1...1
            ) { editing in
                if editing {
                    viewModel.beginBrushSettingsEdit(id: brushID)
                } else {
                    viewModel.endBrushSettingsEdit(id: brushID)
                }
            }
            .frame(width: 80)
            Text("\(Int((stroke.opacity * 100).rounded()))%")
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 34, alignment: .trailing)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Brush opacity")
    }
}
