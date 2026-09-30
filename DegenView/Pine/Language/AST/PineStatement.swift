import Foundation

indirect enum PineStatement: Sendable {
    case declaration(
        name: String, annotation: PineTypeAnnotation, mode: PineDeclarationMode,
        value: PineExpression, range: PineSourceRange)
    case assignment(
        name: String, op: PineAssignmentOperator, value: PineExpression, range: PineSourceRange)
    case expression(PineExpression)
    case conditional(
        condition: PineExpression, whenTrue: [PineStatement], whenFalse: [PineStatement],
        range: PineSourceRange)
    /// `for i = from to end [by step]`
    case forRange(
        variable: String, from: PineExpression, to: PineExpression, step: PineExpression?,
        body: [PineStatement], range: PineSourceRange)
    /// `for value in array` / `for [index, value] in array`
    case forIn(
        index: String?, value: String, collection: PineExpression, body: [PineStatement],
        range: PineSourceRange)
    case whileLoop(condition: PineExpression, body: [PineStatement], range: PineSourceRange)
    /// `switch subject` compares each arm's condition to the subject; `switch` without a
    /// subject takes the first arm whose condition is true.
    case switchStatement(subject: PineExpression?, arms: [PineSwitchArm], range: PineSourceRange)
    /// `[a, b] = expression`
    case tupleDeclaration(names: [String], value: PineExpression, range: PineSourceRange)
    case loopControl(PineLoopControl, PineSourceRange)
    /// User-defined function. The body's last statement is the return value.
    case function(
        name: String, parameters: [PineParameter], body: [PineStatement], range: PineSourceRange)
}
