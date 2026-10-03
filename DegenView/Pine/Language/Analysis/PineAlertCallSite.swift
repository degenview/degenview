import Foundation

/// One `alert()` or `alertcondition()` call in a script's source.
struct PineAlertCallSite: Equatable, Sendable {
    /// How often the call may fire. Nil when the script computes it, so only a run can say.
    var frequency: PineAlertFrequency?
    /// `alertcondition()` rather than `alert()`.
    var isCondition: Bool
    var range: PineSourceRange
    /// An `alertcondition()` title, when it is a plain string literal.
    var title: String? = nil
    /// The message, when it is a plain string literal. Nil when absent or built at run time.
    var message: String? = nil
}
