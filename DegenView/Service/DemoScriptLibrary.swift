import Foundation

/// The example Pine scripts that ship inside the app (`Resources/DemoScripts/*.pine`).
/// They are copied into the user's script library, never edited in place, so they behave
/// like any other script once they are there.
enum DemoScriptLibrary {
    struct Demo: Equatable {
        let name: String
        let source: String
    }

    /// Set once the first launch has offered the demos, so ones the user deletes stay deleted.
    static let seededKey = "scripts.demosSeeded"

    private final class BundleToken {}

    /// Every bundled demo, named after its file, in alphabetical order.
    static func bundled(in bundle: Bundle = Bundle(for: BundleToken.self)) -> [Demo] {
        let urls = bundle.urls(forResourcesWithExtension: ScriptStore.fileExtension, subdirectory: nil) ?? []
        return urls.compactMap { url in
            guard let source = try? String(contentsOf: url, encoding: .utf8) else { return nil }
            return Demo(name: url.deletingPathExtension().lastPathComponent, source: source)
        }
        .sorted { $0.name < $1.name }
    }

    /// Adds each demo the library doesn't already hold under that name, and returns how many
    /// it added. Existing scripts are never touched.
    @discardableResult
    static func addMissing(to store: ScriptStore = .shared, demos: [Demo] = bundled()) async throws -> Int {
        var added = 0
        // The sidebar lists newest first, so adding in reverse leaves the demos alphabetical.
        for demo in demos.reversed() {
            let type = PineCompiler.compile(source: demo.source).declaration.type
            do {
                try await store.create(name: demo.name, type: type, source: demo.source, disambiguating: false)
                added += 1
            } catch ScriptStoreError.nameConflict {
                continue
            }
        }
        return added
    }

    /// Offers the demos on the first launch only.
    static func seedIfNeeded(
        in store: ScriptStore = .shared, defaults: UserDefaults = .standard, demos: [Demo] = bundled()
    ) async {
        guard !defaults.bool(forKey: seededKey) else { return }
        guard (try? await addMissing(to: store, demos: demos)) != nil else { return }
        defaults.set(true, forKey: seededKey)
        NotificationCenter.default.post(name: .localScriptsDidChange, object: nil)
    }
}
