import Foundation

extension PineRuntimeSession {
    typealias CallHandler = (PineRuntimeSession) -> (PineCall, inout PineRuntimeContext) throws ->
        PineRuntimeValue

    /// Builtins dispatched by their exact name. Checked before `namespaceHandlers`.
    static let exactHandlers: [String: CallHandler] = {
        var table: [String: CallHandler] = [
            "na": PineRuntimeSession.naCall, "nz": PineRuntimeSession.nzCall,
            "chart.point.from_index": PineRuntimeSession.chartPointCall,
            "chart.point.from_time": PineRuntimeSession.chartPointCall,
            "chart.point.now": PineRuntimeSession.chartPointCall,
            "chart.point.new": PineRuntimeSession.chartPointCall,
            "max_bars_back": PineRuntimeSession.maxBarsBackCall, "time": PineRuntimeSession.timeCall,
            "input": PineRuntimeSession.inputCall, "runtime.error": PineRuntimeSession.runtimeErrorCall,
            "ticker.standard": PineRuntimeSession.tickerCall, "ticker.modify": PineRuntimeSession.tickerCall,
            "ticker.inherit": PineRuntimeSession.tickerCall, "ticker.new": PineRuntimeSession.tickerCall,
            "time_close": PineRuntimeSession.timeCloseCall,
            "color": PineRuntimeSession.colorCast, "string": PineRuntimeSession.stringCast,
            "int": PineRuntimeSession.intCast, "float": PineRuntimeSession.floatCast,
            "bool": PineRuntimeSession.boolCast, "color.new": PineRuntimeSession.colorNew,
            "color.rgb": PineRuntimeSession.colorRGB, "str.tostring": PineRuntimeSession.toStringCall,
            "timestamp": PineRuntimeSession.timestampCall, "alert": PineRuntimeSession.alertCall,
            "alertcondition": PineRuntimeSession.alertCall, "ta.atr": PineRuntimeSession.atrCall,
            "ta.tr": PineRuntimeSession.trueRangeCall, "ta.pivothigh": PineRuntimeSession.pivotCall,
            "ta.pivotlow": PineRuntimeSession.pivotCall, "ta.barssince": PineRuntimeSession.barsSinceCall,
            "ta.correlation": PineRuntimeSession.correlationCall, "ta.vwap": PineRuntimeSession.vwapCall,
            "ta.dmi": PineRuntimeSession.dmiCall, "ta.sar": PineRuntimeSession.sarCall,
            "ta.linreg": PineRuntimeSession.linregCall, "ta.cum": PineRuntimeSession.cumulativeCall, "ta.bb": PineRuntimeSession.bollingerCall,
            "timeframe.in_seconds": PineRuntimeSession.timeframeSecondsCall,
            "request.security": PineRuntimeSession.securityCall,
            "request.security_lower_tf": PineRuntimeSession.securityLowerTimeframeCall,
            "timeframe.change": PineRuntimeSession.timeframeChangeCall,
            "color.from_gradient": PineRuntimeSession.colorGradient,
            "color.r": PineRuntimeSession.colorComponent, "color.g": PineRuntimeSession.colorComponent,
            "color.b": PineRuntimeSession.colorComponent, "color.t": PineRuntimeSession.colorComponent,
        ]
        for name in ["indicator", "strategy", "library"] { table[name] = PineRuntimeSession.declarationCall }
        for name in ["line", "label", "box", "table"] { table[name] = PineRuntimeSession.handleCast }
        for name in PineTime.partNames { table[name] = PineRuntimeSession.timePartCall }
        for name in PineRuntimeSession.visualNames { table[name] = PineRuntimeSession.visualCall }
        return table
    }()

    /// Builtins dispatched by namespace, for whatever the exact table did not claim.
    static let namespaceHandlers: [(prefix: String, handler: CallHandler)] = [
        ("input.", PineRuntimeSession.inputCall), ("math.", PineRuntimeSession.mathCall),
        ("str.", PineRuntimeSession.stringCall), ("strategy.", PineRuntimeSession.strategyCall),
        ("ta.", PineRuntimeSession.taCall), ("array.", PineRuntimeSession.arrayCall),
        ("line.", PineRuntimeSession.drawingCall), ("label.", PineRuntimeSession.drawingCall),
        ("box.", PineRuntimeSession.drawingCall), ("table.", PineRuntimeSession.drawingCall),
        ("linefill.", PineRuntimeSession.drawingCall), ("map.", PineRuntimeSession.mapCall),
        ("polyline.", PineRuntimeSession.polylineCall),
        ("matrix.", PineRuntimeSession.matrixCall),
    ]

    func call(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if let overloads = methods[call.name], overloads.count > 1 {
            return try callOverloadedMethod(overloads, call, &context)
        }
        if let function = functions[call.name] { return try invoke(function, call, &context) }
        if let handler = Self.exactHandlers[call.name] { return try handler(self)(call, &context) }
        if let entry = Self.namespaceHandlers.first(where: { call.name.hasPrefix($0.prefix) }) {
            return try entry.handler(self)(call, &context)
        }
        if let object = try constructorCall(call, &context) { return object }
        if let result = try methodCall(call, &context) { return result }
        throw call.unknownFunction
    }

    /// `values.get(i)` for a variable holding an array or drawing handle is the method spelling of
    /// `array.get(values, i)`: the receiver becomes the first argument of the namespaced builtin. The
    /// receiver may be a field path (`zone.box.get_top()`). Only reached once nothing else claimed the
    /// name, so a namespace never loses to a variable.
    private func methodCall(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        guard let dot = call.name.lastIndex(of: ".") else { return nil }
        let receiverName = String(call.name[..<dot])
        let member = String(call.name[call.name.index(after: dot)...])
        let receiver: PineRuntimeValue
        if receiverName.contains(".") {
            if let value = try fieldPath(receiverName, call.range) {
                receiver = value
            } else if let dot = receiverName.firstIndex(of: "."), enums[String(receiverName[..<dot])] != nil {
                // `Mode.slow.label()`: an enum member as the receiver.
                receiver = try resolveIdentifier(receiverName, call.range, context)
            } else {
                return nil
            }
        } else {
            guard let value = working.variables[receiverName] else { return nil }
            receiver = value
        }
        let receiverArgument = PineArgument(name: nil, value: .identifier(receiverName, call.range))
        return try dispatchMethod(receiver, receiverArgument, member, call, &context)
    }

    /// `receiver.member(args)` where the receiver is any expression (`zones.get(k).kill()`): evaluated once,
    /// then passed on as a literal.
    func methodCallOnValue(
        _ receiverExpression: PineExpression, _ member: String, _ arguments: [PineArgument], _ site: Int,
        _ range: PineSourceRange, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let receiver = try eval(receiverExpression, &context)
        let receiverArgument = PineArgument(name: nil, value: .literal(receiver, range))
        let call = PineCall(name: member, arguments: arguments, site: site, range: range)
        if let result = try dispatchMethod(receiver, receiverArgument, member, call, &context) {
            return result
        }
        if receiver == .na {
            throw PineDiagnostic.error(
                "PINE4018", .runtime, "Cannot call '\(member)' on a value that is na.", range)
        }
        throw PineDiagnostic.error(
            "PINE4007", .runtime, "Unknown or unsupported method '\(member)'.", range)
    }

    /// `name(receiver, args)` for a method defined for several receiver types: the first argument picks the
    /// definition. It is evaluated once and passed on as a literal.
    private func callOverloadedMethod(
        _ overloads: [PineRuntimeFunction], _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard let first = call.arguments.first else { throw call.unknownFunction }
        let receiver = try eval(first.value, &context)
        guard
            let function = overloads.first(where: {
                $0.acceptsReceiver(receiver, instances: working.instances)
            })
        else {
            throw PineDiagnostic.error(
                "PINE4029", .runtime, "No definition of '\(call.name)' accepts that first argument.", call.range)
        }
        var arguments = call.arguments
        arguments[0] = PineArgument(name: first.name, value: .literal(receiver, call.range))
        return try invoke(
            function, PineCall(name: call.name, arguments: arguments, site: call.site, range: call.range),
            &context)
    }

    /// A user `method`, or the namespaced builtin for the receiver's kind (`array.get`, `map.put`…).
    /// Nil when the receiver is not something with methods.
    private func dispatchMethod(
        _ receiver: PineRuntimeValue, _ receiverArgument: PineArgument, _ member: String,
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue? {
        if let function = methods[member]?.first(where: {
            $0.acceptsReceiver(receiver, instances: working.instances)
        }) {
            let method = PineCall(
                name: member, arguments: [receiverArgument] + call.arguments, site: call.site,
                range: call.range)
            return try invoke(function, method, &context)
        }
        guard case .ref(let kind, _) = receiver, kind != .plot else { return nil }
        if kind == .object { return member == "copy" ? try copyInstance(receiver, call.range) : nil }
        let rewritten = PineCall(
            name: "\(kind.rawValue).\(member)", arguments: [receiverArgument] + call.arguments,
            site: call.site, range: call.range)
        return try self.call(rewritten, &context)
    }

    /// Evaluates the `index`-th positional argument, or the one named `key`, lazily: arguments
    /// a builtin never reads are never evaluated. Absent arguments are `na`.
    func argument(
        _ call: PineCall, _ index: Int, _ key: String? = nil, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        if let key, let found = call.arguments.first(where: { $0.name == key }) {
            return try eval(found.value, &context)
        }
        guard index < call.arguments.count else { return .na }
        return try eval(call.arguments[index].value, &context)
    }

    /// Every argument, in order.
    func allArguments(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> [PineRuntimeValue] {
        try call.arguments.map { try eval($0.value, &context) }
    }

    /// Binds Pine positional arguments to `names` in order; named arguments bind by name.
    func bind(
        _ call: PineCall, _ names: [String], _ context: inout PineRuntimeContext
    ) throws -> [String: PineRuntimeValue] {
        var out: [String: PineRuntimeValue] = [:]
        var position = 0
        for argument in call.arguments {
            if let name = argument.name {
                out[name] = try eval(argument.value, &context)
            } else {
                if position < names.count { out[names[position]] = try eval(argument.value, &context) }
                position += 1
            }
        }
        return out
    }

    /// Like `bind`, for the Pine overloads that take `chart.point`s instead of coordinates
    /// (`line.new(first_point, second_point, …)`): when the first positional argument is a point, or a
    /// point parameter is named, `point` names the positions; otherwise `plain` does.
    func bindOverload(
        _ call: PineCall, plain: [String], point: [String], _ context: inout PineRuntimeContext
    ) throws -> (values: [String: PineRuntimeValue], isPoint: Bool) {
        var out: [String: PineRuntimeValue] = [:]
        var names = plain
        var isPoint = false
        var position = 0
        for argument in call.arguments {
            let value = try eval(argument.value, &context)
            if let name = argument.name {
                out[name] = value
                if point.first == name { isPoint = true }
                continue
            }
            if position == 0, isChartPoint(value) {
                isPoint = true
                names = point
            }
            if position < names.count { out[names[position]] = value }
            position += 1
        }
        return (out, isPoint)
    }

    /// Offsets a call site's key inside a user function so each call site of the function
    /// keeps its own histories.
    func siteKey(_ site: Int, _ context: PineRuntimeContext) -> Int {
        context.sitePrefix == 0 ? site : context.sitePrefix &* 1_000_003 &+ site
    }

    // MARK: - User functions

    private func invoke(
        _ function: PineRuntimeFunction, _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard context.depth < limits.callDepth else {
            throw PineDiagnostic.error(
                "PINE8005", .resource, "Call depth limit exceeded in '\(call.name)'.", call.range)
        }
        let bound = try bindParameters(function, call, &context)
        let prefix = siteKey(call.site, context)
        func stashKey(_ local: String) -> String { "\(Self.internalPrefix)fn:\(prefix):\(local)" }

        let saved = function.locals.map { ($0, working.variables[$0]) }
        for local in function.locals { working.variables[local] = nil }
        for local in function.persistentLocals {
            working.variables[local] = working.variables[stashKey(local)]
        }
        for (parameter, value) in bound { working.variables[parameter] = value }

        let callerPrefix = context.sitePrefix
        context.sitePrefix = prefix
        context.depth += 1
        defer {
            context.sitePrefix = callerPrefix
            context.depth -= 1
        }
        let (_, value) = try run(function.body, &context)

        for local in function.persistentLocals {
            working.variables[stashKey(local)] = working.variables[local]
        }
        for (local, previous) in saved { working.variables[local] = previous }
        return value
    }

    /// Arguments evaluate in the caller's scope, before any local is bound: named first, then
    /// positional, then the parameter's default, else `na`.
    private func bindParameters(
        _ function: PineRuntimeFunction, _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> [String: PineRuntimeValue] {
        let positional = call.arguments.filter { $0.name == nil }
        var bound: [String: PineRuntimeValue] = [:]
        for (i, parameter) in function.parameters.enumerated() {
            if let named = call.arguments.first(where: { $0.name == parameter.name }) {
                bound[parameter.name] = try eval(named.value, &context)
            } else if i < positional.count {
                bound[parameter.name] = try eval(positional[i].value, &context)
            } else if let fallback = parameter.defaultValue {
                bound[parameter.name] = try eval(fallback, &context)
            } else {
                bound[parameter.name] = .na
            }
        }
        return bound
    }
}
