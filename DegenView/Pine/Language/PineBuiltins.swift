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
            "display.none": .int(PineDisplay.none), "display.pane": .int(PineDisplay.pane),
            "display.data_window": .int(PineDisplay.dataWindow),
            "display.price_scale": .int(PineDisplay.priceScale),
            "display.status_line": .int(PineDisplay.statusLine), "display.all": .int(PineDisplay.all),
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
            "alert.freq_all", "alert.freq_once_per_bar", "alert.freq_once_per_bar_close",
            "format.inherit", "format.price", "format.volume", "format.percent", "format.mintick",
            "order.ascending", "order.descending",
        ]
        for name in names { table[name] = .string(name) }
        return table
    }()

    /// Pine transparency is 0 (opaque) … 100 (invisible).
    static func withTransparency(_ rgba: UInt32, _ transparency: Double) -> UInt32 {
        (rgba & 0xFFFF_FF00) | UInt32(((100 - min(100, max(0, transparency))) * 2.55).rounded())
    }

    static func rgb(_ r: Double, _ g: Double, _ b: Double, _ transparency: Double) -> UInt32 {
        func channel(_ v: Double) -> UInt32 { UInt32(min(255, max(0, v.isFinite ? v : 0))) }
        return withTransparency(
            (channel(r) << 24) | (channel(g) << 16) | (channel(b) << 8) | 0xFF, transparency)
    }
}
