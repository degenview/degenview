import Foundation

/// What a set of committed bars belongs to. Series from one key are never reused for another.
struct PineDatasetKey: Hashable, Sendable {
    /// `"<source>:<ticker>"`, matching `syminfo.tickerid`.
    var symbolKey: String
    /// The chart timeframe's label, e.g. `"1h"`.
    var timeframe: String
}
