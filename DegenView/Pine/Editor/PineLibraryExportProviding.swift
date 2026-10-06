import Foundation

/// One name an imported library exports.
struct PineLibraryExport: Equatable, Sendable {
    enum Kind: Equatable, Sendable {
        case function, method, variable, type, enumeration
    }

    let name: String
    let kind: Kind
    let parameters: [PineSourceSymbolIndex.Parameter]?

    /// `myEMA(source, length)` for a callable, the bare name otherwise.
    var signatureText: String {
        guard let parameters else { return name }
        return "\(name)(\(parameters.map(\.name).joined(separator: ", ")))"
    }
}

/// What the editor knows about the libraries a script can import, read from memory only.
/// The app's implementation reads `PineLibraryRegistry`; tests substitute their own.
protocol PineLibraryExportProviding: Sendable {
    /// The exports of the library an import path (`user/Library/1`) names, or nil when none matches.
    func exports(forImportPath path: String) -> [PineLibraryExport]?

    /// The names of the libraries available to import, sorted.
    func libraryNames() -> [String]
}

/// No libraries: every alias is unknown.
struct PineNoLibraryExports: PineLibraryExportProviding {
    func exports(forImportPath path: String) -> [PineLibraryExport]? { nil }
    func libraryNames() -> [String] { [] }
}
