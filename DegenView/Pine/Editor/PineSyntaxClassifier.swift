import Foundation

/// Splits Pine source into categorized UTF-16 ranges for the editor.
///
/// Strings, comments and annotations are found first; everything the lexer yields afterwards is
/// classified against `PineSymbolCatalog`. Builtins are matched as whole tokens (or whole dotted
/// chains), never as substrings, and names the script declares stay plain identifiers.
enum PineSyntaxClassifier {
    private static let protectedExpression = try! NSRegularExpression(
        pattern: #"(\"(?:\\.|[^\"\\\r\n])*\"?)|(//[^\r\n]*)"#)
    private static let annotationExpression = try! NSRegularExpression(
        pattern: #"^//@[A-Za-z_]+(?:=\d+)?"#)

    static func classify(_ source: String) -> [PineHighlightSpan] {
        var spans: [PineHighlightSpan] = []
        let protected = lexicalSpans(in: source)
        spans.append(contentsOf: protected)
        let blocked = protected.map(\.range)

        let normalized = PineCompiler.normalizeLineEndings(in: source)
        let offsets = normalized == source ? nil : PineOffsetMap(original: source, normalized: normalized)
        let tokens = PineLexer(source: normalized, limits: .default).lex().tokens
        var run = TokenRun(tokens: tokens, offsets: offsets, blocked: blocked)
        spans.append(contentsOf: run.classify())
        return spans
    }

    // MARK: - Strings, comments, annotations

    private static func lexicalSpans(in source: String) -> [PineHighlightSpan] {
        var spans: [PineHighlightSpan] = []
        let text = source as NSString
        let whole = NSRange(location: 0, length: text.length)
        protectedExpression.enumerateMatches(in: source, range: whole) { match, _, _ in
            guard let match else { return }
            if match.range(at: 1).location != NSNotFound {
                spans.append(.init(range: match.range, category: .string))
                return
            }
            let annotation = annotationExpression.firstMatch(
                in: source, range: match.range)?.range
            guard let annotation else {
                spans.append(.init(range: match.range, category: .comment))
                return
            }
            spans.append(.init(range: annotation, category: .annotation))
            let rest = NSRange(
                location: NSMaxRange(annotation), length: NSMaxRange(match.range) - NSMaxRange(annotation))
            if rest.length > 0 { spans.append(.init(range: rest, category: .comment)) }
        }
        return spans
    }
}

/// One pass over the token stream.
private struct TokenRun {
    let tokens: [PineToken]
    let offsets: PineOffsetMap?
    let blocked: [NSRange]
    var scopes = PineHighlightScopes()
    var parenDepth = 0
    var spans: [PineHighlightSpan] = []

    init(tokens: [PineToken], offsets: PineOffsetMap?, blocked: [NSRange]) {
        self.tokens = tokens
        self.offsets = offsets
        self.blocked = blocked
    }

    mutating func classify() -> [PineHighlightSpan] {
        var index = 0
        while index < tokens.count {
            scopes.observe(index, in: tokens)
            index = classifyToken(at: index)
        }
        return spans
    }

    // MARK: - Tokens

    /// Classifies the token at `index` (and any dotted chain it heads); returns the next index.
    private mutating func classifyToken(at index: Int) -> Int {
        let token = tokens[index]
        switch token.kind {
        case .identifier, .typeKeyword:
            return classifyName(at: index)
        case .number:
            add(token, .number)
        case .color:
            add(token, .colorLiteral)
        case .bool:
            add(token, .builtinConstant)
        case .na:
            add(token, next(index) == .leftParen ? .builtinFunction : .builtinConstant)
        case .and, .or, .not, .ifKeyword, .elseKeyword, .varKeyword, .varipKeyword, .forKeyword,
            .breakKeyword, .continueKeyword, .whileKeyword, .switchKeyword:
            add(token, .keyword)
        case .leftParen:
            parenDepth += 1
            add(token, .punctuation)
        case .rightParen:
            parenDepth = max(0, parenDepth - 1)
            add(token, .punctuation)
        case .leftBracket, .rightBracket, .comma, .dot:
            add(token, .punctuation)
        case .plus, .minus, .star, .slash, .percent, .power, .assign, .reassign, .plusAssign,
            .minusAssign, .starAssign, .slashAssign, .equal, .notEqual, .less, .lessEqual,
            .greater, .greaterEqual, .question, .colon, .arrow:
            add(token, .operator)
        case .string, .newline, .indent, .dedent, .eof:
            break
        }
        return index + 1
    }

    // MARK: - Names

    /// A name, or a dotted chain `a.b.c` headed by a name.
    private mutating func classifyName(at index: Int) -> Int {
        var chain = [index]
        // After a `.` (the head was `)` or `]`) this is a member of a value, not a chain head.
        if index > 0, tokens[index - 1].kind == .dot {
            add(tokens[index], .identifier)
            return index + 1
        }
        while let last = chain.last, tokens[last + 1].kind == .dot, nameText(last + 2) != nil {
            chain.append(last + 2)
        }
        let end = chain[chain.count - 1] + 1
        let names = chain.map { nameText($0) ?? "" }
        let isCall = tokens[end].kind == .leftParen
        let categories = categories(of: names, at: chain, isCall: isCall)
        for (position, tokenIndex) in chain.enumerated() {
            add(tokens[tokenIndex], categories[position])
            if position + 1 < chain.count { add(tokens[tokenIndex + 1], .punctuation) }
        }
        return end
    }

    private func categories(
        of names: [String], at chain: [Int], isCall: Bool
    ) -> [PineSyntaxCategory] {
        let head = names[0]
        let plain = [PineSyntaxCategory](repeating: .identifier, count: names.count)
        if scopes.role(at: chain[0]) == .declaration { return plain }
        if scopes.role(at: chain[0]) == .forKeyword { return [.keyword] }
        if scopes.isUserDefined(head) { return plain }
        if names.count == 1 { return [singleName(head, at: chain[0], isCall: isCall)] }

        guard PineSymbolCatalog.namespaces.contains(head) else { return plain }
        var result = [PineSyntaxCategory](repeating: .builtinNamespace, count: names.count)
        let full = names.joined(separator: ".")
        if PineSymbolCatalog.constants.contains(full) {
            result[names.count - 1] = .builtinConstant
        } else if isCall {
            result[names.count - 1] = .builtinFunction
        } else if PineSymbolCatalog.variables.contains(full)
            || PineSymbolCatalog.valueNamespaces.contains(head)
        {
            result[names.count - 1] = .builtinVariable
        } else {
            result[names.count - 1] = .identifier
        }
        return result
    }

    private func singleName(_ name: String, at index: Int, isCall: Bool) -> PineSyntaxCategory {
        // `title = "x"` inside a call is a named argument, whatever the name is.
        if parenDepth > 0, tokens[index + 1].kind == .assign { return .identifier }
        let next = tokens[index + 1].kind
        if case .typeKeyword = tokens[index].kind { return isCall ? .builtinFunction : .type }
        if PineSymbolCatalog.reservedWords.contains(name) { return .keyword }
        if PineSymbolCatalog.qualifiers.contains(name), !isCall, isTypeStart(next) { return .type }
        if PineSymbolCatalog.objectTypes.contains(name) { return isCall ? .builtinFunction : .type }
        if isCall { return PineSymbolCatalog.functions.contains(name) ? .builtinFunction : .identifier }
        if PineSymbolCatalog.variables.contains(name) { return .builtinVariable }
        return .identifier
    }

    private func isTypeStart(_ kind: PineTokenKind) -> Bool {
        switch kind {
        case .typeKeyword, .identifier: true
        default: false
        }
    }

    private func nameText(_ index: Int) -> String? {
        guard index < tokens.count else { return nil }
        switch tokens[index].kind {
        case .identifier(let name): return name
        case .typeKeyword(let type): return type.rawValue
        default: return nil
        }
    }

    private func next(_ index: Int) -> PineTokenKind? {
        index + 1 < tokens.count ? tokens[index + 1].kind : nil
    }

    // MARK: - Output

    private mutating func add(_ token: PineToken, _ category: PineSyntaxCategory) {
        guard let range = nsRange(of: token) else { return }
        // Text inside a string or comment keeps that category.
        if blocked.contains(where: { NSLocationInRange(range.location, $0) }) { return }
        spans.append(.init(range: range, category: category))
    }

    private func nsRange(of token: PineToken) -> NSRange? {
        let start = token.range.start
        let end = token.range.end
        if let offsets {
            guard let from = offsets.original(line: start.line, offset: start.offset),
                let to = offsets.original(line: end.line, offset: end.offset)
            else { return nil }
            return NSRange(location: from, length: max(0, to - from))
        }
        return NSRange(location: start.offset, length: max(0, end.offset - start.offset))
    }
}
