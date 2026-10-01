import Foundation

/// Tracks which names the script itself declares, so `close = 1` or a parameter called `close`
/// is not colored as the builtin. This mirrors the compiler, where a user scope wins over a
/// builtin (`PineTypeChecker`: `scope[name] ?? PineBuiltinTypes.identifier(name)`).
///
/// It is lexical: scopes follow the lexer's indent and dedent tokens and a name is visible from
/// the end of its declaring statement. There is no type inference.
struct PineHighlightScopes {
    enum Role {
        /// The name being declared.
        case declaration
        /// `in`, `to` or `by` in a `for` header.
        case forKeyword
    }

    private var scopes: [Set<String>] = [[]]
    /// Declared by the current statement, visible once it ends.
    private var declared: Set<String> = []
    /// A function's parameters or a loop's variables, visible in the block that follows.
    private var pending: Set<String> = []
    private var pendingIsArmed = false
    private var arrowIndex: Int?
    private var inlineParameters: Set<String> = []
    private var inInlineBody = false
    private var atStatementStart = true
    private var roles: [Int: Role] = [:]

    func role(at index: Int) -> Role? { roles[index] }

    func isUserDefined(_ name: String) -> Bool {
        scopes.contains { $0.contains(name) }
    }

    /// Feed every token in order, before classifying it.
    mutating func observe(_ index: Int, in tokens: [PineToken]) {
        let kind = tokens[index].kind
        if pendingIsArmed, kind != .indent {
            pending = []
            pendingIsArmed = false
        }
        switch kind {
        case .indent:
            scopes.append(pending)
            pending = []
            pendingIsArmed = false
            atStatementStart = true
        case .dedent:
            if scopes.count > 1 { scopes.removeLast() }
            atStatementStart = true
        case .newline:
            if inInlineBody {
                scopes.removeLast()
                inInlineBody = false
            }
            scopes[scopes.count - 1].formUnion(declared)
            declared = []
            pendingIsArmed = !pending.isEmpty
            atStatementStart = true
        case .eof:
            break
        default:
            if atStatementStart { analyzeStatement(at: index, in: tokens) }
            atStatementStart = false
            if index == arrowIndex {
                arrowIndex = nil
                if tokens[index + 1].kind != .newline {
                    scopes.append(inlineParameters)
                    inInlineBody = true
                    pending = []
                }
            }
        }
    }

    // MARK: - Statement shapes

    private mutating func analyzeStatement(at start: Int, in tokens: [PineToken]) {
        switch tokens[start].kind {
        case .forKeyword:
            analyzeFor(at: start, in: tokens)
        case .varKeyword, .varipKeyword, .identifier, .typeKeyword, .leftBracket:
            analyzeDeclaration(at: start, in: tokens)
        default:
            break
        }
    }

    /// `for i = a to b [by c]`, `for x in xs`, `for [i, x] in xs`.
    private mutating func analyzeFor(at start: Int, in tokens: [PineToken]) {
        var index = start + 1
        if tokens[index].kind == .leftBracket {
            index += 1
            while index < tokens.count, tokens[index].kind != .rightBracket,
                tokens[index].kind != .newline
            {
                if case .identifier(let name) = tokens[index].kind { declareLoopVariable(name, index) }
                index += 1
            }
        } else if case .identifier(let name) = tokens[index].kind {
            declareLoopVariable(name, index)
        }
        var depth = 0
        while index < tokens.count, tokens[index].kind != .newline, tokens[index].kind != .eof {
            switch tokens[index].kind {
            case .leftParen, .leftBracket: depth += 1
            case .rightParen, .rightBracket: depth -= 1
            case .identifier(let word) where depth == 0 && PineSymbolCatalog.forHeaderWords.contains(word):
                roles[index] = .forKeyword
            default: break
            }
            index += 1
        }
    }

    private mutating func declareLoopVariable(_ name: String, _ index: Int) {
        pending.insert(name)
        roles[index] = .declaration
    }

    /// `[var|varip] [type] name = …`, `[a, b] = …`, and `name(params) => …`.
    private mutating func analyzeDeclaration(at start: Int, in tokens: [PineToken]) {
        var depth = 0
        var firstParen: Int?
        var index = start
        while index < tokens.count {
            let kind = tokens[index].kind
            if kind == .newline || kind == .eof || kind == .indent { return }
            switch kind {
            case .leftParen:
                if depth == 0, firstParen == nil { firstParen = index }
                depth += 1
            case .leftBracket: depth += 1
            case .rightParen, .rightBracket: depth -= 1
            case .assign where depth == 0:
                declareAssignedNames(before: index, from: start, in: tokens)
                return
            case .arrow where depth == 0:
                if let firstParen { declareFunction(openParen: firstParen, arrow: index, in: tokens) }
                return
            case .reassign where depth == 0:
                return
            default: break
            }
            index += 1
        }
    }

    private mutating func declareAssignedNames(
        before assign: Int, from start: Int, in tokens: [PineToken]
    ) {
        if tokens[start].kind == .leftBracket {
            for index in start..<assign {
                if case .identifier(let name) = tokens[index].kind { declare(name, at: index) }
            }
            return
        }
        guard case .identifier(let name) = tokens[assign - 1].kind else { return }
        if assign - 2 >= start, tokens[assign - 2].kind == .dot { return }
        declare(name, at: assign - 1)
    }

    private mutating func declareFunction(openParen: Int, arrow: Int, in tokens: [PineToken]) {
        if openParen > 0, case .identifier(let name) = tokens[openParen - 1].kind {
            declare(name, at: openParen - 1)
        }
        var parameters: Set<String> = []
        var depth = 0
        for index in openParen..<arrow {
            switch tokens[index].kind {
            case .leftParen: depth += 1
            case .rightParen: depth -= 1
            case .identifier(let name) where depth == 1:
                guard index + 1 < arrow else { break }
                switch tokens[index + 1].kind {
                case .comma, .rightParen, .assign:
                    parameters.insert(name)
                    roles[index] = .declaration
                default: break
                }
            default: break
            }
        }
        pending = parameters
        inlineParameters = parameters
        arrowIndex = arrow
    }

    private mutating func declare(_ name: String, at index: Int) {
        declared.insert(name)
        roles[index] = .declaration
    }
}
