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
        PineMath.call(call.name, try allArguments(call, &context), mintick: mintick)
    }

    func stringCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        try PineStrings.call(
            call.name, try allArguments(call, &context), call.range, mintick: mintick)
    }

    func toStringCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let value = try argument(call, 0, nil, &context)
        let pattern = try argument(call, 1, "format", &context).textValue
        return .string(PineFormat.format(value, pattern, mintick: mintick))
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
