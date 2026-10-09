import Foundation

/// One line of the Style section: a script output the user can hide, recolor or thicken.
struct PineStyleRow: Identifiable, Equatable, Sendable {
    let kind: PineStyleKind
    let outputID: Int
    let title: String
    /// The script's color when it is one fixed color; nil when the script computes it per bar, which
    /// leaves nothing sensible to pick.
    let defaultColor: UInt32?
    /// The script's line width for the plot styles that draw one.
    let defaultWidth: Int?

    /// The key prefix shared by this row's overrides.
    var id: String { "\(kind.rawValue).\(outputID)" }
}

enum PineStyleRows {
    /// The widest line a row offers.
    static let widthRange = 1...10

    /// Every output of `output` a user can style, in the order the script made them. Outputs the script
    /// itself keeps off the pane (`display.none`) are left out.
    static func rows(for output: PineVisualOutput) -> [PineStyleRow] {
        var rows: [PineStyleRow] = []
        var counts: [PineStyleKind: Int] = [:]
        func add(
            _ kind: PineStyleKind, _ id: Int, _ title: String?, fallback: String, color: UInt32?,
            width: Int? = nil
        ) {
            counts[kind, default: 0] += 1
            let name = title.flatMap { $0.isEmpty ? nil : $0 } ?? "\(fallback) \(counts[kind] ?? 1)"
            rows.append(
                PineStyleRow(
                    kind: kind, outputID: id, title: name, defaultColor: color, defaultWidth: width))
        }

        for plot in output.plots where plot.display.contains(.pane) {
            let width = widthStyles.contains(plot.style) ? plot.lineWidth : nil
            add(.plot, plot.id, plot.title, fallback: "Plot", color: fixedColor(plot.colors), width: width)
        }
        for line in output.hlines { add(.hline, line.id, line.title, fallback: "Level", color: line.color) }
        for fill in output.fills {
            let color = fill.gradients.contains { $0 != nil } ? nil : fixedColor(fill.colors)
            add(.fill, fill.id, fill.title, fallback: "Fill", color: color)
        }
        for marker in output.markers where marker.display.contains(.pane) {
            let fallback = marker.kind == .shape ? "Shape" : "Character"
            add(.marker, marker.id, marker.title, fallback: fallback, color: fixedColor(marker.colors))
        }
        for candle in output.candles where candle.display.contains(.pane) {
            add(.candle, candle.id, candle.title, fallback: "Candles", color: nil)
        }
        for background in output.backgrounds {
            add(
                .bgcolor, background.id, background.title, fallback: "Background",
                color: fixedColor(background.colors))
        }
        for barColor in output.barColors {
            add(
                .barcolor, barColor.id, barColor.title, fallback: "Bar Color",
                color: fixedColor(barColor.colors))
        }
        return rows.sorted { ($0.outputID, $0.kind.rawValue) < ($1.outputID, $1.kind.rawValue) }
    }

    private static let widthStyles: Set<PinePlotStyle> = [.line, .stepline, .circles, .cross]

    /// The one color every visible bar uses, or nil when the script varies it (or never shows one).
    private static func fixedColor(_ colors: [UInt32?]) -> UInt32? {
        let visible = Set(colors.compactMap { $0 }.filter { $0 & 0xFF != 0 })
        return visible.count == 1 ? visible.first : nil
    }
}
