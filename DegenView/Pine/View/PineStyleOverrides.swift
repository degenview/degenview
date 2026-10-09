import Foundation

/// The kinds of script output the Style section lists. Each kind numbers its outputs by call site
/// separately, so a key needs the kind as well as the id.
enum PineStyleKind: String, CaseIterable, Sendable {
    case plot, hline, fill, marker, candle, bgcolor, barcolor
}

/// What a user can change about one output.
enum PineStyleAttribute: String, Sendable {
    case visible, color, width
}

/// A typed view over `ChartScriptInstance.styleOverrides`.
///
/// Keys are `"<kind>.<siteId>.<attribute>"`. `visible` is stored only as `"false"`, `color` as an
/// 8-digit RRGGBBAA hex string and `width` as an integer. A missing key means "as the script has it",
/// and a key that matches no output is ignored.
struct PineStyleOverrides: Equatable, Sendable {
    let raw: [String: String]

    init(_ raw: [String: String]) { self.raw = raw }

    static func key(_ kind: PineStyleKind, _ id: Int, _ attribute: PineStyleAttribute) -> String {
        "\(kind.rawValue).\(id).\(attribute.rawValue)"
    }

    func isHidden(_ kind: PineStyleKind, _ id: Int) -> Bool {
        raw[Self.key(kind, id, .visible)] == "false"
    }

    func color(_ kind: PineStyleKind, _ id: Int) -> UInt32? {
        raw[Self.key(kind, id, .color)].flatMap { UInt32($0, radix: 16) }
    }

    func width(_ kind: PineStyleKind, _ id: Int) -> Int? {
        raw[Self.key(kind, id, .width)].flatMap { Int($0) }.map { min(max($0, 1), 20) }
    }

    /// `raw` with one attribute set, or removed when `value` is nil.
    static func setting(
        _ raw: [String: String], _ kind: PineStyleKind, _ id: Int, _ attribute: PineStyleAttribute,
        to value: String?
    ) -> [String: String] {
        var copy = raw
        copy[key(kind, id, attribute)] = value
        return copy
    }

    /// Whether the user changed anything about `row`.
    func hasChanges(_ row: PineStyleRow) -> Bool {
        raw.keys.contains { $0.hasPrefix(row.id + ".") }
    }

    /// `raw` without any choice about `row`.
    static func cleared(_ raw: [String: String], _ row: PineStyleRow) -> [String: String] {
        raw.filter { !$0.key.hasPrefix(row.id + ".") }
    }

    static func hex(_ rgba: UInt32) -> String { String(format: "%08X", rgba) }

    /// The overrides that still belong to one of `rows`; the rest are left over from an edited script.
    static func pruned(_ raw: [String: String], keeping rows: [PineStyleRow]) -> [String: String] {
        let prefixes = Set(rows.map { $0.id + "." })
        return raw.filter { entry in prefixes.contains { entry.key.hasPrefix($0) } }
    }

    /// A color the user picked replaces a visible one; `na` (nil) and fully transparent stay as they are.
    fileprivate static func recolored(_ color: UInt32?, to override: UInt32) -> UInt32? {
        guard let color, color & 0xFF != 0 else { return color }
        return override
    }

    fileprivate static func recolored(_ color: UInt32, to override: UInt32) -> UInt32 {
        color & 0xFF != 0 ? override : color
    }
}

extension PineVisualOutput {
    /// This output with the user's Style choices applied. A hidden plot, marker or candle set stays in
    /// the output with its pane display cleared, because a `fill()` may still reference a hidden plot;
    /// the other kinds are dropped.
    func applying(styleOverrides raw: [String: String]) -> PineVisualOutput {
        guard !raw.isEmpty else { return self }
        let style = PineStyleOverrides(raw)
        var result = self

        for index in result.plots.indices {
            let id = result.plots[index].id
            if style.isHidden(.plot, id) { result.plots[index].display.remove(.pane) }
            if let color = style.color(.plot, id) {
                result.plots[index].color = PineStyleOverrides.recolored(result.plots[index].color, to: color)
                result.plots[index].colors = result.plots[index].colors.map {
                    PineStyleOverrides.recolored($0, to: color)
                }
            }
            if let width = style.width(.plot, id) { result.plots[index].lineWidth = width }
        }

        result.hlines.removeAll { style.isHidden(.hline, $0.id) }
        for index in result.hlines.indices {
            if let color = style.color(.hline, result.hlines[index].id) { result.hlines[index].color = color }
        }

        result.fills.removeAll { style.isHidden(.fill, $0.id) }
        for index in result.fills.indices {
            guard let color = style.color(.fill, result.fills[index].id) else { continue }
            result.fills[index].colors = result.fills[index].colors.map {
                PineStyleOverrides.recolored($0, to: color)
            }
        }

        for index in result.markers.indices {
            let id = result.markers[index].id
            if style.isHidden(.marker, id) { result.markers[index].display.remove(.pane) }
            if let color = style.color(.marker, id) {
                result.markers[index].color = PineStyleOverrides.recolored(result.markers[index].color, to: color)
                result.markers[index].colors = result.markers[index].colors.map {
                    PineStyleOverrides.recolored($0, to: color)
                }
            }
        }

        for index in result.candles.indices where style.isHidden(.candle, result.candles[index].id) {
            result.candles[index].display.remove(.pane)
        }

        result.backgrounds = Self.styled(result.backgrounds, as: .bgcolor, by: style)
        result.barColors = Self.styled(result.barColors, as: .barcolor, by: style)
        return result
    }

    private static func styled(
        _ outputs: [PineColorOutput], as kind: PineStyleKind, by style: PineStyleOverrides
    ) -> [PineColorOutput] {
        outputs.filter { !style.isHidden(kind, $0.id) }.map { output in
            guard let color = style.color(kind, output.id) else { return output }
            var copy = output
            copy.colors = copy.colors.map { PineStyleOverrides.recolored($0, to: color) }
            return copy
        }
    }
}
