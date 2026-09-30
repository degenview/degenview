import Foundation

extension PineRuntimeSession {
    typealias CallHandler = (PineRuntimeSession) -> (PineCall, inout PineRuntimeContext) throws ->
        PineRuntimeValue

    /// Builtins dispatched by their exact name. Checked before `namespaceHandlers`.
    private static let exactHandlers: [String: CallHandler] = {
        var table: [String: CallHandler] = [
            "na": PineRuntimeSession.naCall, "nz": PineRuntimeSession.nzCall,
            "int": PineRuntimeSession.intCast, "float": PineRuntimeSession.floatCast,
            "bool": PineRuntimeSession.boolCast, "color.new": PineRuntimeSession.colorNew,
            "color.rgb": PineRuntimeSession.colorRGB, "str.tostring": PineRuntimeSession.toStringCall,
            "timestamp": PineRuntimeSession.timestampCall, "alert": PineRuntimeSession.alertCall,
            "alertcondition": PineRuntimeSession.alertCall, "ta.atr": PineRuntimeSession.atrCall,
            "ta.tr": PineRuntimeSession.trueRangeCall, "ta.pivothigh": PineRuntimeSession.pivotCall,
            "ta.pivotlow": PineRuntimeSession.pivotCall, "ta.barssince": PineRuntimeSession.barsSinceCall,
            "ta.cum": PineRuntimeSession.cumulativeCall, "ta.bb": PineRuntimeSession.bollingerCall,
        ]
        for name in ["indicator", "strategy", "library"] { table[name] = PineRuntimeSession.declarationCall }
        for name in ["line", "label", "box", "table"] { table[name] = PineRuntimeSession.handleCast }
        for name in PineTime.partNames { table[name] = PineRuntimeSession.timePartCall }
        for name in PineRuntimeSession.visualNames { table[name] = PineRuntimeSession.visualCall }
        return table
    }()

    /// Builtins dispatched by namespace, for whatever the exact table did not claim.
    private static let namespaceHandlers: [(prefix: String, handler: CallHandler)] = [
        ("input.", PineRuntimeSession.inputCall), ("math.", PineRuntimeSession.mathCall),
        ("str.", PineRuntimeSession.stringCall), ("strategy.", PineRuntimeSession.strategyCall),
        ("ta.", PineRuntimeSession.taCall), ("array.", PineRuntimeSession.arrayCall),
        ("line.", PineRuntimeSession.drawingCall), ("label.", PineRuntimeSession.drawingCall),
        ("box.", PineRuntimeSession.drawingCall), ("table.", PineRuntimeSession.drawingCall),
    ]

    func call(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if let function = functions[call.name] { return try invoke(function, call, &context) }
        if let handler = Self.exactHandlers[call.name] { return try handler(self)(call, &context) }
        if let entry = Self.namespaceHandlers.first(where: { call.name.hasPrefix($0.prefix) }) {
            return try entry.handler(self)(call, &context)
        }
        throw call.unknownFunction
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
