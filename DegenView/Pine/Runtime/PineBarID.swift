import Foundation

/// A bar's identity: dataset plus open time. Duplicate transport messages for one bar share it, so they
/// cannot advance the series twice.
struct PineBarID: Hashable, Sendable {
    var dataset: PineDatasetKey
    var openTime: Date
}
