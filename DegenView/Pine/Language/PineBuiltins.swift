import Foundation

/// Builtin constants shared by compile-time input folding and the runtime.
enum PineBuiltins {
    static let colors: [String: UInt32] = [
        "color.aqua": 0x00bc_d4ff, "color.black": 0x0000_00ff, "color.blue": 0x2196_f3ff,
        "color.fuchsia": 0xe040_fbff, "color.gray": 0x787b_86ff, "color.green": 0x4caf_50ff,
        "color.lime": 0x00e6_76ff, "color.maroon": 0x880e_4fff, "color.navy": 0x0d47_a1ff,
        "color.olive": 0x8277_17ff, "color.orange": 0xff98_00ff, "color.purple": 0x9c27_b0ff,
        "color.red": 0xf236_45ff, "color.silver": 0xb2b5_beff, "color.teal": 0x0089_7bff,
        "color.white": 0xffff_ffff, "color.yellow": 0xffeb_3bff,
    ]

    /// Named constants that are not colors. Enumeration-like values are their own name, so a
    /// script compares and passes them exactly as it would in Pine; `display.*` and
    /// `dayofweek.*` are integers because scripts do arithmetic on them.
    static let constants: [String: PineRuntimeValue] = {
        var table: [String: PineRuntimeValue] = [
            "display.none": .int(PineDisplay.hidden.rawValue), "display.pane": .int(PineDisplay.pane.rawValue),
            "display.data_window": .int(PineDisplay.dataWindow.rawValue),
            "display.price_scale": .int(PineDisplay.priceScale.rawValue),
            "display.status_line": .int(PineDisplay.statusLine.rawValue),
            "display.all": .int(PineDisplay.all.rawValue),
            "dayofweek.sunday": .int(1), "dayofweek.monday": .int(2), "dayofweek.tuesday": .int(3),
            "dayofweek.wednesday": .int(4), "dayofweek.thursday": .int(5), "dayofweek.friday": .int(6),
            "dayofweek.saturday": .int(7), "math.pi": .float(Double.pi), "math.e": .float(M_E),
            "math.phi": .float((1 + 5.0.squareRoot()) / 2),
        ]
        let names = [
            "strategy.long", "strategy.short", "strategy.cash", "strategy.fixed",
            "strategy.percent_of_equity", "strategy.commission.percent",
            "strategy.commission.cash_per_order", "strategy.commission.cash_per_contract",
            "strategy.oca.none", "strategy.oca.cancel", "strategy.oca.reduce",
            "format.inherit", "format.price", "format.volume", "format.percent", "format.mintick",
            "order.ascending", "order.descending",
        ]
        for name in names { table[name] = .string(name) }
        for frequency in [PineAlertFrequency.all, .oncePerBar, .oncePerBarClose] {
            let name = PineAlertFrequency.pinePrefix + frequency.rawValue
            table[name] = .string(name)
        }
        return table
    }()

    /// The value type an `input.*` function declares; anything unlisted is a string.
    static func inputType(function: String) -> PineValueType {
        switch function {
        case "input.int": .int
        case "input.float": .float
        case "input.bool": .bool
        case "input.color": .color
        case "input.time": .time
        default: .string
        }
    }

    /// Pine transparency runs from 0 (opaque) to 100 (invisible); the alpha byte is the rest.
    private static let maxTransparency = 100.0
    private static let alphaPerPercent = 2.55

    /// A non-finite transparency (`na`, NaN) counts as opaque.
    static func withTransparency(_ rgba: UInt32, _ transparency: Double) -> UInt32 {
        let clamped = transparency.isFinite ? min(maxTransparency, max(0, transparency)) : 0
        let alpha = UInt32(((maxTransparency - clamped) * alphaPerPercent).rounded())
        return (rgba & 0xFFFF_FF00) | alpha
    }

    /// `color.from_gradient`: each RGBA channel runs linearly from `bottomColor` at `bottom` to
    /// `topColor` at `top`, and a value outside that range takes the nearer end.
    static func gradient(
        _ value: Double, bottom: Double, top: Double, bottomColor: UInt32, topColor: UInt32
    ) -> UInt32 {
        guard top != bottom else { return value >= top ? topColor : bottomColor }
        let t = min(1, max(0, (value - bottom) / (top - bottom)))
        var result: UInt32 = 0
        for shift in stride(from: 24, through: 0, by: -8) {
            let from = Double((bottomColor >> UInt32(shift)) & 0xFF)
            let to = Double((topColor >> UInt32(shift)) & 0xFF)
            result |= UInt32((from + (to - from) * t).rounded()) << UInt32(shift)
        }
        return result
    }

    static func rgb(_ r: Double, _ g: Double, _ b: Double, _ transparency: Double) -> UInt32 {
        func channel(_ v: Double) -> UInt32 { UInt32(min(255, max(0, v.isFinite ? v : 0))) }
        return withTransparency(
            (channel(r) << 24) | (channel(g) << 16) | (channel(b) << 8) | 0xFF, transparency)
    }
}
