import Foundation

/// What a catalog name is, for tooling that lists members by namespace.
enum PineCatalogSymbolKind: Equatable, Sendable {
    case function
    case variable
    case constant
    case namespace
}

/// One direct child of a namespace: `rsi` under `ta`, `point` under `chart`.
struct PineCatalogMember: Equatable, Sendable {
    /// The last segment, `rsi`.
    let label: String
    /// The full path, `ta.rsi`.
    let qualifiedName: String
    let kind: PineCatalogSymbolKind
}

extension PineSymbolCatalog {
    /// Builtins that are both a namespace and a callable keep the callable. Every other overlap
    /// (`color`, `line`, `label`, `box`, `table`) lists as the namespace, since `color(` and
    /// `line(` are rare casts and `color.red` is not.
    private static let callableBeatsNamespace: Set<String> = ["plot", "hline"]

    /// The direct members of a namespace path: `members(of: "ta")`, `members(of: "chart.point")`.
    /// The empty path lists the root: unqualified builtins plus every namespace that has members.
    /// Sorted by label. Names Pine defines but DegenView does not implement are left out.
    static func members(of path: String) -> [PineCatalogMember] { memberIndex[path] ?? [] }

    /// What `name` is, or nil for a name the catalog does not list or does not implement.
    static func kind(of name: String) -> PineCatalogSymbolKind? { leafKinds[name] }

    /// Whether an editor should offer `name`: listed, and implemented.
    static func isCompletable(_ name: String) -> Bool { leafKinds[name] != nil }

    /// Whether `name` is a Pine function this release does not implement.
    static func isUnimplementedFunction(_ name: String) -> Bool { reservedFunctions.contains(name) }

    /// Words that start a statement and are not names: `if`, `for`, `var`, `import`…
    static let statementKeywords: [String] = [
        "if", "for", "while", "switch", "var", "varip", "import", "export", "method", "type", "enum",
        "break", "continue",
    ]

    /// Words that read as values or operators inside an expression.
    static let expressionKeywords: [String] = ["true", "false", "and", "or", "not"]

    /// Names usable where a type is expected: `int`, `float`, `line`, `array`…
    static let typeNames: Set<String> = {
        var names: Set<String> = ["int", "float", "bool", "string", "color"]
        names.formUnion(objectTypes)
        return names
    }()

    // MARK: - Index

    private static let leafKinds: [String: PineCatalogSymbolKind] = {
        var kinds: [String: PineCatalogSymbolKind] = [:]
        func record(_ name: String, _ kind: PineCatalogSymbolKind) {
            guard !name.isEmpty, kinds[name].map({ rank($0) < rank(kind) }) ?? true else { return }
            kinds[name] = kind
        }
        for name in variables where !isUnimplementedVariable(name) { record(name, .variable) }
        for name in constants { record(name, .constant) }
        var functionNames = Set(PineRuntimeSession.exactHandlers.keys)
        functionNames.formUnion(PineRuntimeSession.visualNames)
        functionNames.formUnion(PineTA.supported)
        functionNames.formUnion(PineSymbolMetadata.table.keys)
        for name in functionNames where !reservedFunctions.contains(name) { record(name, .function) }
        return kinds
    }()

    /// Higher wins when one name is listed twice: `time` is a variable and a function.
    private static func rank(_ kind: PineCatalogSymbolKind) -> Int {
        switch kind {
        case .variable: 3
        case .constant: 2
        case .function: 1
        case .namespace: 0
        }
    }

    private static let memberIndex: [String: [PineCatalogMember]] = {
        var children: [String: [String: PineCatalogMember]] = [:]
        for (name, kind) in leafKinds {
            let parts = name.split(separator: ".").map(String.init)
            for depth in 0..<parts.count {
                let parent = parts[..<depth].joined(separator: ".")
                let path = parts[...depth].joined(separator: ".")
                let isLeaf = depth == parts.count - 1
                let member = PineCatalogMember(
                    label: parts[depth], qualifiedName: path, kind: isLeaf ? kind : .namespace)
                if let existing = children[parent, default: [:]][parts[depth]] {
                    children[parent, default: [:]][parts[depth]] = merged(existing, member)
                } else {
                    children[parent, default: [:]][parts[depth]] = member
                }
            }
        }
        return children.mapValues { $0.values.sorted { $0.label < $1.label } }
    }()

    /// The same label seen as a leaf and as a namespace segment, or as two leaves.
    private static func merged(_ a: PineCatalogMember, _ b: PineCatalogMember) -> PineCatalogMember {
        let namespace = a.kind == .namespace ? a : (b.kind == .namespace ? b : nil)
        guard let namespace else { return rank(a.kind) >= rank(b.kind) ? a : b }
        let other = namespace == a ? b : a
        if other.kind == .namespace { return namespace }
        if other.kind == .function, callableBeatsNamespace.contains(other.qualifiedName) { return other }
        // A variable or constant that is also a namespace prefix keeps its value kind.
        return other.kind == .function ? namespace : other
    }
}
