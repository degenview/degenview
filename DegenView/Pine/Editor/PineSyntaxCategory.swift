import Foundation

/// What a piece of Pine source is, for coloring. Colors live in `PineSyntaxTheme`.
enum PineSyntaxCategory: CaseIterable, Sendable {
    case keyword
    /// Value types, qualifiers and object types: `float`, `series`, `array`.
    case type
    case builtinNamespace
    case builtinFunction
    /// Builtins whose value changes per bar or symbol: `close`, `bar_index`, `barstate.isfirst`.
    case builtinVariable
    /// Builtins with a fixed value: `color.red`, `math.pi`, `true`, `na`.
    case builtinConstant
    /// A name the script declares, or any name Pine does not define.
    case identifier
    case number
    case string
    case comment
    /// `//@version=6` and the other `//@…` compiler directives.
    case annotation
    case `operator`
    case punctuation
    /// `#RRGGBB` literals.
    case colorLiteral
}
