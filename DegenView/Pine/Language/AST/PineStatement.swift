import Foundation

indirect enum PineStatement: Sendable {
    case declaration(String, PineTypeAnnotation, PineDeclarationMode, PineExpression, PineSourceRange)
    case assignment(String, PineTokenKind, PineExpression, PineSourceRange)
    case expression(PineExpression)
    case conditional(PineExpression, [PineStatement], [PineStatement], PineSourceRange)
    /// `for i = from to end [by step]`
    case forRange(String, PineExpression, PineExpression, PineExpression?, [PineStatement], PineSourceRange)
    /// `for value in array` / `for [index, value] in array`
    case forIn(String?, String, PineExpression, [PineStatement], PineSourceRange)
    case whileLoop(PineExpression, [PineStatement], PineSourceRange)
    /// `switch subject` compares each arm's condition to the subject; `switch` without a
    /// subject takes the first arm whose condition is true.
    case switchStatement(PineExpression?, [PineSwitchArm], PineSourceRange)
    /// `[a, b] = expression`
    case tupleDeclaration([String], PineExpression, PineSourceRange)
    case loopControl(PineLoopControl, PineSourceRange)
    /// User-defined function. The body's last statement is the return value.
    case function(String, [PineParameter], [PineStatement], PineSourceRange)
}
