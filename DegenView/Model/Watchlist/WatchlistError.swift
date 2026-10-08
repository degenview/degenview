import Foundation

enum WatchlistError: LocalizedError, Equatable {
    case duplicate(String)
    case notFound
    case lastFavorites
    case emptyName
    case persistenceUnavailable
    case writeFailed(String)

    var errorDescription: String? {
        switch self {
        case .duplicate(let name): return "\"\(name)\" is already in this watchlist"
        case .notFound: return "That watchlist item no longer exists"
        case .lastFavorites: return "The Favorites watchlist can be renamed but not deleted"
        case .emptyName: return "A name can't be empty"
        case .persistenceUnavailable:
            return "Your saved watchlists couldn't be read, so changes are turned off to protect them"
        case .writeFailed(let reason): return "Couldn't save watchlists: \(reason)"
        }
    }
}
