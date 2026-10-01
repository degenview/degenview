import Foundation

enum ScriptStoreError: LocalizedError, Equatable {
    case missingScript, missingRevision, nameConflict, unreadableFile
    var errorDescription: String? {
        switch self {
        case .missingScript: return "The script no longer exists."
        case .missingRevision: return "The script revision no longer exists."
        case .nameConflict: return "A script with that name already exists."
        case .unreadableFile: return "The file isn't a readable text file."
        }
    }
}

/// The only persistence authority for user-authored scripts. It deliberately has no
/// networking dependency: every operation is confined to the supplied local directories.
///
/// Each script is a plain `<name>.pine` file in `scriptsDirectory`, so it can be opened,
/// edited, or backed up with any tool. Everything a `.pine` file can't carry — the stable
/// id charts reference, favorite flag, revisions, draft, compile cache — lives beside it in
/// `metadataDirectory/<id>/`. A file finds its id through an extended attribute (which
/// survives Finder renames and moves) and falls back to `index.json` (name → id) for
/// editors whose atomic saves drop extended attributes.
actor ScriptStore {
    static let shared = ScriptStore(
        scriptsDirectory: AppSupport.directory.appendingPathComponent("Scripts", isDirectory: true),
        metadataDirectory: AppSupport.directory.appendingPathComponent("ScriptMetadata", isDirectory: true))
    static let compilerVersion = "pine-local-3"
    static let fileExtension = "pine"
    static let idAttribute = "com.cryptocharts.script-id"

    /// Everything about a script except its name and source, which the `.pine` file owns.
    private struct Record: Codable {
        var id: UUID
        var type: ScriptType
        var latestRevisionID: UUID?
        var createdAt: Date
        var modifiedAt: Date
        var lastOpenedAt: Date?
        var isFavorite: Bool
        var compileRecord: ScriptCompileRecord?

        init(_ script: LocalScript) {
            id = script.id
            type = script.type
            latestRevisionID = script.latestRevisionID
            createdAt = script.createdAt
            modifiedAt = script.modifiedAt
            lastOpenedAt = script.lastOpenedAt
            isFavorite = script.isFavorite
            compileRecord = script.compileRecord
        }

        func script(name: String, source: String) -> LocalScript {
            LocalScript(
                id: id, name: name, type: type, source: source, latestRevisionID: latestRevisionID,
                createdAt: createdAt, modifiedAt: modifiedAt, lastOpenedAt: lastOpenedAt,
                isFavorite: isFavorite, compileRecord: compileRecord)
        }
    }

    nonisolated let scriptsDirectory: URL
    private let metadataRoot: URL
    private let fm: FileManager
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private var metadata: [UUID: LocalScript] = [:]
    /// Script id → file name inside `scriptsDirectory`; persisted as `index.json`.
    private var fileNames: [UUID: String] = [:]
    private var loaded = false

    init(scriptsDirectory: URL, metadataDirectory: URL, fileManager: FileManager = .default) {
        self.scriptsDirectory = scriptsDirectory
        metadataRoot = metadataDirectory
        fm = fileManager
        encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
    }

    /// Always rescans the folder, so files added, edited, or renamed outside the app show up.
    func allScripts() throws -> [LocalScript] {
        try rescan()
        return metadata.values.sorted { $0.modifiedAt > $1.modifiedAt }
    }

    @discardableResult
    func create(
        name requestedName: String, type: ScriptType, source: String? = nil, disambiguating: Bool = true
    ) throws -> LocalScript {
        try loadIfNeeded()
        let base = try Self.validatedName(requestedName)
        if !disambiguating, isNameTaken(base) { throw ScriptStoreError.nameConflict }
        let name = disambiguating ? disambiguatedName(base) : base
        let now = Date()
        let id = UUID()
        let script = LocalScript(
            id: id, name: name, type: type,
            source: source ?? Self.template(for: type, title: name), latestRevisionID: nil,
            createdAt: now, modifiedAt: now, lastOpenedAt: nil, isFavorite: false,
            compileRecord: nil)
        try writeFile(id: id, name: name, source: script.source)
        try write(script)
        metadata[id] = script
        try saveIndex()
        return script
    }

    /// Copies an outside `.pine` file into the library. The copy is named after the file
    /// (disambiguated on conflict) and typed by the script's own declaration.
    func importFile(at url: URL) throws -> LocalScript {
        guard let source = Self.readSource(at: url) else { throw ScriptStoreError.unreadableFile }
        let name = url.deletingPathExtension().lastPathComponent
        let type = PineCompiler.compile(source: source).declaration.type
        return try create(name: name, type: type, source: source)
    }

    /// The id of the library script stored at `url`, or nil when `url` is outside the library.
    func scriptID(forFileAt url: URL) throws -> UUID? {
        let folder = url.deletingLastPathComponent().resolvingSymlinksInPath().standardizedFileURL
        guard folder.path == scriptsDirectory.resolvingSymlinksInPath().standardizedFileURL.path else { return nil }
        try rescan()
        let name = url.lastPathComponent
        return fileNames.first { $0.value == name }?.key
    }

    /// Where the script's `.pine` file lives inside the library.
    func fileURL(for id: UUID) throws -> URL? {
        try loadIfNeeded()
        return fileNames[id].map { scriptsDirectory.appendingPathComponent($0) }
    }

    func script(id: UUID) throws -> LocalScript? {
        try rescan()
        return metadata[id]
    }

    /// Saves new source under the script's current name. The type follows the script's own
    /// declaration; while the source has no single declaration (mid-edit) it keeps the old one.
    func save(id: UUID, source: String) throws -> LocalScript {
        try loadIfNeeded()
        guard var script = metadata[id] else { throw ScriptStoreError.missingScript }
        // Invalid source saves too; its errors are recorded in the compile record.
        let compiled = PineCompiler.compile(source: source)
        let status = Self.status(for: compiled.diagnostics)
        let now = Date()
        if source != script.source || script.latestRevisionID == nil {
            let revision = ScriptVersion(
                id: UUID(), scriptID: id, createdAt: now,
                source: source, compileStatus: status)
            try write(revision)
            script.latestRevisionID = revision.id
        }
        script.type = Self.hasSingleDeclaration(compiled) ? compiled.declaration.type : script.type
        script.source = source
        script.modifiedAt = now
        script.compileRecord = ScriptCompileRecord(
            sourceHash: Self.hash(source),
            compilerVersion: Self.compilerVersion, pineVersion: compiled.declaration.pineVersion,
            status: status, diagnostics: compiled.diagnostics, declaration: compiled.declaration,
            compiledAt: now)
        try writeFile(id: id, name: script.name, source: source)
        try write(script)
        try removeDraft(id: id)
        metadata[id] = script
        try saveIndex()
        try pruneRevisions(for: id, keeping: 100)
        return script
    }

    /// Renames the script's file and nothing else: source, revisions and draft are untouched.
    @discardableResult
    func rename(id: UUID, to newName: String) throws -> LocalScript {
        try loadIfNeeded()
        guard var script = metadata[id] else { throw ScriptStoreError.missingScript }
        let cleanName = try Self.validatedName(newName)
        guard cleanName != script.name else { return script }
        if isNameTaken(cleanName, excluding: id) { throw ScriptStoreError.nameConflict }
        try renameFile(id: id, to: cleanName)
        script.name = cleanName
        script.modifiedAt = Date()
        try write(script)
        metadata[id] = script
        try saveIndex()
        return script
    }

    func saveDraft(_ draft: ScriptDraft) throws {
        try loadIfNeeded()
        guard metadata[draft.scriptID] != nil else { throw ScriptStoreError.missingScript }
        try atomicWrite(draft, to: directory(draft.scriptID).appendingPathComponent("draft.json"))
    }

    func draft(id: UUID) throws -> ScriptDraft? {
        try loadIfNeeded()
        return try decodeIfPresent(ScriptDraft.self, at: directory(id).appendingPathComponent("draft.json"))
    }

    func revisions(id: UUID) throws -> [ScriptVersion] {
        try loadIfNeeded()
        let url = directory(id).appendingPathComponent("Revisions")
        let files = (try? fm.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
        return try files.filter { $0.pathExtension == "json" }.compactMap {
            try decodeIfPresent(ScriptVersion.self, at: $0)
        }
        .sorted { $0.createdAt > $1.createdAt }
    }

    func restore(scriptID: UUID, revisionID: UUID) throws -> LocalScript {
        guard let revision = try revisions(id: scriptID).first(where: { $0.id == revisionID }) else {
            throw ScriptStoreError.missingRevision
        }
        return try save(id: scriptID, source: revision.source)
    }

    func setFavorite(id: UUID, _ favorite: Bool) throws {
        try loadIfNeeded()
        guard var script = metadata[id] else { throw ScriptStoreError.missingScript }
        script.isFavorite = favorite
        try write(script)
        metadata[id] = script
    }

    /// Moves the script's file to the Trash (so it can be recovered) and drops its metadata.
    func delete(id: UUID) throws {
        try loadIfNeeded()
        guard metadata[id] != nil else { return }
        if let fileName = fileNames[id] {
            let file = scriptsDirectory.appendingPathComponent(fileName)
            if fm.fileExists(atPath: file.path) {
                do { try fm.trashItem(at: file, resultingItemURL: nil) } catch { try fm.removeItem(at: file) }
            }
        }
        let tombstone = metadataRoot.appendingPathComponent(".deleting-\(id.uuidString)")
        let dir = directory(id)
        if fm.fileExists(atPath: dir.path) { try fm.moveItem(at: dir, to: tombstone) }
        metadata.removeValue(forKey: id)
        fileNames.removeValue(forKey: id)
        try saveIndex()
        try? fm.removeItem(at: tombstone)
    }

    static func template(for type: ScriptType, title: String) -> String {
        let safe = title.replacingOccurrences(of: "\"", with: "'")
        switch type {
        case .indicator: return "//@version=6\nindicator(\"\(safe)\", overlay=true)\nplot(close)\n"
        case .strategy: return "//@version=6\nstrategy(\"\(safe)\", overlay=true)\n"
        case .library: return "//@version=6\nlibrary(\"\(safe)\")\n"
        }
    }

    // MARK: - Scanning

    private func loadIfNeeded() throws {
        guard !loaded else { return }
        try rescan()
    }

    /// Rebuilds the in-memory catalog from the `.pine` files on disk.
    private func rescan() throws {
        try fm.createDirectory(at: scriptsDirectory, withIntermediateDirectories: true)
        try fm.createDirectory(at: metadataRoot, withIntermediateDirectories: true)
        if !loaded { fileNames = (try? decodeIfPresent([UUID: String].self, at: indexURL)) ?? [:] }

        let files = ((try? fm.contentsOfDirectory(at: scriptsDirectory, includingPropertiesForKeys: nil)) ?? [])
            .filter {
                $0.pathExtension.caseInsensitiveCompare(Self.fileExtension) == .orderedSame
                    && !$0.lastPathComponent.hasPrefix(".")
            }
            .map { (url: $0, attributeID: Self.readID(at: $0)) }
            // A Finder duplicate copies the extended attribute too. The file the index
            // already knows under that id keeps it; any other claimant gets a new id.
            .sorted { lhs, rhs in
                let lhsKnown = lhs.attributeID.map { fileNames[$0] == lhs.url.lastPathComponent } ?? false
                let rhsKnown = rhs.attributeID.map { fileNames[$0] == rhs.url.lastPathComponent } ?? false
                if lhsKnown != rhsKnown { return lhsKnown }
                return lhs.url.lastPathComponent < rhs.url.lastPathComponent
            }
        let idsByName = Dictionary(fileNames.map { ($1, $0) }, uniquingKeysWith: { first, _ in first })

        var scripts: [UUID: LocalScript] = [:]
        var names: [UUID: String] = [:]
        for (url, attributeID) in files {
            guard let source = Self.readSource(at: url) else { continue }
            let fileName = url.lastPathComponent
            var id = attributeID
            if id == nil || scripts[id!] != nil { id = idsByName[fileName] }
            if id == nil || scripts[id!] != nil { id = UUID() }
            let scriptID = id!
            if attributeID != scriptID { Self.writeID(scriptID, at: url) }

            let values = try? url.resourceValues(forKeys: [.creationDateKey, .contentModificationDateKey])
            let fileModified = values?.contentModificationDate ?? Date()
            var record: Record
            if let existing = try? decodeIfPresent(Record.self, at: recordURL(scriptID)) {
                record = existing
            } else {
                let now = Date()
                record = Record(
                    LocalScript(
                        id: scriptID, name: "", type: PineCompiler.compile(source: source).declaration.type,
                        source: "", latestRevisionID: nil, createdAt: values?.creationDate ?? now,
                        modifiedAt: fileModified, lastOpenedAt: nil, isFavorite: false, compileRecord: nil))
                try? writeRecord(record)
            }
            // Edited outside the app: the cached diagnostics describe another source.
            if record.compileRecord?.sourceHash != Self.hash(source) { record.compileRecord = nil }
            record.modifiedAt = max(record.modifiedAt, fileModified)
            scripts[scriptID] = record.script(name: url.deletingPathExtension().lastPathComponent, source: source)
            names[scriptID] = fileName
        }

        metadata = scripts
        let indexChanged = names != fileNames
        fileNames = names
        loaded = true
        if indexChanged { try saveIndex() }
    }

    // MARK: - Files

    private var indexURL: URL { metadataRoot.appendingPathComponent("index.json") }
    private func directory(_ id: UUID) -> URL { metadataRoot.appendingPathComponent(id.uuidString, isDirectory: true) }
    private func recordURL(_ id: UUID) -> URL { directory(id).appendingPathComponent("script.json") }
    private func fileURL(forName name: String) -> URL {
        scriptsDirectory.appendingPathComponent(name).appendingPathExtension(Self.fileExtension)
    }
    /// Defense in depth behind `ScriptNameValidator`: a file must land directly in the library.
    private func assertInsideLibrary(_ url: URL) throws {
        guard url.deletingLastPathComponent().standardizedFileURL.path == scriptsDirectory.standardizedFileURL.path
        else { throw ScriptNameError.forbiddenCharacter("/") }
    }

    private func writeFile(id: UUID, name: String, source: String) throws {
        let url = fileURL(forName: name)
        try assertInsideLibrary(url)
        try Data(source.utf8).write(to: url, options: .atomic)
        // An atomic write replaces the file, dropping the previous extended attributes.
        Self.writeID(id, at: url)
        fileNames[id] = url.lastPathComponent
    }

    private func renameFile(id: UUID, to name: String) throws {
        guard let oldName = fileNames[id] else { return }
        let source = scriptsDirectory.appendingPathComponent(oldName)
        let destination = fileURL(forName: name)
        try assertInsideLibrary(destination)
        guard fm.fileExists(atPath: source.path) else { return }
        if oldName.caseInsensitiveCompare(destination.lastPathComponent) == .orderedSame {
            // A case-only rename on a case-insensitive volume: the destination "exists".
            let temporary = scriptsDirectory.appendingPathComponent(".renaming-\(id.uuidString)")
            try fm.moveItem(at: source, to: temporary)
            try fm.moveItem(at: temporary, to: destination)
        } else {
            guard !fm.fileExists(atPath: destination.path) else { throw ScriptStoreError.nameConflict }
            try fm.moveItem(at: source, to: destination)
        }
        fileNames[id] = destination.lastPathComponent
    }

    private func saveIndex() throws { try atomicWrite(fileNames, to: indexURL) }

    private func write(_ script: LocalScript) throws { try writeRecord(Record(script)) }
    private func writeRecord(_ record: Record) throws {
        try fm.createDirectory(at: directory(record.id), withIntermediateDirectories: true)
        try atomicWrite(record, to: recordURL(record.id))
    }
    private func write(_ revision: ScriptVersion) throws {
        let dir = directory(revision.scriptID).appendingPathComponent("Revisions", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try atomicWrite(revision, to: dir.appendingPathComponent("\(revision.id.uuidString).json"))
    }
    private func atomicWrite<T: Encodable>(_ value: T, to url: URL) throws {
        let data = try encoder.encode(value)
        try data.write(to: url, options: .atomic)
    }
    private func decodeIfPresent<T: Decodable>(_ type: T.Type, at url: URL) throws -> T? {
        guard fm.fileExists(atPath: url.path) else { return nil }
        return try decoder.decode(type, from: Data(contentsOf: url))
    }
    private func removeDraft(id: UUID) throws {
        try? fm.removeItem(at: directory(id).appendingPathComponent("draft.json"))
    }

    // MARK: - Helpers

    private func takenNames(excluding id: UUID? = nil) -> Set<String> {
        var taken = Set(metadata.values.filter { $0.id != id }.map { $0.name.lowercased() })
        // Files not (yet) in the catalog still occupy their names on disk.
        let own = id.flatMap { fileNames[$0] }
        let onDisk = ((try? fm.contentsOfDirectory(atPath: scriptsDirectory.path)) ?? []).filter { $0 != own }
        taken.formUnion(onDisk.map { Self.stem($0).lowercased() })
        return taken
    }
    private func isNameTaken(_ name: String, excluding id: UUID? = nil) -> Bool {
        takenNames(excluding: id).contains(name.lowercased())
    }
    private func disambiguatedName(_ base: String) -> String {
        disambiguatedName(base, taken: takenNames())
    }
    private func disambiguatedName(_ base: String, taken: Set<String>) -> String {
        if !taken.contains(base.lowercased()) { return base }
        var n = 2
        while taken.contains("\(base) \(n)".lowercased()) { n += 1 }
        return "\(base) \(n)"
    }
    private func pruneRevisions(for id: UUID, keeping count: Int) throws {
        for revision in try revisions(id: id).dropFirst(count) {
            try? fm.removeItem(at: directory(id).appendingPathComponent("Revisions/\(revision.id.uuidString).json"))
        }
    }

    /// Script names double as file names, so they are validated before any path is built.
    private static func validatedName(_ name: String) throws -> String {
        try ScriptNameValidator.validate(name).get()
    }

    /// False when the compiler reported a missing or duplicated declaration (PINE3001), in
    /// which case `declaration.type` is a default rather than something the script declared.
    private static func hasSingleDeclaration(_ compiled: PineCompiledProgram) -> Bool {
        !compiled.diagnostics.contains { $0.code == "PINE3001" }
    }
    private static func stem(_ fileName: String) -> String { (fileName as NSString).deletingPathExtension }

    private static func readSource(at url: URL) -> String? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1)
    }

    private static func readID(at url: URL) -> UUID? {
        url.withUnsafeFileSystemRepresentation { path -> UUID? in
            guard let path else { return nil }
            var buffer = [UInt8](repeating: 0, count: 64)
            let length = getxattr(path, idAttribute, &buffer, buffer.count, 0, 0)
            guard length > 0 else { return nil }
            return UUID(uuidString: String(decoding: buffer.prefix(length), as: UTF8.self))
        }
    }
    private static func writeID(_ id: UUID, at url: URL) {
        let value = Array(id.uuidString.utf8)
        url.withUnsafeFileSystemRepresentation { path in
            guard let path else { return }
            _ = setxattr(path, idAttribute, value, value.count, 0, 0)
        }
    }

    private static func status(for diagnostics: [PineDiagnostic]) -> CompileStatus {
        if diagnostics.contains(where: { $0.severity == .error }) { return .error }
        if diagnostics.contains(where: { $0.severity == .warning }) { return .warning }
        return .valid
    }
    private static func hash(_ source: String) -> String {
        ScriptSourceHash.sha256(source)
    }
}

/// Watches the scripts folder so edits made in Finder or another editor reach open views.
final class ScriptFolderMonitor {
    static let shared = ScriptFolderMonitor()
    private var source: DispatchSourceFileSystemObject?
    private var pending: DispatchWorkItem?

    func start(watching directory: URL = ScriptStore.shared.scriptsDirectory) {
        guard source == nil else { return }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor, eventMask: [.write, .rename, .delete], queue: .main)
        source.setEventHandler { [weak self] in self?.scheduleNotification() }
        source.setCancelHandler { close(descriptor) }
        source.resume()
        self.source = source
    }

    /// Atomic saves touch the folder several times per write; coalesce them into one refresh.
    private func scheduleNotification() {
        pending?.cancel()
        let work = DispatchWorkItem { NotificationCenter.default.post(name: .localScriptsDidChange, object: nil) }
        pending = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }
}
