import Foundation

/// Why `PineRuntimeSession.execute` refused a bar. Not a script failure: the feed repeated or reordered
/// itself, and the session's committed state is untouched.
enum PineExecutionRejection: Error, Equatable, Sendable {
    /// The bar opened before one already executed.
    case staleBar
    /// The bar is the one most recently committed.
    case duplicateBar
}
