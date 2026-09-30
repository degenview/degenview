import Foundation

/// An `alert()` / `alertcondition()` that fired.
struct PineAlertEvent: Sendable, Identifiable, Equatable {
    let id: Int
    /// Call-site key, so `alert.freq_once_per_bar` can fire once per bar per call.
    var site: Int
    var bar: Int
    var time: Date
    var message: String
}
