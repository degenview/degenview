import Foundation

/// A question that needs one line of text from the user.
enum WatchlistPrompt: Identifiable, Equatable {
    case newList
    case renameList(current: String)
    case copyList(suggested: String)
    case newSection
    case renameSection(id: UUID, current: String)
    /// A new list that the given market goes into straight away.
    case newListFor(WatchlistInstrument)

    var id: String {
        switch self {
        case .newList: return "newList"
        case .renameList: return "renameList"
        case .copyList: return "copyList"
        case .newSection: return "newSection"
        case .renameSection(let id, _): return "renameSection-\(id)"
        case .newListFor(let item): return "newListFor-\(item.id)"
        }
    }

    var title: String {
        switch self {
        case .newList, .newListFor: return "New Watchlist"
        case .renameList: return "Rename Watchlist"
        case .copyList: return "Make a Copy"
        case .newSection: return "New Section"
        case .renameSection: return "Rename Section"
        }
    }

    var message: String {
        switch self {
        case .newList: return "Name the new watchlist."
        case .newListFor(let item): return "Name the new watchlist. \(item.name) will be added to it."
        case .renameList: return "Enter a new name."
        case .copyList: return "Name the copy."
        case .newSection: return "Sections group symbols under a heading."
        case .renameSection: return "Enter a new name."
        }
    }

    var confirmTitle: String {
        switch self {
        case .newList, .newListFor, .newSection: return "Create"
        case .renameList, .renameSection: return "Rename"
        case .copyList: return "Copy"
        }
    }

    var initialText: String {
        switch self {
        case .newList, .newSection, .newListFor: return ""
        case .renameList(let current), .renameSection(_, let current): return current
        case .copyList(let suggested): return suggested
        }
    }
}
