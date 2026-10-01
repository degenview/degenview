import Foundation

extension PineCompiler {
    /// Named-argument lookups over an `input.*` call, folded at compile time.
    private struct InputArguments {
        let arguments: [PineArgument]
        let environment: [String: PineRuntimeValue]

        func string(_ key: String) -> String? {
            guard let argument = arguments.first(where: { $0.name == key }),
                case .string(let value)? = PineCompiler.constantValue(argument.value, environment)
            else { return nil }
            return value
        }

        func number(_ key: String) -> Double? {
            arguments.first { $0.name == key }.flatMap {
                PineCompiler.constantValue($0.value, environment)?.number
            }
        }

        func boolean(_ key: String) -> Bool {
            guard let argument = arguments.first(where: { $0.name == key }),
                case .literal(.bool(let value), _) = argument.value
            else { return false }
            return value
        }

        /// `title =` or the second positional argument.
        var title: String? {
            if let named = string("title") { return named }
            guard arguments.count > 1, arguments[1].name == nil,
                case .string(let value)? = PineCompiler.constantValue(arguments[1].value, environment)
            else { return nil }
            return value
        }
    }

    /// Finds `name = input.*(…)` declarations, including those nested in `if` blocks.
    static func collectInputs(
        _ statements: [PineStatement], _ environment: [String: PineRuntimeValue],
        _ schema: inout PineInputSchema, _ diagnostics: inout [PineDiagnostic]
    ) {
        for statement in statements {
            if case .declaration(let variable, _, _, .call(let name, let args, _, let range), _) = statement,
                name.hasPrefix("input.")
            {
                guard let first = args.inputDefault,
                    let defaultValue = inputValue(first.value, function: name, environment)
                else {
                    diagnostics.append(
                        .error("PINE3010", .semantic, "\(name) requires a constant default value.", range))
                    continue
                }
                let lookup = InputArguments(arguments: args, environment: environment)
                schema.inputs.append(
                    .init(
                        id: variable, type: PineBuiltins.inputType(function: name),
                        defaultValue: defaultValue, title: lookup.title,
                        tooltip: lookup.string("tooltip"), group: lookup.string("group"),
                        inline: lookup.string("inline"), confirm: lookup.boolean("confirm"),
                        minValue: lookup.number("minval"), maxValue: lookup.number("maxval"),
                        step: lookup.number("step"), options: options(args, function: name, environment)))
            }
            if case .conditional(_, let whenTrue, let whenFalse, _) = statement {
                collectInputs(whenTrue, environment, &schema, &diagnostics)
                collectInputs(whenFalse, environment, &schema, &diagnostics)
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
}
