import Foundation

/// `str.*` except `str.tostring`, which is `PineFormat`.
enum PineStrings {
    private static let predicates: Set<String> = ["str.contains", "str.startswith", "str.endswith"]

    static func call(
        _ name: String, _ values: [PineRuntimeValue], _ range: PineSourceRange, mintick: Double
    ) throws -> PineRuntimeValue {
        func text(_ index: Int) -> String? {
            index < values.count ? Optional(values[index]).textValue : nil
        }
        guard let subject = text(0) else {
            // `na` (or a non-string) in, `na` out — except the predicates, which are false.
            return predicates.contains(name) ? .bool(false) : .na
        }
        switch name {
        case "str.length": return .int(subject.count)
        case "str.upper": return .string(subject.uppercased())
        case "str.lower": return .string(subject.lowercased())
        case "str.trim": return .string(subject.trimmingCharacters(in: .whitespacesAndNewlines))
        case "str.contains": return .bool(text(1).map { $0.isEmpty || subject.contains($0) } ?? false)
        case "str.startswith": return .bool(text(1).map(subject.hasPrefix) ?? false)
        case "str.endswith": return .bool(text(1).map(subject.hasSuffix) ?? false)
        case "str.replace_all":
            guard let target = text(1), !target.isEmpty, let replacement = text(2) else {
                return .string(subject)
            }
            return .string(subject.replacingOccurrences(of: target, with: replacement))
        case "str.substring": return try substring(subject, values, range)
        case "str.match": return .string(firstMatch(of: text(1), in: subject))
        case "str.repeat":
            // str.repeat(source, repeat, separator = ""): `na` for a negative count, "" for zero.
            guard let count = values.count > 1 ? values[1].intValue : nil, count >= 0 else { return .na }
            let separator = text(2) ?? ""
            return .string(Array(repeating: subject, count: count).joined(separator: separator))
        case "str.tonumber":
            return Double(subject.trimmingCharacters(in: .whitespaces)).map(PineRuntimeValue.float) ?? .na
        case "str.format":
            return .string(formatTemplate(subject, Array(values.dropFirst()), mintick: mintick))
        default:
            throw PineDiagnostic.error(
                "PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
        }
    }

    private static func substring(
        _ subject: String, _ values: [PineRuntimeValue], _ range: PineSourceRange
    ) throws -> PineRuntimeValue {
        let characters = Array(subject)
        guard values.count > 1, let begin = Optional(values[1]).intValue, begin >= 0,
            begin <= characters.count
        else {
            throw PineDiagnostic.error(
                "PINE4016", .runtime, "str.substring begin index is out of range.", range)
        }
        let end = values.count > 2 ? (Optional(values[2]).intValue ?? characters.count) : characters.count
        guard end >= begin, end <= characters.count else {
            throw PineDiagnostic.error(
                "PINE4016", .runtime, "str.substring end index is out of range.", range)
        }
        return .string(String(characters[begin..<end]))
    }

    /// `str.format("{0} of {1,number,#.##}", a, b)`: `{n}` inserts the n-th argument,
    /// `{n,number,pattern}` formats it with a `str.tostring` pattern.
    static func formatTemplate(
        _ template: String, _ values: [PineRuntimeValue], mintick: Double
    ) -> String {
        var output = ""
        var index = template.startIndex
        while index < template.endIndex {
            let character = template[index]
            guard character == "{", let close = template[index...].firstIndex(of: "}") else {
                output.append(character)
                index = template.index(after: index)
                continue
            }
            let parts = template[template.index(after: index)..<close].split(
                separator: ",", maxSplits: 2, omittingEmptySubsequences: false
            ).map { $0.trimmingCharacters(in: .whitespaces) }
            if let slot = parts.first.flatMap({ Int($0) }), values.indices.contains(slot) {
                let pattern = parts.count > 2 && parts[1] == "number" ? parts[2] : nil
                output += PineFormat.format(values[slot], pattern, mintick: mintick)
            } else {
                output += String(template[index...close])
            }
            index = template.index(after: close)
        }
        return output
    }

    /// `str.match(source, regex)`: the first substring the regular expression matches, or an empty
    /// string when nothing matches or the expression is invalid.
    private static func firstMatch(of pattern: String?, in subject: String) -> String {
        guard let pattern, let expression = try? NSRegularExpression(pattern: pattern),
            let match = expression.firstMatch(
                in: subject, range: NSRange(subject.startIndex..., in: subject)),
            let range = Range(match.range, in: subject)
        else { return "" }
        return String(subject[range])
    }
}
