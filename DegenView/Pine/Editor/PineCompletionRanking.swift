import Foundation

/// Deterministic ordering of candidates. No fuzzy matching and no history: the same text and
/// caret always give the same list.
///
/// The sort key, compared in order:
/// 1. a label equal to the prefix (ignoring case) comes first;
/// 2. then a label that starts with the prefix in the same case;
/// 3. then the tier: the nearest scope's parameters and locals, outer locals, globals the script
///    declares, library members, parameter names, builtins, namespaces, types, keywords;
/// 4. then the kind: parameters and variables before functions before constants;
/// 5. then the label, case-insensitively, and finally case-sensitively.
///
/// After `ta.` every candidate gets the same tier, so only the match, the kind and the
/// alphabet decide, and nothing from the script can land between `ta.rma` and `ta.roc`.
enum PineCompletionRanking {
    enum Tier: Int, Equatable, Sendable, Comparable {
        case innermostScope = 0
        case outerScope
        case scriptGlobal
        case library
        case argumentName
        case builtin
        case namespace
        case type
        case keyword
        /// Every candidate after a dot.
        case member

        static func < (lhs: Tier, rhs: Tier) -> Bool { lhs.rawValue < rhs.rawValue }
    }

    static func sorted(_ items: [PineCompletionItem], prefix: String) -> [PineCompletionItem] {
        let lowered = prefix.lowercased()
        func key(_ item: PineCompletionItem) -> (Int, Int, Int, Int, String, String) {
            let label = item.label.lowercased()
            return (
                label == lowered ? 0 : 1,
                item.label.hasPrefix(prefix) ? 0 : 1,
                item.tier.rawValue,
                kindRank(item.kind),
                label,
                item.label
            )
        }
        return items.sorted { key($0) < key($1) }
    }

    static func kindRank(_ kind: PineCompletionKind) -> Int {
        switch kind {
        case .parameter: 0
        case .local: 1
        case .variable, .field: 2
        case .function, .libraryMember: 3
        case .constant, .enumMember: 4
        case .library: 5
        case .argumentName: 6
        case .namespace: 7
        case .type: 8
        case .keyword: 9
        }
    }
}
