import CryptoKit
import Foundation

enum ScriptStoreError: LocalizedError {
    case missingScript, missingRevision, nameConflict, invalidName
    var errorDescription: String? {
        switch self {
        case .missingScript: return "The script no longer exists."
        case .missingRevision: return "The script revision no longer exists."
        case .nameConflict: return "A script with that name already exists."
        case .invalidName: return "Script names cannot be empty."
        }
    }
}

/// The only persistence authority for user-authored scripts. It deliberately has no
/// networking dependency: every operation is confined to the supplied local directory.
actor ScriptStore {
    static let shared = ScriptStore(rootDirectory: AppSupport.directory.appendingPathComponent("Scripts"))
    static let compilerVersion = "pine-local-2"

    private let root: URL
    private let fm: FileManager
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private var metadata: [UUID: LocalScript] = [:]
    private var loaded = false

    init(rootDirectory: URL, fileManager: FileManager = .default) {
        root = rootDirectory; fm = fileManager
        encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        decoder = JSONDecoder(); decoder.dateDecodingStrategy = .iso8601
    }

    func allScripts() throws -> [LocalScript] {
        try loadIfNeeded()
        return metadata.values.sorted { $0.modifiedAt > $1.modifiedAt }
    }

    @discardableResult
    func create(name requestedName: String, type: ScriptType, source: String? = nil) throws -> LocalScript {
        try loadIfNeeded()
        let base = requestedName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !base.isEmpty else { throw ScriptStoreError.invalidName }
        let name = disambiguatedName(base)
        let now = Date(), id = UUID()
        let script = LocalScript(id: id, name: name, type: type,
            source: source ?? Self.template(for: type, title: name), latestRevisionID: nil,
            createdAt: now, modifiedAt: now, lastOpenedAt: nil, isFavorite: false,
            compileRecord: nil)
        try write(script)
        metadata[id] = script
        return script
    }

    func script(id: UUID) throws -> LocalScript? { try loadIfNeeded(); return metadata[id] }

    func save(id: UUID, name: String, type: ScriptType, source: String) throws -> LocalScript {
        try loadIfNeeded()
        guard var script = metadata[id] else { throw ScriptStoreError.missingScript }
        let cleanName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanName.isEmpty else { throw ScriptStoreError.invalidName }
        if metadata.values.contains(where: { $0.id != id && $0.name.caseInsensitiveCompare(cleanName) == .orderedSame }) {
            throw ScriptStoreError.nameConflict
        }
        let compiled = PineCompiler.compile(source: source)
        let status = Self.status(for: compiled.diagnostics)
        let now = Date()
        if source != script.source || script.latestRevisionID == nil {
            let revision = ScriptVersion(id: UUID(), scriptID: id, createdAt: now,
                                         source: source, compileStatus: status)
            try write(revision)
            script.latestRevisionID = revision.id
        }
        script.name = cleanName; script.type = type; script.source = source; script.modifiedAt = now
        script.compileRecord = ScriptCompileRecord(sourceHash: Self.hash(source),
            compilerVersion: Self.compilerVersion, pineVersion: compiled.declaration.pineVersion,
            status: status, diagnostics: compiled.diagnostics, declaration: compiled.declaration,
            compiledAt: now)
        try write(script); try removeDraft(id: id)
        metadata[id] = script
        try pruneRevisions(for: id, keeping: 100)
        return script
    }

    func saveDraft(_ draft: ScriptDraft) throws {
        try loadIfNeeded(); guard metadata[draft.scriptID] != nil else { throw ScriptStoreError.missingScript }
        try atomicWrite(draft, to: directory(draft.scriptID).appendingPathComponent("draft.json"))
    }

    func draft(id: UUID) throws -> ScriptDraft? {
        try loadIfNeeded(); return try decodeIfPresent(ScriptDraft.self, at: directory(id).appendingPathComponent("draft.json"))
    }

    func revisions(id: UUID) throws -> [ScriptVersion] {
        try loadIfNeeded()
        let url = directory(id).appendingPathComponent("Revisions")
        let files = (try? fm.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
        return try files.filter { $0.pathExtension == "json" }.compactMap { try decodeIfPresent(ScriptVersion.self, at: $0) }
            .sorted { $0.createdAt > $1.createdAt }
    }

    func restore(scriptID: UUID, revisionID: UUID) throws -> LocalScript {
        guard let revision = try revisions(id: scriptID).first(where: { $0.id == revisionID }),
              let script = try script(id: scriptID) else { throw ScriptStoreError.missingRevision }
        return try save(id: scriptID, name: script.name, type: script.type, source: revision.source)
    }

    func setFavorite(id: UUID, _ favorite: Bool) throws {
        try loadIfNeeded(); guard var script = metadata[id] else { throw ScriptStoreError.missingScript }
        script.isFavorite = favorite; try write(script); metadata[id] = script
    }

    func delete(id: UUID) throws {
        try loadIfNeeded(); guard metadata[id] != nil else { return }
        let tombstone = root.appendingPathComponent(".deleting-\(id.uuidString)")
        let dir = directory(id)
        if fm.fileExists(atPath: dir.path) { try fm.moveItem(at: dir, to: tombstone) }
        metadata.removeValue(forKey: id)
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

    private func loadIfNeeded() throws {
        guard !loaded else { return }
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        let dirs = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        for dir in dirs where !dir.lastPathComponent.hasPrefix(".") {
            if let script = try decodeIfPresent(LocalScript.self, at: dir.appendingPathComponent("script.json")) { metadata[script.id] = script }
        }
        loaded = true
    }
    private func directory(_ id: UUID) -> URL { root.appendingPathComponent(id.uuidString, isDirectory: true) }
    private func write(_ script: LocalScript) throws {
        let dir = directory(script.id); try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try atomicWrite(script, to: dir.appendingPathComponent("script.json"))
    }
    private func write(_ revision: ScriptVersion) throws {
        let dir = directory(revision.scriptID).appendingPathComponent("Revisions", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try atomicWrite(revision, to: dir.appendingPathComponent("\(revision.id.uuidString).json"))
    }
    private func atomicWrite<T: Encodable>(_ value: T, to url: URL) throws {
        let data = try encoder.encode(value); try data.write(to: url, options: .atomic)
    }
    private func decodeIfPresent<T: Decodable>(_ type: T.Type, at url: URL) throws -> T? {
        guard fm.fileExists(atPath: url.path) else { return nil }
        return try decoder.decode(type, from: Data(contentsOf: url))
    }
    private func removeDraft(id: UUID) throws { try? fm.removeItem(at: directory(id).appendingPathComponent("draft.json")) }
    private func disambiguatedName(_ base: String) -> String {
        let names = Set(metadata.values.map { $0.name.lowercased() }); if !names.contains(base.lowercased()) { return base }
        var n = 2; while names.contains("\(base) \(n)".lowercased()) { n += 1 }; return "\(base) \(n)"
    }
    private func pruneRevisions(for id: UUID, keeping count: Int) throws {
        for revision in try revisions(id: id).dropFirst(count) {
            try? fm.removeItem(at: directory(id).appendingPathComponent("Revisions/\(revision.id.uuidString).json"))
        }
    }
    private static func status(for diagnostics: [PineDiagnostic]) -> CompileStatus {
        if diagnostics.contains(where: { $0.severity == .error }) { return .error }
        if diagnostics.contains(where: { $0.severity == .warning }) { return .warning }
        return .valid
    }
    private static func hash(_ source: String) -> String {
        SHA256.hash(data: Data(source.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}
