import Foundation

/// What the type checker knows about builtin identifiers and functions. Anything not listed is
/// `unknown`, which the checker lets through.
enum PineBuiltinTypes {
    typealias Inferred = PineTypeChecker.Inferred

    // MARK: - Identifiers

    static let floatSeries: Set<String> = [
        "open", "high", "low", "close", "volume", "hl2", "hlc3", "ohlc4", "hlcc4", "ta.tr",
        "strategy.position_size", "strategy.position_avg_price", "strategy.equity",
        "strategy.netprofit", "strategy.openprofit", "strategy.initial_capital",
        "strategy.grossprofit", "strategy.grossloss",
    ]

    static let intSeries: Set<String> = [
        "time", "time_close", "bar_index", "last_bar_index", "last_bar_time", "timenow",
        "time_tradingday", "year", "month", "dayofmonth", "hour", "minute", "second", "dayofweek",
        "strategy.closedtrades", "strategy.opentrades", "strategy.wintrades", "strategy.losstrades",
    ]

    static let simpleStrings: Set<String> = [
        "syminfo.ticker", "syminfo.tickerid", "syminfo.currency", "syminfo.type", "syminfo.root",
        "syminfo.prefix", "syminfo.timezone",
    ]

    static let simpleFloats: Set<String> = ["syminfo.mintick", "syminfo.pointvalue"]
    static let simpleColors: Set<String> = ["chart.fg_color", "chart.bg_color"]

    static func identifier(_ name: String) -> Inferred {
        switch PineBuiltins.constants[name] {
        case .int?: return .known(.int, .constant)
        case .float?: return .known(.float, .constant)
        case .string?: return .known(.string, .constant)
        default: break
        }
        if PineBuiltins.colors[name] != nil { return .known(.color, .constant) }
        if floatSeries.contains(name) { return .known(.float, .series) }
        if intSeries.contains(name) { return .known(.int, .series) }
        if simpleStrings.contains(name) { return .known(.string, .simple) }
        if simpleFloats.contains(name) { return .known(.float, .simple) }
        if simpleColors.contains(name) { return .known(.color, .simple) }
        return name.hasPrefix("barstate.") ? .known(.bool, .series) : .unknown
    }

    // MARK: - Functions

    /// The type `input.*` returns, or nil for functions that are not typed inputs.
    static func inputResult(_ name: String) -> PineValueType? {
        switch name {
        case "input.int", "input.time": .int
        case "input.float", "input.source": .float
        case "input.bool": .bool
        case "input.string", "input.session": .string
        case "input.color": .color
        default: nil
        }
    }

    /// Functions whose result is always a series of this type.
    private static let seriesResults: [String: PineValueType] = {
        var table: [String: PineValueType] = [
            "line.new": .line, "label.new": .label, "box.new": .box, "table.new": .table,
            "array.from": .array, "array.copy": .array, "array.slice": .array,
        ]
        for name in [
            "ta.sma", "ta.ema", "ta.rma", "ta.wma", "ta.rsi", "ta.atr", "ta.tr", "ta.stdev",
            "ta.highest", "ta.lowest", "ta.mom", "ta.roc", "ta.cum", "ta.pivothigh", "ta.pivotlow",
            "ta.median", "ta.range", "ta.variance", "ta.dev", "ta.swma", "ta.cmo", "ta.cci", "ta.hma",
            "ta.percentrank", "ta.correlation", "ta.linreg",
        ] {
            table[name] = .float
        }
        for name in [
            "ta.cross", "ta.crossover", "ta.crossunder", "ta.rising", "ta.falling", "timeframe.change",
        ] {
            table[name] = .bool
        }
        return table
    }()

    /// Functions whose result type is fixed but whose qualifier is that of their arguments.
    private static let propagatingResults: [String: PineValueType] = {
        var table: [String: PineValueType] = [
            "int": .int, "float": .float, "bool": .bool, "na": .bool, "timestamp": .int,
            "color.new": .color, "color.rgb": .color, "color.from_gradient": .color,
            "color.r": .float, "color.g": .float, "color.b": .float, "color.t": .float,
            "str.length": .int, "str.tonumber": .float, "math.floor": .int, "math.ceil": .int,
        ]
        for name in [
            "str.tostring", "timeframe.from_seconds", "str.format", "str.upper", "str.lower", "str.trim",
            "str.replace_all",
            "str.substring",
        ] {
            table[name] = .string
        }
        for name in ["str.contains", "str.startswith", "str.endswith"] { table[name] = .bool }
        for name in [
            "math.sqrt", "math.pow", "math.log", "math.log10", "math.exp", "math.sin", "math.cos",
            "math.tan", "math.asin", "math.acos", "math.atan", "math.avg", "math.todegrees",
            "math.toradians", "math.round_to_mintick",
        ] {
            table[name] = .float
        }
        return table
    }()

    /// Result of calling `name`, given the inferred types of its arguments and their combined
    /// qualifier.
    static func call(
        _ name: String, arguments: [PineArgument], values: [Inferred], qualifier: PineQualifier?
    ) -> Inferred {
        if let type = seriesResults[name] { return .known(type, .series) }
        if let type = propagatingResults[name] { return .known(type, qualifier) }
        switch name {
        case "nz", "math.abs":
            if let type = values.first?.type, isNumeric(type) { return .known(type, qualifier) }
            return .unknown
        case "math.round": return .known(arguments.count > 1 ? .float : .int, qualifier)
        case "math.max", "math.min":
            let types = values.compactMap(\.type)
            guard types.count == values.count, !types.isEmpty, types.allSatisfy(isNumeric) else {
                return .unknown
            }
            return .known(types.allSatisfy { $0 == .int } ? .int : .float, qualifier)
        default:
            return name.hasPrefix("array.new_") ? .known(.array, .series) : .unknown
        }
    }

    private static func isNumeric(_ type: PineValueType) -> Bool { type == .int || type == .float }
}
