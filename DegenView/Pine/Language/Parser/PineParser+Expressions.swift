import Foundation

extension PineParser {
    /// Ternary binds looser than every binary operator, so `a and b ? x : y` parses as
    /// `(a and b) ? x : y`.
    private static let ternaryBindingPower = 5
    private static let unaryBindingPower = 80

    /// What one round of the postfix/infix loop produced.
    private enum Extension {
        /// The expression grew; keep looking for more suffixes.
        case extended(PineExpression)
        /// Nothing more to attach, or an error ended the expression.
        case finished(PineExpression)
    }

    mutating func expression(_ minBP: Int = 0) -> PineExpression? {
        let token = current
        advance()
        // A block expression consumes its own dedent, so the suffix loop must not run: the
        // next line's first token could be mistaken for an operator.
        if token.kind == .ifKeyword || token.kind == .switchKeyword {
            guard let block = token.kind == .ifKeyword ? ifStatement() : switchStatement() else {
                return nil
            }
            return .statementExpression(block, token.range)
        }
        guard var lhs = prefixExpression(token) else { return nil }
        while true {
            switch extend(lhs, minBP) {
            case .extended(let grown): lhs = grown
            case .finished(let done): return done
            }
        }
    }

    // MARK: - Prefix

    private mutating func prefixExpression(_ token: PineToken) -> PineExpression? {
        switch token.kind {
        case .number(let n, let integer): return numberLiteral(n, integer: integer, token.range)
        case .color(let value): return .literal(.color(value), token.range)
        case .string(let s): return .literal(.string(s), token.range)
        case .bool(let b): return .literal(.bool(b), token.range)
        case .na:
            // `na(x)` is the builtin test; a bare `na` is the value.
            return at(.leftParen) ? .identifier("na", token.range) : .literal(.na, token.range)
        case .identifier(let name): return .identifier(name, token.range)
        case .typeKeyword(let type): return .identifier(type.rawValue, token.range)
        case .minus, .plus, .not:
            guard let op = PineUnaryOperator(token: token.kind),
                let operand = expression(Self.unaryBindingPower)
            else { return nil }
            return .unary(op, operand: operand, range: token.range)
        case .leftParen:
            guard let inner = expression() else { return nil }
            expect(.rightParen, "Expected ')'.")
            return inner
        case .leftBracket: return tupleLiteral(token.range)
        default:
            error("PINE2003", "Expected an expression.", token.range)
            return nil
        }
    }

    private mutating func numberLiteral(
        _ n: Double, integer: Bool, _ range: PineSourceRange
    ) -> PineExpression {
        guard integer else { return .literal(.float(n), range) }
        if let exact = Int(exactly: n) { return .literal(.int(exact), range) }
        error("PINE2014", "Integer literal is out of range.", range)
        return .literal(.float(n), range)
    }

    private mutating func tupleLiteral(_ range: PineSourceRange) -> PineExpression {
        var values: [PineExpression] = []
        if !at(.rightBracket) {
            repeat { if let e = expression() { values.append(e) } } while take(.comma)
        }
        expect(.rightBracket, "Expected ']'.")
        return .tuple(values, range)
    }

    // MARK: - Suffixes and infix

    private mutating func extend(_ lhs: PineExpression, _ minBP: Int) -> Extension {
        if take(.dot) { return memberAccess(lhs) }
        if case .identifier("array.new", _) = lhs, at(.less) { return genericArrayConstructor(lhs) }
        if case .identifier("map.new", _) = lhs, at(.less) { return genericMapConstructor(lhs) }
        if case .identifier("matrix.new", _) = lhs, at(.less) { return genericMatrixConstructor(lhs) }
        if take(.leftParen) { return callSuffix(lhs) }
        if take(.leftBracket) { return historySuffix(lhs) }
        if minBP <= Self.ternaryBindingPower, take(.question) { return ternarySuffix(lhs) }
        return binarySuffix(lhs, minBP)
    }

    private mutating func memberAccess(_ lhs: PineExpression) -> Extension {
        let member: String
        switch current.kind {
        case .identifier(let value): member = value
        case .typeKeyword(let type): member = type.rawValue
        default:
            error("PINE2004", "Expected member name.", current.range)
            return .finished(lhs)
        }
        advance()
        // A plain name stays one dotted identifier (`ta.ema`, `zone.top`); anything else is a field read.
        guard case .identifier(let base, let r) = lhs else {
            return .extended(.member(base: lhs, name: member, range: lhs.range))
        }
        return .extended(.identifier(base + "." + member, r))
    }

    /// `array.new<float>(…)` is the v6 spelling of `array.new_float(…)`; arrays are untyped at
    /// runtime, so the element type only picks the name the builtin dispatch already knows.
    private mutating func genericArrayConstructor(_ lhs: PineExpression) -> Extension {
        advance()
        guard let element = typeArgument() else { return .finished(lhs) }
        expect(.greater, "Expected '>'.")
        guard take(.leftParen), case .identifier(_, let range) = lhs else {
            error("PINE2008", "Expected '('.", current.range)
            return .finished(lhs)
        }
        return callSuffix(.identifier("array.new_" + element, range))
    }

    /// `map.new<K, V>()`: the key and value types are not tracked at runtime.
    private mutating func genericMapConstructor(_ lhs: PineExpression) -> Extension {
        advance()
        _ = typeArgument()
        expect(.comma, "Expected ',' between the key and value types.")
        _ = typeArgument()
        expect(.greater, "Expected '>'.")
        guard take(.leftParen) else {
            error("PINE2008", "Expected '('.", current.range)
            return .finished(lhs)
        }
        return callSuffix(lhs)
    }

    /// `matrix.new<float>(rows, columns, initial)`: the element type is not tracked at runtime.
    private mutating func genericMatrixConstructor(_ lhs: PineExpression) -> Extension {
        advance()
        _ = typeArgument()
        expect(.greater, "Expected '>'.")
        guard take(.leftParen) else {
            error("PINE2008", "Expected '('.", current.range)
            return .finished(lhs)
        }
        return callSuffix(lhs)
    }

    private mutating func callSuffix(_ lhs: PineExpression) -> Extension {
        let arguments = callArguments()
        expect(.rightParen, "Expected ')'.")
        if case .member(let receiver, let name, let range) = lhs {
            callSite += 1
            return .extended(
                .methodCall(
                    receiver: receiver, name: name, arguments: arguments, site: callSite, range: range))
        }
        guard case .identifier(let name, let r) = lhs else {
            error("PINE2006", "Only named functions can be called.", lhs.range)
            return .finished(lhs)
        }
        callSite += 1
        return .extended(.call(name: name, arguments: arguments, site: callSite, range: r))
    }

    /// Arguments up to, not including, the closing `)`. Stops at the first malformed one.
    private mutating func callArguments() -> [PineArgument] {
        var arguments: [PineArgument] = []
        guard !at(.rightParen) else { return arguments }
        repeat {
            let labelRange = current.range
            let name = argumentName()
            guard let value = expression() else { break }
            arguments.append(.init(name: name, value: value, nameRange: name == nil ? nil : labelRange))
        } while take(.comma)
        return arguments
    }

    /// Consumes a leading `name =` when present.
    private mutating func argumentName() -> String? {
        guard peek(1)?.kind == .assign else { return nil }
        let name: String
        switch current.kind {
        case .identifier(let n): name = n
        case .typeKeyword(let type): name = type.rawValue
        default: return nil
        }
        advance()
        advance()
        return name
    }

    private mutating func historySuffix(_ lhs: PineExpression) -> Extension {
        guard let offset = expression() else { return .finished(lhs) }
        expect(.rightBracket, "Expected ']'.")
        return .extended(.history(base: lhs, offset: offset, range: lhs.range))
    }

    private mutating func ternarySuffix(_ lhs: PineExpression) -> Extension {
        guard let yes = expression(), take(.colon), let no = expression() else {
            error("PINE2007", "Malformed ternary expression.", current.range)
            return .finished(lhs)
        }
        return .extended(.ternary(condition: lhs, whenTrue: yes, whenFalse: no, range: lhs.range))
    }

    private mutating func binarySuffix(_ lhs: PineExpression, _ minBP: Int) -> Extension {
        guard let op = PineBinaryOperator(token: current.kind), op.bindingPower.left >= minBP else {
            return .finished(lhs)
        }
        advance()
        guard let rhs = expression(op.bindingPower.right) else { return .finished(lhs) }
        return .extended(.binary(lhs: lhs, op: op, rhs: rhs, range: lhs.range))
    }
}
