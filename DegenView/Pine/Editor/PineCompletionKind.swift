import Foundation

/// What a completion is. Insertion and ranking read this, never the label's spelling.
enum PineCompletionKind: Equatable, Sendable {
    case function
    case variable
    case constant
    /// A parameter of the enclosing function.
    case parameter
    /// A variable declared in a block, or a loop variable.
    case local
    case namespace
    case keyword
    case type
    /// A field of a user type.
    case field
    case enumMember
    /// An import alias: `lib` in `lib.myEMA`.
    case library
    /// An exported function, type or constant of an imported library.
    case libraryMember
    /// A parameter name inside a call, inserted as `name = `.
    case argumentName
}

/// Where a completion comes from.
enum PineCompletionOrigin: Equatable, Sendable {
    case builtin
    case user
    case library(alias: String)
}

/// What accepting a completion inserts around its name.
enum PineCompletionInsertionStyle: Equatable, Sendable {
    /// The name alone: variables, constants, fields, keywords, types.
    case identifier
    /// The name and a call: `ta.rsi(` or `ta.rsi()`.
    case callable
    /// The name and a dot, which opens member completion.
    case namespace
    /// The name and ` = `.
    case argumentName
}
