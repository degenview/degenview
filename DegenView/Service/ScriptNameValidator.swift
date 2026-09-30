import Foundation

enum ScriptNameError: LocalizedError, Equatable {
    case empty
    case hidden
    case forbiddenCharacter(Character)
    case controlCharacter
    case tooLong

    var errorDescription: String? {
        switch self {
        case .empty: return "Enter a name for the script."
        case .hidden: return "Script names cannot start with a period."
        case .forbiddenCharacter(let character):
            return "Script names cannot contain “\(character)”."
        case .controlCharacter: return "Script names cannot contain control characters."
        case .tooLong: return "Script names must be \(ScriptNameValidator.maxNameBytes) bytes or fewer."
        }
    }
}

/// Script names double as file names inside the library folder, so anything that could
/// escape it or produce an unusable file is rejected up front, before any disk access.
enum ScriptNameValidator {
    /// The 255-byte file-name limit minus room for the `.pine` extension.
    static let maxNameBytes = 250
    private static let forbidden: Set<Character> = ["/", "\\", ":"]

    /// The name as it will be stored: trimmed, without a typed `.pine` extension.
    static func validate(_ raw: String) -> Result<String, ScriptNameError> {
        var name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let suffix = "." + ScriptStore.fileExtension
        if name.count > suffix.count, name.lowercased().hasSuffix(suffix) {
            name = String(name.dropLast(suffix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        guard !name.isEmpty else { return .failure(.empty) }
        // Covers ".", "..", and "../x": a traversal attempt always trips this or a separator.
        guard !name.hasPrefix(".") else { return .failure(.hidden) }
        if let bad = name.first(where: { forbidden.contains($0) }) { return .failure(.forbiddenCharacter(bad)) }
        guard !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            return .failure(.controlCharacter)
        }
        guard name.utf8.count <= maxNameBytes else { return .failure(.tooLong) }
        return .success(name)
    }
}
