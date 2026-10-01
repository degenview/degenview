import Foundation

/// The small builtins that map values to values: casts, `na`/`nz`, colors, math, strings,
/// timestamps, calendar parts and `input.*`.
extension PineRuntimeSession {
    func declarationCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        .void
    }

    /// Input calls are declaration initialisers: the owning variable keys the override, so
    /// titles can change and overrides need no recompilation.
    func inputCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if let variable = inputVariables[call.site],
            let override = runtimeInput(inputs[variable], context)
        {
            return override
        }
        guard let defaultValue = call.arguments.inputDefault else { return .na }
        return try eval(defaultValue.value, &context)
    }

    private func runtimeInput(_ value: PineInputValue?, _ context: PineRuntimeContext) -> PineRuntimeValue? {
        switch value {
        case nil: nil
        case .int(let x)?: .int(x)
        case .float(let x)?: .float(x)
        case .bool(let x)?: .bool(x)
        case .string(let x)?: .string(x)
        case .color(let x)?: .color(x)
        case .source(let name)?: market(name, context)
        }
    }

    // MARK: - na and casts

    func naCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        .bool(try argument(call, 0, nil, &context) == .na)
    }

    func nzCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        if value != .na { return value }
        let replacement = try argument(call, 1, nil, &context)
        return replacement == .na ? .float(0) : replacement
    }

    func intCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        try argument(call, 0, nil, &context).number.flatMap { Int(pine: $0) }.map(PineRuntimeValue.int) ?? .na
    }

    func floatCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        try argument(call, 0, nil, &context).number.map(PineRuntimeValue.float) ?? .na
    }

    /// `time(timeframe)`: the open time of the `timeframe` bar that contains the current bar, so
    /// `ta.change(time("D"))` marks a new day. Sessions and time zones are not modelled: the argument
    /// after the timeframe is ignored. Without a usable timeframe it is the bar's own `time`.
    func timeCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let open = PineTime.milliseconds(context.bar.openTime)
        guard case .string(let text) = try argument(call, 0, "timeframe", &context), !text.isEmpty else {
            return .int(open)
        }
        guard let seconds = PineTime.seconds(ofTimeframe: text) else {
            throw PineDiagnostic.error(
                "PINE4021", .runtime, "time() needs a timeframe such as \"60\" or \"1D\".", call.range)
        }
        let start = KlineData.bucketStart(of: context.bar.openTime, interval: seconds)
        return .int(PineTime.milliseconds(start))
    }

    /// `time_close(timeframe)`: the close of the `timeframe` bar that contains the current bar.
    func timeCloseCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        guard case .string(let text) = try argument(call, 0, "timeframe", &context), !text.isEmpty else {
            return market("time_close", context) ?? .na
        }
        guard let seconds = PineTime.seconds(ofTimeframe: text) else {
            throw PineDiagnostic.error(
                "PINE4021", .runtime, "time_close() needs a timeframe such as \"60\" or \"1D\".", call.range)
        }
        let start = KlineData.bucketStart(of: context.bar.openTime, interval: seconds)
        return .int(PineTime.milliseconds(KlineData.bucketEnd(after: start, interval: seconds)))
    }

    /// `runtime.error(message)`: stops the script with the message the script chose.
    func runtimeErrorCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let message = try argument(call, 0, "message", &context).textValue ?? "runtime.error()"
        throw PineDiagnostic.error("PINE4030", .runtime, message, call.range)
    }

    /// `ticker.*` build a symbol id. Only the chart's own symbol can be served, so `standard`, `modify` and
    /// `inherit` hand back the symbol they were given (`inherit` the one it takes over), and `new` joins
    /// `prefix:symbol`.
    func tickerCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let values = try allArguments(call, &context)
        switch call.name {
        case "ticker.new":
            guard case .string(let prefix)? = values.first, case .string(let name)? = values.dropFirst().first
            else { return .na }
            return .string(prefix.isEmpty ? name : "\(prefix):\(name)")
        case "ticker.inherit": return values.dropFirst().first ?? .na
        default: return values.first ?? .na
        }
    }

    /// `max_bars_back(series, n)`: a hint about how much history to keep. The runtime keeps all of it.
    func maxBarsBackCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        .void
    }

    /// `color(na)`: the typed na scripts use to clear a color. A color passes through.
    func colorCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        if case .color = value { return value }
        return .na
    }

    /// `string(na)`: a string passes through, anything else is na.
    func stringCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        if case .string = value { return value }
        return .na
    }

    func boolCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        if case .bool = value { return value }
        return value.number.map { .bool($0 != 0) } ?? .bool(false)
    }

    /// `line(x)`, `label(x)`, `box(x)`, `table(x)`: handle casts pass the value through.
    func handleCast(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        try argument(call, 0, nil, &context)
    }

    // MARK: - Colors

    func colorNew(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        guard case .color(let c) = try argument(call, 0, nil, &context),
            let transparency = try argument(call, 1, nil, &context).number
        else { return .na }
        return .color(PineBuiltins.withTransparency(c, transparency))
    }

    /// `color.r/g/b` read a channel (0-255); `color.t` the transparency (0-100, 0 opaque).
    func colorComponent(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard case .color(let rgba) = try argument(call, 0, "color", &context) else { return .na }
        switch call.name {
        case "color.r": return .float(Double((rgba >> 24) & 0xFF))
        case "color.g": return .float(Double((rgba >> 16) & 0xFF))
        case "color.b": return .float(Double((rgba >> 8) & 0xFF))
        default: return .float(100 - Double(rgba & 0xFF) / 2.55)
        }
    }

    func colorGradient(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call, ["value", "bottom_value", "top_value", "bottom_color", "top_color"], &context)
        guard let value = b["value"]?.number, let bottom = b["bottom_value"]?.number,
            let top = b["top_value"]?.number, case .color(let low)? = b["bottom_color"],
            case .color(let high)? = b["top_color"], value.isFinite
        else { return .na }
        return .color(
            PineBuiltins.gradient(value, bottom: bottom, top: top, bottomColor: low, topColor: high))
    }

    func colorRGB(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        .color(
            PineBuiltins.rgb(
                try argument(call, 0, nil, &context).number ?? 0, try argument(call, 1, nil, &context).number ?? 0,
                try argument(call, 2, nil, &context).number ?? 0, try argument(call, 3, nil, &context).number ?? 0))
    }

    // MARK: - math, str

    func mathCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if call.name == "math.sum" {
            // The sliding sum of the last `length` values: a series function with a history per call site.
            let source = try argument(call, 0, "source", &context)
            let length = try argument(call, 1, "length", &context).intValue ?? 0
            return evaluateTA("math.sum", source: source, length: length, site: siteKey(call.site, context))
        }
        return PineMath.call(call.name, try allArguments(call, &context), mintick: mintick)
    }

    func stringCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if call.name == "str.format_time" {
            let b = try bind(call, ["time", "format", "timezone"], &context)
            guard let milliseconds = b["time"].intValue else { return .na }
            return .string(
                PineTime.formatted(
                    milliseconds: milliseconds, format: b["format"].textValue,
                    zone: b["timezone"].textValue))
        }
        if call.name == "str.split" {
            let values = try allArguments(call, &context)
            guard case .string(let text)? = values.first, case .string(let separator)? = values.dropFirst().first
            else { return .na }
            // An empty separator splits into characters; otherwise like Pine, empty pieces are kept.
            let pieces =
                separator.isEmpty
                ? text.map { String($0) } : text.components(separatedBy: separator)
            let id = allocate()
            working.arrays[id] = pieces.map(PineRuntimeValue.string)
            return .ref(.array, id)
        }
        return try PineStrings.call(
            call.name, try allArguments(call, &context), call.range, mintick: mintick)
    }

    func toStringCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        if case .string(let text) = value, let title = enumTitle(of: text) { return .string(title) }
        let pattern = try argument(call, 1, "format", &context).textValue
        return .string(PineFormat.format(value, pattern, mintick: mintick))
    }

    /// The title of the enum member a `"Enum.member"` string names, if any.
    private func enumTitle(of text: String) -> String? {
        guard let dot = text.firstIndex(of: "."), let members = enums[String(text[..<dot])] else { return nil }
        let member = String(text[text.index(after: dot)...])
        return members.first { $0.name == member }?.title
    }

    // MARK: - Time

    func timestampCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        var positional: [PineRuntimeValue] = []
        var named: [String: PineRuntimeValue] = [:]
        for argument in call.arguments {
            let value = try eval(argument.value, &context)
            if let label = argument.name { named[label] = value } else { positional.append(value) }
        }
        return PineTimestamp.evaluate(positional: positional, named: named).map(PineRuntimeValue.int) ?? .na
    }

    /// `year()`, `month()`…: of the bar's open time, or of the timestamp passed in.
    /// `timeframe.in_seconds(tf)`; with no argument, the chart's bar length.
    func timeframeSecondsCall(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard !call.arguments.isEmpty else { return barSeconds > 0 ? .int(Int(barSeconds)) : .na }
        switch try argument(call, 0, "timeframe", &context) {
        case .na: return .na
        case .string(let text):
            if let seconds = PineTime.seconds(ofTimeframe: text) { return .int(Int(seconds)) }
            fallthrough
        default:
            throw PineDiagnostic.error(
                "PINE4017", .runtime, "timeframe.in_seconds() needs a timeframe such as \"60\" or \"1D\".",
                call.range)
        }
    }

    /// `timeframe.change(tf)`: whether this bar opens a new `tf` period. The first bar always does.
    /// Periods follow the same UTC calendar boundaries the app folds weekly and monthly candles on.
    func timeframeChangeCall(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard case .string(let text) = try argument(call, 0, "timeframe", &context),
            let seconds = PineTime.seconds(ofTimeframe: text)
        else {
            throw PineDiagnostic.error(
                "PINE4017", .runtime, "timeframe.change() needs a timeframe such as \"60\" or \"1D\".",
                call.range)
        }
        guard let previous = lastCommittedOpenTime else { return .bool(true) }
        let now = context.bar.openTime
        return .bool(
            KlineData.bucketStart(of: now, interval: seconds)
                != KlineData.bucketStart(of: previous, interval: seconds))
    }

    func timePartCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if call.arguments.isEmpty {
            return PineTime.part(call.name, milliseconds: PineTime.milliseconds(context.bar.openTime))
        }
        guard let stamp = try argument(call, 0, nil, &context).intValue else { return .na }
        return PineTime.part(call.name, milliseconds: stamp)
    }
}
