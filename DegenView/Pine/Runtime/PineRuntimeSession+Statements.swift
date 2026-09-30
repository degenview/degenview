import Foundation

extension PineRuntimeSession {
    /// The effect of one statement: how control left it and, unless it left the previous
    /// value standing, the value it produced.
    struct Step {
        var flow = PineFlow.normal
        var value: PineRuntimeValue?
    }

    /// Runs a block. Returns how the block ended and the value of its last statement,
    /// which is a function's return value.
    @discardableResult
    func run(
        _ statements: [PineStatement], _ context: inout PineRuntimeContext
    ) throws -> (flow: PineFlow, value: PineRuntimeValue) {
        var last = PineRuntimeValue.void
        for statement in statements {
            try budget()
            let step = try runStatement(statement, &context)
            if let value = step.value { last = value }
            if step.flow != .normal { return (step.flow, last) }
        }
        return (.normal, last)
    }

    private func runStatement(
        _ statement: PineStatement, _ context: inout PineRuntimeContext
    ) throws -> Step {
        switch statement {
        case .declaration(let name, _, let mode, let value, _):
            return try runDeclaration(name, mode, value, &context)
        case .assignment(let name, let op, let value, _):
            return try runAssignment(name, op, value, &context)
        case .expression(let expression): return Step(value: try eval(expression, &context))
        case .conditional(let condition, let whenTrue, let whenFalse, _):
            return try runConditional(condition, whenTrue, whenFalse, &context)
        case .forRange(let variable, let from, let to, let step, let body, let range):
            return try runForRange(variable, from, to, step, body, range, &context)
        case .forIn(let index, let value, let collection, let body, let range):
            return try runForIn(index, value, collection, body, range, &context)
        case .whileLoop(let condition, let body, _): return try runWhile(condition, body, &context)
        case .switchStatement(let subject, let arms, let range):
            return try runSwitch(subject, arms, range, &context)
        case .tupleDeclaration(let names, let value, _):
            return try runTupleDeclaration(names, value, &context)
        case .loopControl(let control, _):
            return Step(flow: control == .breakLoop ? .breakLoop : .continueLoop)
        case .function: return Step()
        }
    }

    // MARK: - Variables

    private func runDeclaration(
        _ name: String, _ mode: PineDeclarationMode, _ expression: PineExpression,
        _ context: inout PineRuntimeContext
    ) throws -> Step {
        if mode == .variable, let existing = working.variables[name] { return Step(value: existing) }
        if mode == .intrabar, let value = intrabar[name] ?? working.variables[name] {
            working.variables[name] = value
            intrabar[name] = value
            context.record(mode, for: name)
            return Step(value: value)
        }
        let value = try eval(expression, &context)
        working.variables[name] = value
        context.record(mode, for: name)
        if mode == .intrabar { intrabar[name] = value }
        return Step(value: value)
    }

    private func runAssignment(
        _ name: String, _ op: PineAssignmentOperator, _ expression: PineExpression,
        _ context: inout PineRuntimeContext
    ) throws -> Step {
        let rhs = try eval(expression, &context)
        let old = working.variables[name] ?? .na
        let value = PineOperators.apply(op, old: old, rhs)
        working.variables[name] = value
        if context.intrabarNames.contains(name) || intrabar[name] != nil { intrabar[name] = value }
        return Step(value: value)
    }

    private func runTupleDeclaration(
        _ names: [String], _ expression: PineExpression, _ context: inout PineRuntimeContext
    ) throws -> Step {
        let value = try eval(expression, &context)
        let items: [PineRuntimeValue]
        if case .tuple(let tuple) = value { items = tuple } else { items = [] }
        for (i, name) in names.enumerated() where name != "_" {
            working.variables[name] = i < items.count ? items[i] : .na
        }
        return Step(value: value)
    }

    // MARK: - Control flow

    private func requireBool(
        _ value: PineRuntimeValue, _ message: String, _ range: PineSourceRange
    ) throws -> Bool {
        guard case .bool(let test) = value else {
            throw PineDiagnostic.error("PINE4001", .runtime, message, range)
        }
        return test
    }

    private func runConditional(
        _ condition: PineExpression, _ whenTrue: [PineStatement], _ whenFalse: [PineStatement],
        _ context: inout PineRuntimeContext
    ) throws -> Step {
        let test = try requireBool(
            try eval(condition, &context),
            "if condition must be bool; numeric-to-bool coercion is not allowed in v6.", condition.range)
        let (flow, value) = try run(test ? whenTrue : whenFalse, &context)
        return Step(flow: flow, value: value)
    }

    private func runForRange(
        _ variable: String, _ from: PineExpression, _ to: PineExpression, _ stepExpression: PineExpression?,
        _ body: [PineStatement], _ range: PineSourceRange, _ context: inout PineRuntimeContext
    ) throws -> Step {
        let startValue = try eval(from, &context)
        guard let start = startValue.number, let stop = try eval(to, &context).number else {
            return Step()
        }
        var stride = 1.0
        var integral = startValue.isInt
        if let stepExpression {
            let stepValue = try eval(stepExpression, &context)
            guard let size = stepValue.number, size != 0, size.isFinite else {
                throw PineDiagnostic.error(
                    "PINE4014", .runtime, "for loop step must be a non-zero number.", range)
            }
            stride = abs(size)
            if !stepValue.isInt { integral = false }
        }
        // Pine picks the direction from the bounds: `for i = 0 to -1` runs twice.
        let direction = stop >= start ? 1.0 : -1.0
        var last: PineRuntimeValue?
        var i = start
        while direction > 0 ? i <= stop : i >= stop {
            try budget()
            working.variables[variable] = integral ? PineRuntimeValue.integral(i) : .float(i)
            let (flow, value) = try run(body, &context)
            last = value
            if flow == .breakLoop { break }
            i += direction * stride
        }
        return Step(value: last)
    }

    private func runForIn(
        _ indexName: String?, _ valueName: String, _ collection: PineExpression,
        _ body: [PineStatement], _ range: PineSourceRange, _ context: inout PineRuntimeContext
    ) throws -> Step {
        let target = try eval(collection, &context)
        if target == .na { return Step() }
        guard case .ref(.array, let id) = target else {
            throw PineDiagnostic.error("PINE4011", .runtime, "for...in requires an array.", range)
        }
        var last: PineRuntimeValue?
        // Iterate a snapshot: Pine forbids resizing the array inside the loop.
        for (offset, item) in (working.arrays[id] ?? []).enumerated() {
            try budget()
            if let indexName { working.variables[indexName] = .int(offset) }
            working.variables[valueName] = item
            let (flow, value) = try run(body, &context)
            last = value
            if flow == .breakLoop { break }
        }
        return Step(value: last)
    }

    private func runWhile(
        _ condition: PineExpression, _ body: [PineStatement], _ context: inout PineRuntimeContext
    ) throws -> Step {
        var last: PineRuntimeValue?
        while true {
            try budget()
            let test = try requireBool(
                try eval(condition, &context), "while condition must be bool.", condition.range)
            if !test { break }
            let (flow, value) = try run(body, &context)
            last = value
            if flow == .breakLoop { break }
        }
        return Step(value: last)
    }

    private func runSwitch(
        _ subject: PineExpression?, _ arms: [PineSwitchArm], _ range: PineSourceRange,
        _ context: inout PineRuntimeContext
    ) throws -> Step {
        let target = try subject.map { try eval($0, &context) }
        guard let chosen = try chooseArm(arms, target: target, range, &context) else {
            return Step(value: .na)
        }
        let (flow, value) = try run(chosen, &context)
        return Step(flow: flow, value: value)
    }

    /// The first arm whose condition matches `target` (or is true, for a subject-less switch);
    /// otherwise the default arm.
    private func chooseArm(
        _ arms: [PineSwitchArm], target: PineRuntimeValue?, _ range: PineSourceRange,
        _ context: inout PineRuntimeContext
    ) throws -> [PineStatement]? {
        var fallback: [PineStatement]?
        for arm in arms {
            guard let condition = arm.condition else {
                fallback = fallback ?? arm.body
                continue
            }
            let value = try eval(condition, &context)
            let matches: Bool
            if let target {
                matches = PineOperators.apply(.equal, target, value) == .bool(true)
            } else {
                matches = try requireBool(value, "switch arm condition must be bool.", range)
            }
            if matches { return arm.body }
        }
        return fallback
    }
}
