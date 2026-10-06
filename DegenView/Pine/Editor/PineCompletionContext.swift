import Foundation

/// What a completion request knows about the caret, derived from one analysis snapshot. Pure:
/// it reads the snapshot, the symbol index and the text, never the text view.
///
/// Everything is UTF-16 (`NSRange`), the unit AppKit and the lexer share.
struct PineCompletionContext {
    /// Why no ordinary completion applies at the caret.
    enum Suppression: Equatable {
        case comment
        case string
        /// Text is selected; completion replaces nothing implicitly.
        case selection
        /// The caret is naming something being declared: `float |`, `for |`, `type |`.
        case declarationName
        /// After a dot that follows a value rather than a name: `f().|`, `a[0].|`.
        case valueMember
    }

    /// What the caret is in the middle of.
    enum Position: Equatable {
        /// The first token of a statement.
        case statementStart
        /// Inside an expression.
        case expression
        /// After `var` or `varip`, or on a field line of a type: a type is expected.
        case typeExpected
        /// After `base.`: `["ta"]` in `ta.r|`, `["chart", "point"]` in `chart.point.|`.
        case member(base: [String])
        /// In an `import` line; `typedBefore` is the path before the word being typed.
        case importPath(typedBefore: String)
    }

    let caret: Int
    /// The part of the word before the caret, which candidates must start with.
    let prefix: String
    /// The whole word under the caret (empty at the caret when there is none): what acceptance replaces.
    let replacementRange: NSRange
    let position: Position
    let suppression: Suppression?
    /// The innermost scope holding the caret, in the analysis's symbol index.
    let scope: Int
    /// Columns of indentation of the caret's line, up to the caret.
    let lineIndent: Int
    /// The call the caret is inside, if any.
    let callSite: PineCallSite?
    /// True when the user asked (Control-Space) rather than typed.
    let isExplicit: Bool

    var isMember: Bool {
        if case .member = position { return true }
        return false
    }

    // MARK: - Building

    init(analysis: PineEditorAnalysisSnapshot, selection: NSRange, explicit: Bool) {
        let editor = PineEditorContext(source: analysis.source, selection: selection, snapshot: analysis.lexical)
        let snapshot = analysis.lexical
        let index = analysis.index
        let caret = editor.caret
        self.caret = caret
        isExplicit = explicit

        // The word under the caret.
        var wordStart = caret
        var wordEnd = caret
        if caret > 0, let found = snapshot.itemIndex(containing: caret - 1),
            Self.isWord(snapshot.items[found], in: editor.source)
        {
            wordStart = snapshot.items[found].range.location
            wordEnd = NSMaxRange(snapshot.items[found].range)
        }
        prefix = editor.source.substring(with: NSRange(location: wordStart, length: caret - wordStart))
        replacementRange = NSRange(location: wordStart, length: wordEnd - wordStart)

        // Indentation of the caret's line, up to the caret, and the scope that implies.
        let line = editor.contentRange(ofLineAt: caret)
        let leading = min(editor.leadingWhitespaceLength(ofLineAt: line.location), caret - line.location)
        var indent = 0
        for offset in line.location..<(line.location + leading) {
            indent += editor.unit(at: offset) == 0x09 ? 4 : 1
        }
        lineIndent = indent
        scope = index.scope(atOffset: caret, lineIndent: indent)

        // The statement the word belongs to: none when it begins a new one.
        var statementStart: Int?
        if let statement = index.statement(atOrBefore: wordStart) {
            let endsBeforeWord = NSMaxRange(statement.range) <= wordStart
            let onLaterLine = line.location > NSMaxRange(statement.range)
            let continues = statement.endsWithContinuation || indent % 4 != 0
            if !(endsBeforeWord && onLaterLine && !continues) { statementStart = statement.range.location }
        }
        let before: [PineLexicalSnapshot.Item]
        if let statementStart {
            before = Array(snapshot.items(in: NSRange(location: statementStart, length: wordStart - statementStart)))
        } else {
            before = []
        }
        callSite = PineCallSite.locate(at: caret, statementStart: statementStart ?? Int.max, in: snapshot)

        // What the caret may not do.
        var suppression: Suppression?
        if selection.length > 0 {
            suppression = .selection
        } else if editor.isInsideComment(at: caret) {
            suppression = .comment
        } else if editor.isInsideString(at: caret) {
            suppression = .string
        }

        var position = Self.classify(
            before: before, wordStart: wordStart, snapshot: snapshot, source: editor.source,
            suppression: &suppression)
        if position == .statementStart {
            switch index.scopes[scope].owner {
            case .type: position = .typeExpected
            case .enumeration: suppression = suppression ?? .declarationName
            default: break
            }
        }
        self.position = position
        self.suppression = suppression
    }

    // MARK: - Words

    /// The name a token spells, for the kinds that can head or continue a dotted name.
    static func name(of kind: PineTokenKind) -> String? {
        switch kind {
        case .identifier(let name): name
        case .typeKeyword(let type): type.rawValue
        case .na: "na"
        default: nil
        }
    }

    /// Whether a token is a word: it starts with a letter or `_`, so keywords, `true` and `na` count.
    private static func isWord(_ item: PineLexicalSnapshot.Item, in source: NSString) -> Bool {
        guard item.range.length > 0 else { return false }
        let first = source.substring(with: NSRange(location: item.range.location, length: 1))
        guard let character = first.first else { return false }
        return character.isLetter || character == "_"
    }

    // MARK: - Position

    private static func classify(
        before: [PineLexicalSnapshot.Item], wordStart: Int, snapshot: PineLexicalSnapshot, source: NSString,
        suppression: inout Suppression?
    ) -> Position {
        func text(_ item: PineLexicalSnapshot.Item) -> String { source.substring(with: item.range) }

        guard let last = before.last else { return .statementStart }

        // `base.|` — the dot touches the word.
        if last.kind == .dot, NSMaxRange(last.range) == wordStart {
            if let base = memberBase(endingAtDotAt: last.range.location, in: snapshot) { return .member(base: base) }
            suppression = suppression ?? .valueMember
            return .expression
        }

        // `import user/Lib/1 as alias`.
        if case .identifier("import") = before[0].kind, before.count == 1 || before[1].kind != .assign {
            if before.contains(where: { $0.kind == .identifier("as") }) {
                suppression = suppression ?? .declarationName
                return .expression
            }
            let typedStart = before.count > 1 ? before[1].range.location : wordStart
            return .importPath(
                typedBefore: source.substring(with: NSRange(location: typedStart, length: wordStart - typedStart)))
        }

        var rest = before[...]
        if case .identifier("export") = rest.first?.kind, rest.count >= 1 {
            rest = rest.dropFirst()
            if rest.isEmpty { return .statementStart }
        }
        guard let head = rest.first else { return .statementStart }

        // Naming a declaration.
        switch head.kind {
        case .identifier("method"), .identifier("type"), .identifier("enum"):
            if rest.count == 1 {
                suppression = suppression ?? .declarationName
                return .expression
            }
        case .forKeyword:
            if rest.count == 1 {
                suppression = suppression ?? .declarationName
                return .expression
            }
        case .varKeyword, .varipKeyword:
            if rest.count == 1 { return .typeExpected }
            if isTypeLike(rest.dropFirst()) {
                suppression = suppression ?? .declarationName
                return .expression
            }
        default:
            break
        }
        if isTypeLike(rest), let kind = rest.last?.kind {
            switch kind {
            case .identifier, .typeKeyword, .greater:
                suppression = suppression ?? .declarationName
                return .expression
            default:
                break
            }
        }
        return .expression
    }

    /// Whether every token reads as part of a type: `float`, `Point`, `array<float>`, `series int`.
    private static func isTypeLike(_ items: ArraySlice<PineLexicalSnapshot.Item>) -> Bool {
        guard !items.isEmpty else { return false }
        return items.allSatisfy {
            switch $0.kind {
            case .identifier, .typeKeyword, .less, .greater, .comma, .dot: true
            default: false
            }
        }
    }

    /// The dotted name left of the dot at `dot`: `["ta"]`, `["chart", "point"]`. Nil when the dot
    /// follows something that is not a name, such as `)` or `]`.
    private static func memberBase(endingAtDotAt dot: Int, in snapshot: PineLexicalSnapshot) -> [String]? {
        let items = snapshot.items
        guard var current = snapshot.itemIndex(containing: dot) else { return nil }
        var base: [String] = []
        while true {
            guard current >= 1, NSMaxRange(items[current - 1].range) == items[current].range.location,
                let name = name(of: items[current - 1].kind)
            else { return nil }
            base.insert(name, at: 0)
            guard current >= 3, items[current - 2].kind == .dot,
                NSMaxRange(items[current - 2].range) == items[current - 1].range.location
            else {
                // A dot touching the head of the chain means the chain started on a value.
                if current >= 2, items[current - 2].kind == .dot,
                    NSMaxRange(items[current - 2].range) == items[current - 1].range.location
                {
                    return nil
                }
                return base
            }
            current -= 2
        }
    }
}
