import Foundation

/// What a script declares and where each name is visible, built once per text version from the
/// lexical snapshot. It is lexical: scopes follow indentation, a name is visible from the end of
/// its declaring statement, and nothing is parsed or type-checked, so it works on half-typed code.
///
/// Highlighting and completion both resolve names through it, so they cannot disagree about
/// which `close` is the builtin and which one the script declared. This mirrors the compiler,
/// where a user scope wins over a builtin.
struct PineSourceSymbolIndex {
    enum Role: Equatable, Sendable {
        /// The name being declared.
        case declaration
        /// `in`, `to` or `by` in a `for` header.
        case forKeyword
    }

    struct Parameter: Equatable, Sendable {
        let name: String
        /// The type as written (`float`, `array<float>`, `series int`), or nil.
        let typeText: String?
        /// The default as written, or nil.
        let defaultText: String?
    }

    struct Declaration: Equatable, Sendable {
        enum Kind: Equatable, Sendable {
            case variable, function, method, parameter, loopVariable, type, enumeration
        }

        let name: String
        let kind: Kind
        /// Editor range of the declared name.
        let nameRange: NSRange
        /// The scope the name lives in. A parameter or loop variable lives in its body's scope.
        var scope: Int
        /// Editor offset from which the name is visible: the end of the declaring statement.
        var visibleFrom: Int
        /// The parameters of a function or method.
        let parameters: [Parameter]?
        let isExported: Bool
        /// The user type a variable or parameter holds, when the source says so: `Point p = …`
        /// or `p = Point.new(…)`.
        let typeName: String?

        /// Whether the name shadows a builtin of the same name.
        var isValue: Bool {
            switch kind {
            case .variable, .function, .method, .parameter, .loopVariable: true
            case .type, .enumeration: false
            }
        }

        var isCallable: Bool { kind == .function || kind == .method }
    }

    struct Scope: Equatable, Sendable {
        enum Owner: Equatable, Sendable {
            case root
            case block
            case loop
            /// The body of `name(params) =>` on following lines.
            case function(String)
            /// The expression after `=>` on the same line.
            case inline
            case type(String)
            case enumeration(String)
        }

        let id: Int
        let parent: Int?
        let depth: Int
        /// Editor offsets: the first token of the scope and where it ends.
        var start: Int
        var end: Int
        /// Columns of indentation of the scope's statements.
        let indentWidth: Int
        let owner: Owner
    }

    struct Import: Equatable, Sendable {
        /// `user/Library/1`, as written.
        let path: String
        let pathRange: NSRange
        /// The alias after `as`, or the library name when there is none; nil for a malformed path.
        let alias: String?
        let aliasRange: NSRange?
    }

    struct Field: Equatable, Sendable {
        let name: String
        let typeText: String?
        /// The user type this field holds, when the type is a plain name.
        let typeName: String?
        let defaultText: String?
        let nameRange: NSRange
    }

    struct UserType: Equatable, Sendable {
        let name: String
        var fields: [Field]
        let isExported: Bool
    }

    struct UserEnum: Equatable, Sendable {
        let name: String
        var members: [String]
        let isExported: Bool
    }

    /// One statement of the source, as the splitter reads it.
    struct Statement: Equatable, Sendable {
        /// From the first token's start to the last token's end.
        let range: NSRange
        /// The last token is an operator or comma, so the next line continues the statement.
        let endsWithContinuation: Bool
        let indent: Int
    }

    let declarations: [Declaration]
    let statements: [Statement]
    let scopes: [Scope]
    let imports: [Import]
    let types: [String: UserType]
    let enums: [String: UserEnum]

    /// Declarations by scope and name, in source order.
    private let byScope: [[String: [Int]]]
    private let roles: [Int: Role]
    /// Per snapshot token: its scope and its editor offset (`-1` for structural tokens).
    private let tokenScopes: [Int]
    private let tokenOffsets: [Int]

    init(
        declarations: [Declaration], statements: [Statement], scopes: [Scope], imports: [Import],
        types: [String: UserType], enums: [String: UserEnum], roles: [Int: Role], tokenScopes: [Int],
        tokenOffsets: [Int]
    ) {
        self.declarations = declarations
        self.statements = statements
        self.scopes = scopes
        self.imports = imports
        self.types = types
        self.enums = enums
        self.roles = roles
        self.tokenScopes = tokenScopes
        self.tokenOffsets = tokenOffsets
        var grouped = [[String: [Int]]](repeating: [:], count: scopes.count)
        for (index, declaration) in declarations.enumerated()
        where declaration.scope >= 0 && declaration.scope < scopes.count {
            grouped[declaration.scope][declaration.name, default: []].append(index)
        }
        byScope = grouped
    }

    // MARK: - Tokens (highlighting)

    /// The role of the snapshot token at `tokenIndex`.
    func role(at tokenIndex: Int) -> Role? { roles[tokenIndex] }

    /// The declaration a use of `name` at the snapshot token `tokenIndex` refers to, when the
    /// script declares one that shadows a builtin.
    func resolve(_ name: String, atToken tokenIndex: Int) -> Declaration? {
        guard tokenIndex >= 0, tokenIndex < tokenScopes.count, tokenOffsets[tokenIndex] >= 0 else { return nil }
        return resolve(name, scope: tokenScopes[tokenIndex], atOffset: tokenOffsets[tokenIndex])
    }

    // MARK: - Offsets (completion)

    /// The declaration of `name` visible in `scope` at an editor offset: the innermost scope that
    /// declares it, and within it the latest declaration already visible.
    func resolve(_ name: String, scope: Int, atOffset offset: Int) -> Declaration? {
        var current: Int? = scope
        while let id = current, id < scopes.count {
            if let list = byScope[id][name] {
                for index in list.reversed() {
                    let declaration = declarations[index]
                    if declaration.isValue, declaration.visibleFrom <= offset { return declaration }
                }
            }
            current = scopes[id].parent
        }
        return nil
    }

    /// The innermost scope holding the caret. `lineIndent` is how far the caret's line is
    /// indented, so a blank line at column 0 after a function body is global while one indented
    /// four columns is still inside it.
    func scope(atOffset offset: Int, lineIndent: Int) -> Int {
        var best = 0
        var bestDepth = 0
        for scope in scopes where scope.id != 0 {
            guard scope.start <= offset, offset <= scope.end, scope.indentWidth <= lineIndent,
                scope.depth > bestDepth
            else { continue }
            best = scope.id
            bestDepth = scope.depth
        }
        return best
    }

    /// Every name visible in `scope` at `offset`, innermost first, each name once: the effective
    /// declaration after shadowing. Includes types and enumerations.
    func visibleDeclarations(scope: Int, atOffset offset: Int) -> [Declaration] {
        var seen = Set<String>()
        var result: [Declaration] = []
        var current: Int? = scope
        while let id = current, id < scopes.count {
            let names = byScope[id].keys.sorted()
            for name in names where !seen.contains(name) {
                guard
                    let index = byScope[id][name]?.last(where: { declarations[$0].visibleFrom <= offset })
                else { continue }
                seen.insert(name)
                result.append(declarations[index])
            }
            current = scopes[id].parent
        }
        return result
    }

    /// The function or method that encloses `scope`, innermost first.
    func enclosingFunction(of scope: Int) -> Declaration? {
        var current: Int? = scope
        while let id = current, id < scopes.count {
            switch scopes[id].owner {
            case .function(let name):
                return declarations.last { $0.isCallable && $0.name == name && $0.scope == scopes[id].parent }
            default:
                break
            }
            current = scopes[id].parent
        }
        return nil
    }

    /// The last statement that starts at or before `offset`.
    func statement(atOrBefore offset: Int) -> Statement? {
        var low = 0
        var high = statements.count
        while low < high {
            let middle = (low + high) / 2
            if statements[middle].range.location <= offset { low = middle + 1 } else { high = middle }
        }
        return low > 0 ? statements[low - 1] : nil
    }

    // MARK: - Imports and exports

    func importDeclaration(alias: String) -> Import? { imports.first { $0.alias == alias } }

    /// Everything the script marks `export`, for when it is a library.
    var exports: [Declaration] { declarations.filter(\.isExported) }
}
