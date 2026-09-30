import Foundation

enum PineCompiler {
    static func compile(source: String, limits: PineLimits = .default) -> PineCompiledProgram {
        let normalizedSource = normalizeLineEndings(in: source)
        var diagnostics: [PineDiagnostic] = []
        let versionMatches = normalizedSource.split(separator: "\n").compactMap { line -> String? in
            let s = line.trimmingCharacters(in: .whitespacesAndNewlines)
            return s.hasPrefix("//@version=") ? String(s.dropFirst(11)) : nil
        }
        if versionMatches.isEmpty {
            diagnostics.append(PineDiagnostic.error("PINE0001", .semantic, "Missing //@version=6 annotation.", .zero))
        } else if versionMatches.first != "6" {
            diagnostics.append(
                PineDiagnostic.error(
                    "PINE0002", .semantic,
                    "Pine Script version \(versionMatches.first!) is not supported; this runtime targets v6.",
                    .zero))
        }
        let lexed = PineLexer(source: normalizedSource, limits: limits).lex()
        diagnostics += lexed.diagnostics
        var parser = PineParser(tokens: lexed.tokens, limits: limits)
        let (statements, parseDiagnostics) = parser.parse()
        diagnostics += parseDiagnostics
        var declarations: [(ScriptType, PineExpression, PineSourceRange)] = []
        for statement in statements {
            if case .expression(let e) = statement, case .call(let name, _, _, let r) = e,
                name == "indicator"
            {
                declarations.append((.indicator, e, r))
            } else if case .expression(let e) = statement,
                case .call(let name, _, _, let r) = e,
                let type = ScriptType(rawValue: name)
            {
                declarations.append((type, e, r))
            }
        }
        if declarations.count != 1 {
            diagnostics.append(
                PineDiagnostic.error(
                    "PINE3001", .semantic,
                    "A script must contain exactly one indicator(), strategy(), or library() declaration.",
                    declarations.first?.2 ?? .zero))
        }
        let environment = constantEnvironment(statements)
        var metadata = PineDeclarationMetadata(
            type: declarations.first?.0 ?? .indicator,
            pineVersion: versionMatches.first.flatMap(Int.init),
            title: "Untitled", shortTitle: nil, overlay: false, format: nil, precision: nil,
            maxBarsBack: nil)
        if metadata.type == .strategy { metadata.strategy = PineStrategySettings() }
        if let expression = declarations.first?.1, case .call(let declarationName, let args, _, let range) = expression
        {
            if let first = args.first, first.name == nil || first.name == "title",
                case .string(let title)? = constantValue(first.value, environment)
            {
                metadata.title = title
            } else {
                diagnostics.append(
                    PineDiagnostic.error("PINE3002", .semantic, "\(declarationName)() title must be a constant string.", range))
            }
            var supported: Set<String> = [
                "title", "shorttitle", "overlay", "format", "precision", "max_bars_back",
                "max_lines_count", "max_labels_count", "max_boxes_count",
            ]
            if metadata.type == .strategy {
                supported.formUnion([
                    "initial_capital", "default_qty_type", "default_qty_value", "commission_type",
                    "commission_value", "slippage", "pyramiding", "currency", "process_orders_on_close",
                    "calc_on_order_fills", "calc_on_every_tick", "close_entries_rule",
                ])
            }
            for arg in args where arg.name != nil {
                let name = arg.name!
                if !supported.contains(name) {
                    diagnostics.append(
                        PineDiagnostic.error(
                            "PINE9001", .unsupported, "Unsupported \(declarationName)() argument '\(name)'.",
                            arg.value.range))
                    continue
                }
                guard let value = constantValue(arg.value, environment) else { continue }
                switch (name, value) {
                case ("shorttitle", .string(let v)): metadata.shortTitle = v
                case ("overlay", .bool(let v)): metadata.overlay = v
                case ("format", .string(let v)): metadata.format = v
                case ("precision", .int(let v)): metadata.precision = v
                case ("max_bars_back", .int(let v)): metadata.maxBarsBack = v
                case ("max_lines_count", .int(let v)): metadata.maxLinesCount = v
                case ("max_labels_count", .int(let v)): metadata.maxLabelsCount = v
                case ("max_boxes_count", .int(let v)): metadata.maxBoxesCount = v
                default: applyStrategySetting(name, value, &metadata)
                }
            }
        }
        var schema = PineInputSchema()
        collectInputs(statements, environment, &schema, &diagnostics)
        validate(statements, diagnostics: &diagnostics)
        // Type errors on top of a broken parse would only be noise from a half-built tree.
        if !diagnostics.contains(where: { $0.category == .lexical || $0.category == .syntax }) {
            diagnostics += PineTypeChecker.check(statements)
        }
        return .init(
            source: normalizedSource, statements: statements, declaration: metadata, inputSchema: schema,
            diagnostics: diagnostics)
    }

    /// Text copied from browsers and editors can contain CR-only or Unicode line separators.
    /// Normalize them before both annotation discovery and lexing so a leading `//` comment
    /// cannot accidentally consume the entire script.
    private static func normalizeLineEndings(in source: String) -> String {
        source
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .replacingOccurrences(of: "\u{2028}", with: "\n")
            .replacingOccurrences(of: "\u{2029}", with: "\n")
    }

    /// Values of top-level declarations that fold to a constant and are never reassigned, so
    /// `group = g_sr` and `default_qty_value = riskPercent` resolve at compile time.
    private static func constantEnvironment(_ statements: [PineStatement]) -> [String: PineRuntimeValue] {
        var reassigned = Set<String>()
        func collect(_ statements: [PineStatement]) {
            for statement in statements {
                switch statement {
                case .assignment(let name, _, _, _): reassigned.insert(name)
                case .conditional(_, let a, let b, _):
                    collect(a)
                    collect(b)
                case .forRange(_, _, _, _, let body, _), .forIn(_, _, _, let body, _),
                    .whileLoop(_, let body, _), .function(_, _, let body, _):
                    collect(body)
                case .switchStatement(_, let arms, _): arms.forEach { collect($0.body) }
                default: break
                }
            }
        }
        collect(statements)
        var environment: [String: PineRuntimeValue] = [:]
        for statement in statements {
            guard case .declaration(let name, _, .ordinary, let expression, _) = statement,
                !reassigned.contains(name), let value = constantValue(expression, environment)
            else { continue }
            environment[name] = value
        }
        return environment
    }

    /// Folds compile-time expressions: literals, builtin constants, earlier constants,
    /// arithmetic on numbers, string concatenation, colors, and `timestamp()`.
    static func constantValue(_ e: PineExpression, _ environment: [String: PineRuntimeValue])
        -> PineRuntimeValue?
    {
        switch e {
        case .literal(let value, _): return value == .na ? nil : value
        case .identifier(let name, _):
            if let value = environment[name] { return value }
            if let value = PineBuiltins.constants[name] { return value }
            return PineBuiltins.colors[name].map(PineRuntimeValue.color)
        case .unary(.negate, let inner, _):
            switch constantValue(inner, environment) {
            case .int(let x)?: return .int(0 &- x)
            case .float(let x)?: return .float(-x)
            default: return nil
            }
        case .binary(let l, let op, let r, _):
            guard op.isArithmetic, let a = constantValue(l, environment),
                let b = constantValue(r, environment)
            else { return nil }
            let value = PineOperators.apply(op, a, b)
            return value == .na ? nil : value
        case .call("timestamp", let arguments, _, _):
            var positional: [PineRuntimeValue] = []
            var named: [String: PineRuntimeValue] = [:]
            for argument in arguments {
                guard let value = constantValue(argument.value, environment) else { return nil }
                if let name = argument.name { named[name] = value } else { positional.append(value) }
            }
            return PineTimestamp.evaluate(positional: positional, named: named).map(PineRuntimeValue.int)
        case .call("color.new", _, _, _), .call("color.rgb", _, _, _):
            return constantColor(e).map(PineRuntimeValue.color)
        default: return nil
        }
    }

    private static func applyStrategySetting(
        _ name: String, _ value: PineRuntimeValue, _ metadata: inout PineDeclarationMetadata
    ) {
        guard var settings = metadata.strategy else { return }
        switch (name, value) {
        case ("initial_capital", _):
            if let n = value.number, n > 0 { settings.initialCapital = n }
        case ("default_qty_type", .string(let v)):
            switch v {
            case "strategy.cash": settings.quantityType = .cash
            case "strategy.percent_of_equity": settings.quantityType = .percentOfEquity
            default: settings.quantityType = .fixed
            }
        case ("default_qty_value", _):
            if let n = value.number, n >= 0 { settings.quantityValue = n }
        case ("commission_type", .string(let v)):
            switch v {
            case "strategy.commission.cash_per_order": settings.commissionType = .cashPerOrder
            case "strategy.commission.cash_per_contract": settings.commissionType = .cashPerContract
            default: settings.commissionType = .percent
            }
        case ("commission_value", _):
            if let n = value.number, n >= 0 { settings.commissionValue = n }
        case ("slippage", _):
            if let n = value.number, n >= 0, n < 1e6 { settings.slippage = Int(n) }
        case ("pyramiding", _):
            if let n = value.number, n >= 0, n < 1e6 { settings.pyramiding = Int(n) }
        case ("currency", .string(let v)): settings.currency = v
        case ("process_orders_on_close", .bool(let v)): settings.processOrdersOnClose = v
        default: break
        }
        metadata.strategy = settings
    }

    private static func collectInputs(
        _ statements: [PineStatement], _ environment: [String: PineRuntimeValue],
        _ schema: inout PineInputSchema, _ diagnostics: inout [PineDiagnostic]
    ) {
        for statement in statements {
            if case .declaration(let variable, _, _, let expr, _) = statement,
                case .call(let name, let args, _, let range) = expr, name.hasPrefix("input.")
            {
                let type: PineValueType
                switch name {
                case "input.int": type = .int
                case "input.float": type = .float
                case "input.bool": type = .bool
                case "input.color": type = .color
                case "input.time": type = .time
                default: type = .string
                }
                guard let first = args.first,
                    let defaultValue = inputValue(first.value, function: name, environment)
                else {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3010", .semantic, "\(name) requires a constant default value.", range))
                    continue
                }
                func string(_ key: String) -> String? {
                    args.first { $0.name == key }.flatMap {
                        if case .string(let v)? = constantValue($0.value, environment) { return v }
                        return nil
                    }
                }
                func number(_ key: String) -> Double? {
                    args.first { $0.name == key }.flatMap { constantValue($0.value, environment)?.number }
                }
                func boolean(_ key: String) -> Bool {
                    args.first { $0.name == key }.flatMap {
                        if case .literal(.bool(let v), _) = $0.value { return v }
                        return nil
                    } ?? false
                }
                let title =
                    string("title")
                    ?? (args.count > 1 && args[1].name == nil
                        ? {
                            if case .string(let v)? = constantValue(args[1].value, environment) { return v }
                            return nil
                        }() : nil)
                schema.inputs.append(
                    .init(
                        id: variable, type: type, defaultValue: defaultValue, title: title,
                        tooltip: string("tooltip"), group: string("group"), inline: string("inline"),
                        confirm: boolean("confirm"), minValue: number("minval"), maxValue: number("maxval"),
                        step: number("step"), options: options(args, function: name, environment)))
            }
            if case .conditional(_, let a, let b, _) = statement {
                collectInputs(a, environment, &schema, &diagnostics)
                collectInputs(b, environment, &schema, &diagnostics)
            }
        }
    }
    private static func options(
        _ args: [PineArgument], function: String, _ environment: [String: PineRuntimeValue]
    ) -> [PineInputValue]? {
        guard let argument = args.first(where: { $0.name == "options" }),
            case .tuple(let values, _) = argument.value
        else { return nil }
        let options = values.compactMap { inputValue($0, function: function, environment) }
        return options.count == values.count && !options.isEmpty ? options : nil
    }

    private static func inputValue(
        _ e: PineExpression, function: String, _ environment: [String: PineRuntimeValue]
    ) -> PineInputValue? {
        if function == "input.source", case .identifier(let name, _) = e {
            return .source(name)
        }
        switch constantValue(e, environment) {
        case .int(let x)?: return .int(x)
        case .float(let x)?: return .float(x)
        case .bool(let x)?: return .bool(x)
        case .string(let x)?: return .string(x)
        case .color(let x)?: return .color(x)
        default: return nil
        }
    }

    /// Folds compile-time color expressions: literals, `color.*` constants, and
    /// `color.new`/`color.rgb` with constant arguments.
    static func constantColor(_ e: PineExpression) -> UInt32? {
        switch e {
        case .literal(.color(let rgba), _): return rgba
        case .identifier(let name, _): return PineBuiltins.colors[name]
        case .call("color.new", let arguments, _, _):
            guard arguments.count >= 2, let base = constantColor(arguments[0].value),
                let transparency = constantNumber(arguments[1].value)
            else { return nil }
            return PineBuiltins.withTransparency(base, transparency)
        case .call("color.rgb", let arguments, _, _):
            let numbers = arguments.map { constantNumber($0.value) }
            guard numbers.count >= 3, !numbers.contains(where: { $0 == nil }) else { return nil }
            return PineBuiltins.rgb(numbers[0]!, numbers[1]!, numbers[2]!, numbers.count > 3 ? numbers[3]! : 0)
        default: return nil
        }
    }

    private static func constantNumber(_ e: PineExpression) -> Double? {
        switch e {
        case .literal(let value, _): return value.number
        case .unary(.negate, let inner, _): return constantNumber(inner).map { -$0 }
        default: return nil
        }
    }
    private static func validate(_ statements: [PineStatement], diagnostics: inout [PineDiagnostic]) {
        validate(statements, inheritedDeclarations: [], inLoop: false, diagnostics: &diagnostics)
    }

    private static func validate(
        _ statements: [PineStatement], inheritedDeclarations: Set<String>, inLoop: Bool,
        diagnostics: inout [PineDiagnostic]
    ) {
        var declared = inheritedDeclarations
        for statement in statements {
            switch statement {
            case .declaration(let n, let annotation, _, let e, let r):
                if declared.contains(n) {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3020", .semantic, "Variable '\(n)' is already declared in this scope.", r))
                }
                declared.insert(n)
                if annotation.type == .bool, case .literal(.na, _) = e {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3021", .semantic, "Boolean values cannot be na in Pine v6.", r))
                }
            case .assignment(let n, _, _, let r):
                if !declared.contains(n) {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3022", .semantic, "Cannot reassign undeclared variable '\(n)'.", r))
                }
            case .conditional(_, let a, let b, _):
                validate(a, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
                validate(b, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
            case .forRange(let variable, _, _, _, let body, _):
                validate(
                    body, inheritedDeclarations: declared.union([variable]), inLoop: true,
                    diagnostics: &diagnostics)
            case .forIn(let index, let value, _, let body, _):
                validate(
                    body, inheritedDeclarations: declared.union([index, value].compactMap { $0 }),
                    inLoop: true, diagnostics: &diagnostics)
            case .whileLoop(_, let body, _):
                validate(body, inheritedDeclarations: declared, inLoop: true, diagnostics: &diagnostics)
            case .switchStatement(_, let arms, _):
                for arm in arms {
                    validate(
                        arm.body, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
                }
            case .tupleDeclaration(let names, _, let r):
                for name in names where name != "_" {
                    if declared.contains(name) {
                        diagnostics.append(
                            PineDiagnostic.error("PINE3020", .semantic, "Variable '\(name)' is already declared in this scope.", r))
                    }
                    declared.insert(name)
                }
            case .loopControl(_, let r):
                if !inLoop {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3023", .semantic, "break and continue are only allowed inside a loop.", r))
                }
            case .function(let n, let parameters, let body, let r):
                if declared.contains(n) {
                    diagnostics.append(
                        PineDiagnostic.error("PINE3024", .semantic, "Function '\(n)' is already declared.", r))
                }
                declared.insert(n)
                // Function bodies are their own scope: locals may reuse global names, and
                // globals cannot be reassigned from inside a function.
                validate(
                    body, inheritedDeclarations: Set(parameters.map(\.name)), inLoop: false,
                    diagnostics: &diagnostics)
            case .expression(let e):
                if case .call(let n, _, _, let r) = e,
                    n.hasPrefix("request.")
                {
                    diagnostics.append(
                        PineDiagnostic.error("PINE9003", .unsupported, "Feature '\(n)' is not supported in this release.", r))
                }
            }
        }
    }
}
