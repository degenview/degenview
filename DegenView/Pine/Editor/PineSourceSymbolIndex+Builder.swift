import Foundation

extension PineSourceSymbolIndex {
    /// Indexes one text version. `source` must be the text `snapshot` was lexed from.
    init(snapshot: PineLexicalSnapshot, source: String) {
        var builder = Builder(snapshot: snapshot, source: source)
        self = builder.build()
    }

    /// Walks the statements once, in order, keeping a stack of open scopes like the lexer keeps a
    /// stack of indents. Everything it records is in editor (UTF-16) coordinates.
    fileprivate struct Builder {
        /// What one statement knows about itself while it is analysed.
        private struct Context {
            let first: Int
            let last: Int
            let scope: Int
            /// Editor offset of the end of the statement's last token.
            let end: Int
        }

        private let tokens: [PineToken]
        private let text: NSString
        private let content: [Int]
        private let ranges: [NSRange]
        private let statements: [PineStatementSplitter.Statement]

        private var declarations: [Declaration] = []
        private var scopes: [Scope]
        private var imports: [Import] = []
        private var types: [String: UserType] = [:]
        private var enums: [String: UserEnum] = [:]
        private var roles: [Int: Role] = [:]
        private var tokenScopes: [Int]
        private var tokenOffsets: [Int]
        /// Parameters or loop variables waiting for the block that follows their header.
        private var pending: [Int] = []
        private var pendingOwner: Scope.Owner = .block

        init(snapshot: PineLexicalSnapshot, source: String) {
            let splitter = PineStatementSplitter(snapshot: snapshot, source: source)
            tokens = snapshot.tokens
            text = source as NSString
            content = splitter.content
            ranges = splitter.ranges
            statements = splitter.statements
            scopes = [Scope(id: 0, parent: nil, depth: 0, start: 0, end: text.length, indentWidth: 0, owner: .root)]
            tokenScopes = [Int](repeating: 0, count: snapshot.tokens.count)
            tokenOffsets = [Int](repeating: -1, count: snapshot.tokens.count)
        }

        /// A block header (`f(x) =>`, `for …`, `type T`) whose body has not been seen yet.
        private struct Header {
            let parameters: [Int]
            let owner: Scope.Owner
            let scope: Int
            let indent: Int
            let end: Int
        }

        mutating func build() -> PineSourceSymbolIndex {
            var open: [(indent: Int, scope: Int)] = [(0, 0)]
            var header: Header?
            for statement in statements {
                let start = ranges[statement.first].location
                if statement.indent > open[open.count - 1].indent {
                    let parent = open[open.count - 1].scope
                    let id = openScope(
                        parent: parent, start: start, indent: statement.indent,
                        owner: header?.owner ?? .block)
                    for index in header?.parameters ?? [] {
                        declarations[index].scope = id
                        declarations[index].visibleFrom = start
                    }
                    open.append((statement.indent, id))
                } else if let header {
                    // The header has no body: keep an empty scope so a caret typed there still sees
                    // the parameters.
                    openEmptyScope(for: header, endingAt: start)
                }
                header = nil
                while statement.indent < open[open.count - 1].indent, open.count > 1 {
                    scopes[open[open.count - 1].scope].end = start
                    open.removeLast()
                }
                pending = []
                pendingOwner = .block

                let scope = open[open.count - 1].scope
                for position in statement.first...statement.last {
                    tokenScopes[content[position]] = scope
                    tokenOffsets[content[position]] = ranges[position].location
                }
                let context = Context(
                    first: statement.first, last: statement.last, scope: scope,
                    end: NSMaxRange(ranges[statement.last]))
                analyze(context)
                if !pending.isEmpty || pendingOwner != .block {
                    header = Header(
                        parameters: pending, owner: pendingOwner, scope: scope, indent: statement.indent,
                        end: context.end)
                }
            }
            if let header { openEmptyScope(for: header, endingAt: text.length) }
            return PineSourceSymbolIndex(
                declarations: declarations, statements: spans(), scopes: scopes, imports: imports, types: types,
                enums: enums, roles: roles, tokenScopes: tokenScopes, tokenOffsets: tokenOffsets)
        }

        private mutating func openScope(parent: Int, start: Int, indent: Int, owner: Scope.Owner) -> Int {
            let id = scopes.count
            scopes.append(
                Scope(
                    id: id, parent: parent, depth: scopes[parent].depth + 1, start: start, end: text.length,
                    indentWidth: indent, owner: owner))
            return id
        }

        private func spans() -> [Statement] {
            statements.map { statement in
                let start = ranges[statement.first].location
                return Statement(
                    range: NSRange(location: start, length: NSMaxRange(ranges[statement.last]) - start),
                    endsWithContinuation: PineLexer.continuationKinds.contains(kind(statement.last)),
                    indent: statement.indent)
            }
        }

        private mutating func openEmptyScope(for header: Header, endingAt end: Int) {
            let id = scopes.count
            scopes.append(
                Scope(
                    id: id, parent: header.scope, depth: scopes[header.scope].depth + 1, start: header.end,
                    end: end, indentWidth: header.indent + 1, owner: header.owner))
            for index in header.parameters {
                declarations[index].scope = id
                declarations[index].visibleFrom = header.end
            }
        }

        // MARK: - Token access

        private func kind(_ position: Int) -> PineTokenKind { tokens[content[position]].kind }

        /// The text of an identifier token (not a keyword or a type keyword).
        private func word(_ position: Int) -> String? {
            guard position >= 0, position < content.count, case .identifier(let name) = kind(position)
            else { return nil }
            return name
        }

        /// The source text from the start of one token to the end of another.
        private func slice(_ from: Int, _ through: Int) -> String {
            let start = ranges[from].location
            return text.substring(with: NSRange(location: start, length: NSMaxRange(ranges[through]) - start))
        }

        private mutating func declare(
            _ name: String, _ kind: Declaration.Kind, at position: Int, scope: Int, visibleFrom: Int,
            parameters: [Parameter]? = nil, isExported: Bool = false, typeName: String? = nil
        ) -> Int {
            declarations.append(
                Declaration(
                    name: name, kind: kind, nameRange: ranges[position], scope: scope, visibleFrom: visibleFrom,
                    parameters: parameters, isExported: isExported, typeName: typeName))
            roles[content[position]] = .declaration
            return declarations.count - 1
        }

        // MARK: - Statements

        private mutating func analyze(_ context: Context) {
            switch kind(context.first) {
            case .forKeyword:
                analyzeFor(context)
            case .varKeyword, .varipKeyword, .identifier, .typeKeyword, .leftBracket:
                analyzeNamed(context)
            default:
                break
            }
        }

        private mutating func analyzeNamed(_ context: Context) {
            switch scopes[context.scope].owner {
            case .type(let owner):
                analyzeField(of: owner, context)
                return
            case .enumeration(let owner):
                analyzeMember(of: owner, context)
                return
            default:
                break
            }
            var start = context.first
            var isExported = false
            if word(start) == "export", start < context.last, kind(start + 1) != .assign,
                kind(start + 1) != .reassign
            {
                isExported = true
                start += 1
            }
            if let first = word(start) {
                if first == "import", start < context.last, kind(start + 1) != .assign {
                    analyzeImport(start, context)
                    return
                }
                if first == "type" || first == "enum", context.last - start == 1, word(start + 1) != nil {
                    analyzeTypeHeader(isEnum: first == "enum", start, context, isExported: isExported)
                    return
                }
            }
            analyzeDeclaration(from: start, context, isExported: isExported)
        }

        /// `for i = a to b [by c]`, `for x in xs`, `for [i, x] in xs`.
        private mutating func analyzeFor(_ context: Context) {
            var position = context.first + 1
            guard position <= context.last else { return }
            if kind(position) == .leftBracket {
                position += 1
                while position <= context.last, kind(position) != .rightBracket {
                    if let name = word(position) { declareLoopVariable(name, position) }
                    position += 1
                }
            } else if let name = word(position) {
                declareLoopVariable(name, position)
            }
            var depth = 0
            while position <= context.last {
                switch kind(position) {
                case .leftParen, .leftBracket: depth += 1
                case .rightParen, .rightBracket: depth -= 1
                case .identifier(let word) where depth == 0 && PineSymbolCatalog.forHeaderWords.contains(word):
                    roles[content[position]] = .forKeyword
                default: break
                }
                position += 1
            }
            pendingOwner = .loop
        }

        private mutating func declareLoopVariable(_ name: String, _ position: Int) {
            pending.append(declare(name, .loopVariable, at: position, scope: -1, visibleFrom: 0))
        }

        /// `[var|varip] [type] name = …`, `[a, b] = …`, and `name(params) => …`.
        private mutating func analyzeDeclaration(from start: Int, _ context: Context, isExported: Bool) {
            var depth = 0
            var firstParen: Int?
            for position in start...context.last {
                switch kind(position) {
                case .leftParen:
                    if depth == 0, firstParen == nil { firstParen = position }
                    depth += 1
                case .leftBracket:
                    depth += 1
                case .rightParen, .rightBracket:
                    depth -= 1
                case .assign where depth == 0:
                    declareAssigned(before: position, from: start, context, isExported: isExported)
                    return
                case .arrow where depth == 0:
                    if let firstParen {
                        declareFunction(
                            openParen: firstParen, arrow: position, from: start, context, isExported: isExported)
                    }
                    return
                case .reassign where depth == 0:
                    return
                default:
                    break
                }
            }
        }

        private mutating func declareAssigned(
            before assign: Int, from start: Int, _ context: Context, isExported: Bool
        ) {
            if kind(start) == .leftBracket {
                for position in start..<assign {
                    if let name = word(position) {
                        _ = declare(
                            name, .variable, at: position, scope: context.scope, visibleFrom: context.end)
                    }
                }
                return
            }
            guard assign - 1 >= start, let name = word(assign - 1) else { return }
            if assign - 2 >= start, kind(assign - 2) == .dot { return }
            var typeName: String?
            if assign - 2 >= start, let written = word(assign - 2),
                !PineSymbolCatalog.qualifiers.contains(written)
            {
                typeName = written
            } else if assign + 3 <= context.last, let constructed = word(assign + 1), kind(assign + 2) == .dot,
                word(assign + 3) == "new"
            {
                typeName = constructed
            }
            _ = declare(
                name, .variable, at: assign - 1, scope: context.scope, visibleFrom: context.end,
                isExported: isExported, typeName: typeName)
        }

        private mutating func declareFunction(
            openParen: Int, arrow: Int, from start: Int, _ context: Context, isExported: Bool
        ) {
            guard openParen - 1 >= start, let name = word(openParen - 1),
                let close = closingParen(of: openParen, before: arrow), close + 1 == arrow
            else { return }
            let isMethod = word(start) == "method"
            let parsed = parameters(between: openParen, and: close)
            _ = declare(
                name, isMethod ? .method : .function, at: openParen - 1, scope: context.scope,
                visibleFrom: context.end, parameters: parsed.map(\.parameter), isExported: isExported)
            var indices: [Int] = []
            for entry in parsed {
                indices.append(
                    declare(
                        entry.parameter.name, .parameter, at: entry.position, scope: -1, visibleFrom: 0,
                        typeName: entry.typeName))
            }
            if arrow == context.last {
                pending = indices
                pendingOwner = .function(name)
            } else {
                // `f(x) => x + 1`: the parameters are visible to the rest of this statement only.
                let bodyStart = NSMaxRange(ranges[arrow])
                let id = scopes.count
                scopes.append(
                    Scope(
                        id: id, parent: context.scope, depth: scopes[context.scope].depth + 1, start: bodyStart,
                        end: context.end, indentWidth: scopes[context.scope].indentWidth, owner: .inline))
                for index in indices {
                    declarations[index].scope = id
                    declarations[index].visibleFrom = bodyStart
                }
                for position in (arrow + 1)...context.last {
                    tokenScopes[content[position]] = id
                }
            }
        }

        private func closingParen(of open: Int, before limit: Int) -> Int? {
            var depth = 0
            for position in open..<limit {
                switch kind(position) {
                case .leftParen: depth += 1
                case .rightParen:
                    depth -= 1
                    if depth == 0 { return position }
                default: break
                }
            }
            return nil
        }

        /// The parameters of `name(a, float b = 1, Point p)`: `open` and `close` are the parentheses.
        private func parameters(
            between open: Int, and close: Int
        ) -> [(parameter: Parameter, position: Int, typeName: String?)] {
            var result: [(parameter: Parameter, position: Int, typeName: String?)] = []
            var depth = 0
            var segmentStart = open + 1
            var position = open + 1
            while position <= close {
                switch kind(position) {
                case .leftParen, .leftBracket: depth += 1
                case .rightParen, .rightBracket: depth -= 1
                default: break
                }
                let isEnd = position == close || (depth == 0 && kind(position) == .comma)
                if isEnd {
                    if segmentStart <= position - 1,
                        let entry = parameter(from: segmentStart, through: position - 1)
                    {
                        result.append(entry)
                    }
                    segmentStart = position + 1
                }
                position += 1
            }
            return result
        }

        private func parameter(
            from first: Int, through last: Int
        ) -> (parameter: Parameter, position: Int, typeName: String?)? {
            var depth = 0
            var assign: Int?
            for position in first...last {
                switch kind(position) {
                case .leftParen, .leftBracket: depth += 1
                case .rightParen, .rightBracket: depth -= 1
                case .assign where depth == 0:
                    if assign == nil { assign = position }
                default: break
                }
            }
            let nameEnd = (assign ?? last + 1) - 1
            guard nameEnd >= first, let name = word(nameEnd) else { return nil }
            let typeText = nameEnd > first ? slice(first, nameEnd - 1) : nil
            var defaultText: String?
            if let assign, assign < last { defaultText = slice(assign + 1, last) }
            var typeName: String?
            if nameEnd - 1 == first, let written = word(first), !PineSymbolCatalog.qualifiers.contains(written) {
                typeName = written
            }
            return (Parameter(name: name, typeText: typeText, defaultText: defaultText), nameEnd, typeName)
        }

        // MARK: - Imports, types, enums

        /// `import user/Library/1 [as alias]`. A half-typed line still records what it has.
        private mutating func analyzeImport(_ start: Int, _ context: Context) {
            var asPosition: Int?
            for position in (start + 1)...context.last where word(position) == "as" {
                asPosition = position
                break
            }
            let pathLast = (asPosition ?? context.last + 1) - 1
            guard pathLast >= start + 1 else { return }
            let path = slice(start + 1, pathLast)
            let pathRange = NSRange(
                location: ranges[start + 1].location,
                length: NSMaxRange(ranges[pathLast]) - ranges[start + 1].location)
            var alias = PineLibraryLinker.defaultAlias(forPath: path)
            var aliasRange: NSRange?
            if let asPosition, asPosition + 1 <= context.last, let written = word(asPosition + 1) {
                alias = written
                aliasRange = ranges[asPosition + 1]
            }
            imports.append(Import(path: path, pathRange: pathRange, alias: alias, aliasRange: aliasRange))
        }

        /// `type Name` or `enum Name`; its indented lines are the fields or members.
        private mutating func analyzeTypeHeader(
            isEnum: Bool, _ start: Int, _ context: Context, isExported: Bool
        ) {
            guard let name = word(start + 1) else { return }
            _ = declare(
                name, isEnum ? .enumeration : .type, at: start + 1, scope: context.scope,
                visibleFrom: context.end, isExported: isExported)
            if isEnum {
                enums[name] = UserEnum(name: name, members: [], isExported: isExported)
                pendingOwner = .enumeration(name)
            } else {
                types[name] = UserType(name: name, fields: [], isExported: isExported)
                pendingOwner = .type(name)
            }
        }

        /// `float x = 1`, `Point origin`, `array<float> values`: a field of the type being declared.
        private mutating func analyzeField(of owner: String, _ context: Context) {
            var depth = 0
            var assign: Int?
            for position in context.first...context.last {
                switch kind(position) {
                case .leftParen, .leftBracket: depth += 1
                case .rightParen, .rightBracket: depth -= 1
                case .assign where depth == 0:
                    if assign == nil { assign = position }
                default: break
                }
            }
            let nameEnd = (assign ?? context.last + 1) - 1
            guard nameEnd >= context.first, let name = word(nameEnd) else { return }
            var typeStart = context.first
            if kind(typeStart) == .varKeyword || kind(typeStart) == .varipKeyword { typeStart += 1 }
            let typeText = nameEnd > typeStart ? slice(typeStart, nameEnd - 1) : nil
            var typeName: String?
            if nameEnd - 1 == typeStart, let written = word(typeStart),
                !PineSymbolCatalog.qualifiers.contains(written)
            {
                typeName = written
            }
            var defaultText: String?
            if let assign, assign < context.last { defaultText = slice(assign + 1, context.last) }
            roles[content[nameEnd]] = .declaration
            types[owner]?.fields.append(
                Field(
                    name: name, typeText: typeText, typeName: typeName, defaultText: defaultText,
                    nameRange: ranges[nameEnd]))
        }

        /// `fast` or `slow = "Slow"`: a member of the enum being declared.
        private mutating func analyzeMember(of owner: String, _ context: Context) {
            guard let name = word(context.first) else { return }
            enums[owner]?.members.append(name)
            if context.first < context.last, kind(context.first + 1) == .assign {
                roles[content[context.first]] = .declaration
            }
        }
    }
}
