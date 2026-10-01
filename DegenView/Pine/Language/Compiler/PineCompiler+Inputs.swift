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
        _ schema: inout PineInputSchema, _ diagnostics: inout [PineDiagnostic],
        enums: [String: [PineEnumMember]]? = nil
    ) {
        let enums = enums ?? enumDeclarations(in: statements)
        for statement in statements {
            if case .declaration(let variable, _, _, .call(let name, let args, _, let range), _) = statement,
                name.hasPrefix("input.")
            {
                if name == "input.enum" {
                    if let input = enumInput(variable, args, enums, environment) {
                        schema.inputs.append(input)
                    } else {
                        diagnostics.append(
                            .error(
                                "PINE3010", .semantic,
                                "input.enum requires a member of a declared enum as its default value.", range))
                    }
                    continue
                }
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
                collectInputs(whenTrue, environment, &schema, &diagnostics, enums: enums)
                collectInputs(whenFalse, environment, &schema, &diagnostics, enums: enums)
            }
        }
    }

    private static func enumDeclarations(in statements: [PineStatement]) -> [String: [PineEnumMember]] {
        var enums: [String: [PineEnumMember]] = [:]
        for case .enumDeclaration(let name, let members, _) in statements { enums[name] = members }
        return enums
    }

    /// `input.enum(Name.member, title, options = [Name.a, Name.b], …)`: a string input whose options are
    /// the enum's members (or the ones listed), shown by title and stored as `"Name.member"`.
    private static func enumInput(
        _ variable: String, _ args: [PineArgument], _ enums: [String: [PineEnumMember]],
        _ environment: [String: PineRuntimeValue]
    ) -> PineInputDefinition? {
        func member(_ expression: PineExpression) -> (name: String, member: PineEnumMember)? {
            guard case .identifier(let full, _) = expression, let dot = full.firstIndex(of: "."),
                let found = enums[String(full[..<dot])]?.first(where: {
                    $0.name == String(full[full.index(after: dot)...])
                })
            else { return nil }
            return (full, found)
        }
        guard let first = args.inputDefault, let initial = member(first.value) else { return nil }
        var choices: [(name: String, member: PineEnumMember)] = []
        let listed = args.first { $0.name == "options" }
        if let listed, case .tuple(let values, _) = listed.value {
            choices = values.compactMap(member)
        } else if let dot = initial.name.firstIndex(of: "."), let all = enums[String(initial.name[..<dot])] {
            let prefix = String(initial.name[..<dot])
            choices = all.map { ("\(prefix).\($0.name)", $0) }
        }
        let lookup = InputArguments(arguments: args, environment: environment)
        return PineInputDefinition(
            id: variable, type: .string, defaultValue: .string(initial.name), title: lookup.title,
            tooltip: lookup.string("tooltip"), group: lookup.string("group"), inline: lookup.string("inline"),
            confirm: lookup.boolean("confirm"), minValue: nil, maxValue: nil, step: nil,
            options: choices.map { .string($0.name) }, optionTitles: choices.map(\.member.title))
    }

    private static func options(
        _ args: [PineArgument], function: String, _ environment: [String: PineRuntimeValue]
    ) -> [PineInputValue]? {
        // `options =`, or the third positional argument: input.string(defval, title, options, …).
        let positional = args.filter { $0.name == nil }
        let named = args.first { $0.name == "options" }
        guard let argument = named ?? (positional.count > 2 ? positional[2] : nil),
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
        // A named constant such as `size.small` is the string of its own name at runtime.
        if case .identifier(let name, _) = e, constantValue(e, environment) == nil,
            PineSymbolCatalog.constants.contains(name)
        {
            return .string(name)
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
