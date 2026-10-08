import Foundation

/// One row of a watchlist's flat, ordered list. A section owns the instruments that
/// follow it until the next section; instruments before the first section are at the root.
enum WatchlistEntry: Identifiable, Codable, Equatable, Sendable {
    case instrument(WatchlistInstrument)
    case section(WatchlistSection)

    var id: UUID {
        switch self {
        case .instrument(let instrument): return instrument.id
        case .section(let section): return section.id
        }
    }

    var instrument: WatchlistInstrument? {
        if case .instrument(let instrument) = self { return instrument }
        return nil
    }

    var section: WatchlistSection? {
        if case .section(let section) = self { return section }
        return nil
    }

    private enum CodingKeys: String, CodingKey { case kind, instrument, section }
    private enum Kind: String, Codable { case instrument, section }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(Kind.self, forKey: .kind) {
        case .instrument: self = .instrument(try container.decode(WatchlistInstrument.self, forKey: .instrument))
        case .section: self = .section(try container.decode(WatchlistSection.self, forKey: .section))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .instrument(let instrument):
            try container.encode(Kind.instrument, forKey: .kind)
            try container.encode(instrument, forKey: .instrument)
        case .section(let section):
            try container.encode(Kind.section, forKey: .kind)
            try container.encode(section, forKey: .section)
        }
    }
}
