import Foundation

/// What a watchlist row shows for one market. Transient: it is never stored with a
/// watchlist, and a missing value stays nil so the row can show a dash instead of zero.
struct WatchlistQuote: Equatable, Sendable {
    /// What the change is measured against. Providers disagree, so the row says which.
    enum ChangeBasis: String, Sendable {
        case rolling24h
        case previousClose
        case window

        var label: String {
            switch self {
            case .rolling24h: return "Change over the last 24 hours"
            case .previousClose: return "Change since the previous close"
            case .window: return "Change over the market's comparison window"
            }
        }
    }

    enum Freshness: String, Sendable {
        case live
        case delayed
        case stale
        case marketClosed
        case unavailable
        case unsupported

        var label: String {
            switch self {
            case .live: return "Live"
            case .delayed: return "Delayed"
            case .stale: return "Stale"
            case .marketClosed: return "Market closed"
            case .unavailable: return "No quote"
            case .unsupported: return "No quotes for this market"
            }
        }

        /// Whether the price can be shown as current.
        var isCurrent: Bool { self == .live || self == .delayed }
    }

    enum VolumeKind: String, Sendable {
        case quoteCurrency
        case shares
        case base
    }

    var last: Double?
    var change: Double?
    var changePercent: Double?
    var changeBasis: ChangeBasis?
    var volume: Double?
    var volumeKind: VolumeKind?
    var timestamp: Date?
    var freshness: Freshness
    /// Why a quote is missing, when the provider said ("Add your Alpaca API key in Settings").
    var note: String?

    init(
        last: Double? = nil, change: Double? = nil, changePercent: Double? = nil,
        changeBasis: ChangeBasis? = nil, volume: Double? = nil, volumeKind: VolumeKind? = nil,
        timestamp: Date? = nil, freshness: Freshness = .unavailable, note: String? = nil
    ) {
        self.last = last
        self.change = change
        self.changePercent = changePercent
        self.changeBasis = changeBasis
        self.volume = volume
        self.volumeKind = volumeKind
        self.timestamp = timestamp
        self.freshness = freshness
        self.note = note
    }

    static func unsupported(_ note: String? = nil) -> WatchlistQuote {
        WatchlistQuote(freshness: .unsupported, note: note)
    }

    /// The same quote re-labelled, for the last known value of a source that stopped answering.
    func with(freshness: Freshness) -> WatchlistQuote {
        var copy = self
        copy.freshness = freshness
        return copy
    }
}
