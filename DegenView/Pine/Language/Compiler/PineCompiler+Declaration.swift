import Foundation

extension PineCompiler {
    private struct ScriptDeclaration {
        let type: ScriptType
        let name: String
        let arguments: [PineArgument]
        let range: PineSourceRange
    }

    private static let commonArguments: Set<String> = [
        "title", "shorttitle", "overlay", "format", "precision", "max_bars_back",
        "max_lines_count", "max_labels_count", "max_boxes_count",
    ]

    private static let strategyArguments: Set<String> = [
        "initial_capital", "default_qty_type", "default_qty_value", "commission_type",
        "commission_value", "slippage", "pyramiding", "currency", "process_orders_on_close",
        "calc_on_order_fills", "calc_on_every_tick", "close_entries_rule",
    ]

    /// Upper bound for integer settings read from a script, well inside `Int` range.
    private static let maxIntegerSetting = 1e6

    /// Builds the `indicator()`/`strategy()`/`library()` metadata, reporting a missing or
    /// duplicated declaration and unsupported arguments.
    static func declarationMetadata(
        _ statements: [PineStatement], version: String?,
        environment: [String: PineRuntimeValue], _ diagnostics: inout [PineDiagnostic]
    ) -> PineDeclarationMetadata {
        let declarations = scriptDeclarations(in: statements)
        if declarations.count != 1 {
            diagnostics.append(
                .error(
                    "PINE3001", .semantic,
                    "A script must contain exactly one indicator(), strategy(), or library() declaration.",
                    declarations.first?.range ?? .zero))
        }
        var metadata = PineDeclarationMetadata(
            type: declarations.first?.type ?? .indicator, pineVersion: version.flatMap(Int.init),
            title: "Untitled", shortTitle: nil, overlay: false, format: nil, precision: nil,
            maxBarsBack: nil)
        if metadata.type == .strategy { metadata.strategy = PineStrategySettings() }
        if let declaration = declarations.first {
            apply(declaration, environment: environment, to: &metadata, &diagnostics)
        }
        return metadata
    }

    private static func scriptDeclarations(in statements: [PineStatement]) -> [ScriptDeclaration] {
        statements.compactMap { statement in
            guard case .expression(.call(let name, let arguments, _, let range)) = statement,
                let type = ScriptType(rawValue: name)
            else { return nil }
            return ScriptDeclaration(type: type, name: name, arguments: arguments, range: range)
        }
    }

    private static func apply(
        _ declaration: ScriptDeclaration, environment: [String: PineRuntimeValue],
        to metadata: inout PineDeclarationMetadata, _ diagnostics: inout [PineDiagnostic]
    ) {
        let args = declaration.arguments
        if let first = args.first, first.name == nil || first.name == "title",
            case .string(let title)? = constantValue(first.value, environment)
        {
            metadata.title = title
        } else {
            diagnostics.append(
                .error(
                    "PINE3002", .semantic,
                    "\(declaration.name)() title must be a constant string.", declaration.range))
        }
        var supported = commonArguments
        if metadata.type == .strategy { supported.formUnion(strategyArguments) }
        for arg in args {
            guard let name = arg.name else { continue }
            guard supported.contains(name) else {
                diagnostics.append(
                    .error(
                        "PINE9001", .unsupported,
                        "Unsupported \(declaration.name)() argument '\(name)'.", arg.value.range))
                continue
            }
            guard let value = constantValue(arg.value, environment) else { continue }
            applyArgument(name, value, &metadata)
        }
    }

    private static func applyArgument(
        _ name: String, _ value: PineRuntimeValue, _ metadata: inout PineDeclarationMetadata
    ) {
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
            if let n = value.number, n >= 0, n < maxIntegerSetting { settings.slippage = Int(n) }
        case ("pyramiding", _):
            if let n = value.number, n >= 0, n < maxIntegerSetting { settings.pyramiding = Int(n) }
        case ("currency", .string(let v)): settings.currency = v
        case ("process_orders_on_close", .bool(let v)): settings.processOrdersOnClose = v
        default: break
        }
        metadata.strategy = settings
    }
}
