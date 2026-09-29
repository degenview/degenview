import Foundation

/// `alert.freq_*`: how often one `alert()` call site may fire.
enum PineAlertFrequency: String, PineNamedConstant {
    /// Every execution that reaches the call.
    case all = "freq_all"
    /// The first execution on each bar.
    case oncePerBar = "freq_once_per_bar"
    /// Only the closing execution of a bar.
    case oncePerBarClose = "freq_once_per_bar_close"

    static let pinePrefix = "alert."
}
