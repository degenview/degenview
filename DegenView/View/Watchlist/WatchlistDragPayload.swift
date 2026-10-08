import SwiftUI
import UniformTypeIdentifiers

/// What a watchlist row carries while it is dragged. A type of its own, so a row can never
/// be mistaken for a chart card (plain-text UUID) by the dashboard's drop targets, and the
/// drag stays inside this app.
enum WatchlistDragPayload {
    static let type = UTType(exportedAs: "com.cryptocharts.watchlist-entry")

    static func provider(for entryID: UUID) -> NSItemProvider {
        let provider = NSItemProvider()
        provider.registerDataRepresentation(for: type, visibility: .ownProcess) { completion in
            completion(Data(entryID.uuidString.utf8), nil)
            return nil
        }
        return provider
    }
}
