import SwiftUI

/// The Style section of a script's settings: one row per output the script draws, with a visibility
/// button and, where the script uses one fixed color or width, a color well and a width stepper. A row
/// the user changed shows a revert button. Stateless like `PineInputsView`: it reads `overrides` and
/// reports each change through `onChange`.
struct PineStyleView: View {
    let rows: [PineStyleRow]
    let overrides: [String: String]
    let onChange: ([String: String]) -> Void

    private enum Column {
        static let width: CGFloat = 78
        static let color: CGFloat = 38
        static let button: CGFloat = 24
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            VStack(spacing: 0) {
                ForEach(rows) { row in
                    if row.id != rows.first?.id { Divider().opacity(0.5) }
                    rowView(row)
                }
            }
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12))
            .overlay {
                RoundedRectangle(cornerRadius: 12).stroke(.separator.opacity(0.45), lineWidth: 1)
            }
            if Self.hasPerBarRows(rows) {
                Text("Outputs without a color well take their color bar by bar from the script.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .padding(.horizontal, 4)
            }
        }
    }

    /// Whether any row's color is computed by the script, so the footnote is worth showing.
    static func hasPerBarRows(_ rows: [PineStyleRow]) -> Bool {
        rows.contains { $0.defaultColor == nil && $0.kind != .candle }
    }

    private var style: PineStyleOverrides { PineStyleOverrides(overrides) }

    private func rowView(_ row: PineStyleRow) -> some View {
        let hidden = style.isHidden(row.kind, row.outputID)
        return HStack(spacing: 10) {
            Image(systemName: Self.icon(for: row.kind))
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 18)
            Text(row.title)
                .font(.subheadline.weight(.medium))
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(hidden ? .tertiary : .primary)
                .help(row.title)
            Spacer(minLength: 8)
            Group {
                if row.defaultWidth != nil {
                    widthStepper(row)
                } else {
                    Color.clear
                }
            }
            .frame(width: Column.width, alignment: .trailing)
            Group {
                if let color = row.defaultColor {
                    colorWell(row, defaultColor: color)
                } else {
                    Color.clear
                }
            }
            .frame(width: Column.color, alignment: .center)
            .disabled(hidden)
            visibilityButton(row, hidden: hidden)
            revertButton(row)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .animation(.easeOut(duration: 0.12), value: hidden)
    }

    private func visibilityButton(_ row: PineStyleRow, hidden: Bool) -> some View {
        SettingsIconButton(
            systemImage: hidden ? "eye.slash" : "eye",
            label: hidden ? "Show \(row.title)" : "Hide \(row.title)",
            action: {
                onChange(
                    PineStyleOverrides.setting(
                        overrides, row.kind, row.outputID, .visible, to: hidden ? nil : "false"))
            }
        )
        .frame(width: Column.button + 4)
    }

    /// Takes a row back to the script's own look; kept in the layout (blank when unchanged) so rows align.
    private func revertButton(_ row: PineStyleRow) -> some View {
        Group {
            if style.hasChanges(row) {
                SettingsIconButton(
                    systemImage: "arrow.uturn.backward", label: "Revert \(row.title) to the script's style",
                    action: { onChange(PineStyleOverrides.cleared(overrides, row)) })
            } else {
                Color.clear
            }
        }
        .frame(width: Column.button + 4)
    }

    private func colorWell(_ row: PineStyleRow, defaultColor: UInt32) -> some View {
        ColorPicker(
            "\(row.title) color",
            selection: Binding(
                get: { Color(pineRGBA: style.color(row.kind, row.outputID) ?? defaultColor) },
                set: {
                    guard let rgba = $0.pineRGBA else { return }
                    onChange(
                        PineStyleOverrides.setting(
                            overrides, row.kind, row.outputID, .color,
                            to: rgba == defaultColor ? nil : PineStyleOverrides.hex(rgba)))
                }), supportsOpacity: true
        )
        .labelsHidden()
    }

    private func widthStepper(_ row: PineStyleRow) -> some View {
        let width = Binding(
            get: { style.width(row.kind, row.outputID) ?? row.defaultWidth ?? 1 },
            set: { value in
                onChange(
                    PineStyleOverrides.setting(
                        overrides, row.kind, row.outputID, .width,
                        to: value == row.defaultWidth ? nil : String(value)))
            })
        return HStack(spacing: 2) {
            Text("\(width.wrappedValue)")
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(minWidth: 18, alignment: .trailing)
            Stepper("\(row.title) line width", value: width, in: PineStyleRows.widthRange)
                .labelsHidden()
        }
        .disabled(style.isHidden(row.kind, row.outputID))
        .help("Line width")
    }

    static func icon(for kind: PineStyleKind) -> String {
        switch kind {
        case .plot: "chart.xyaxis.line"
        case .hline: "minus"
        case .fill: "rectangle.fill"
        case .marker: "triangle"
        case .candle: "chart.bar"
        case .bgcolor: "rectangle.dashed"
        case .barcolor: "paintbrush"
        }
    }
}
