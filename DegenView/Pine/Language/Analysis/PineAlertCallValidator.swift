import Foundation

/// Argument rules for `alert()` and `alertcondition()`. Arguments whose value the script computes
/// pass: the engine is lenient there, and a run reports what it cannot resolve.
///
/// | Code | Rule |
/// |---|---|
/// | `PINE3025` | `alert` takes a message and an optional frequency |
/// | `PINE3026` | `alertcondition` takes a condition, a title and a message |
/// | `PINE3027` | an argument name the function does not have |
/// | `PINE3028` | `freq` is not an `alert.freq_*` constant |
struct PineAlertCallValidator {
    private var diagnostics: [PineDiagnostic] = []

    static let alertParameters = ["message", "freq"]
    static let alertConditionParameters = ["condition", "title", "message"]

    static func validate(_ statements: [PineStatement]) -> [PineDiagnostic] {
        var validator = PineAlertCallValidator()
        PineStatement.forEachCall(withArgumentsIn: statements) { name, arguments, range in
            switch name {
            case "alert": validator.checkAlert(arguments, range)
            case "alertcondition": validator.checkAlertCondition(arguments, range)
            default: break
            }
        }
        return validator.diagnostics
    }

    /// Every alert call in source order.
    static func callSites(in statements: [PineStatement]) -> [PineAlertCallSite] {
        var sites: [PineAlertCallSite] = []
        PineStatement.forEachCall(withArgumentsIn: statements) { name, arguments, range in
            switch name {
            case "alert":
                let frequency = argument("freq", at: 1, of: arguments)
                sites.append(
                    PineAlertCallSite(
                        frequency: frequency.map { resolvedFrequency($0.value) } ?? .oncePerBar,
                        isCondition: false, range: range))
            case "alertcondition":
                sites.append(PineAlertCallSite(frequency: .all, isCondition: true, range: range))
            default: break
            }
        }
        return sites
    }

    /// The frequency `expression` names, when it is a constant or a literal of one.
    static func resolvedFrequency(_ expression: PineExpression) -> PineAlertFrequency? {
        switch expression {
        case .identifier(let name, _): PineAlertFrequency(pineName: name)
        case .literal(.string(let name), _): PineAlertFrequency(pineName: name)
        default: nil
        }
    }

    /// The argument bound to `parameter`: by name, or by `position` among those passed without one.
    private static func argument(
        _ parameter: String, at position: Int, of arguments: [PineArgument]
    ) -> PineArgument? {
        if let named = arguments.first(where: { $0.name == parameter }) { return named }
        let positional = arguments.filter { $0.name == nil }
        return position < positional.count ? positional[position] : nil
    }

    private mutating func checkAlert(_ arguments: [PineArgument], _ range: PineSourceRange) {
        if arguments.isEmpty || arguments.count > Self.alertParameters.count {
            report("PINE3025", "alert() takes a message and an optional frequency.", range)
        }
        checkNames(arguments, allowed: Self.alertParameters, function: "alert", range)
        guard let frequency = Self.argument("freq", at: 1, of: arguments) else { return }
        switch frequency.value {
        case .identifier(let name, let at) where name.hasPrefix(PineAlertFrequency.pinePrefix):
            if PineAlertFrequency(pineName: name) == nil { reportFrequency(at) }
        case .literal(let value, let at):
            if case .string(let name) = value, PineAlertFrequency(pineName: name) != nil { return }
            reportFrequency(at)
        default: break
        }
    }

    private mutating func checkAlertCondition(_ arguments: [PineArgument], _ range: PineSourceRange) {
        if arguments.isEmpty || arguments.count > Self.alertConditionParameters.count {
            report("PINE3026", "alertcondition() takes a condition, a title and a message.", range)
        }
        checkNames(arguments, allowed: Self.alertConditionParameters, function: "alertcondition", range)
    }

    private mutating func checkNames(
        _ arguments: [PineArgument], allowed: [String], function: String, _ range: PineSourceRange
    ) {
        for name in arguments.compactMap(\.name) where !allowed.contains(name) {
            report("PINE3027", "\(function)() has no parameter '\(name)'.", range)
        }
    }

    private mutating func reportFrequency(_ range: PineSourceRange) {
        report(
            "PINE3028",
            "Alert frequency must be alert.freq_all, alert.freq_once_per_bar or alert.freq_once_per_bar_close.",
            range)
    }

    private mutating func report(_ code: String, _ message: String, _ range: PineSourceRange) {
        diagnostics.append(.error(code, .semantic, message, range))
    }
}
