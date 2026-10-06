import Foundation

/// Finds the signatures behind a call, from what the editor already knows: the script's own
/// functions, imported libraries' exports and the builtin metadata. No compile, no execution.
enum PineSignatureResolver {
    /// The forms of `callee` visible in `scope` at `offset`, in the order a call resolves:
    /// a user function, a type's constructor, a library export, then a builtin.
    static func signatures(
        forCallee callee: [String], scope: Int, offset: Int, analysis: PineEditorAnalysisSnapshot,
        libraries: PineLibraryExportProviding
    ) -> [PineSymbolSignature] {
        let index = analysis.index
        guard let head = callee.first else { return [] }
        if callee.count == 1 {
            if let declaration = index.resolve(head, scope: scope, atOffset: offset) {
                // The script's own name wins over a builtin; a variable is not callable.
                return declaration.isCallable ? [userSignature(declaration)] : []
            }
        } else if let declaration = index.resolve(head, scope: scope, atOffset: offset) {
            // `variable.method(` calls a method declared on the variable's type.
            return methods(named: callee[1...].joined(separator: "."), on: declaration, in: index)
        }
        if callee.count == 2, callee[1] == "new", let type = index.types[head] {
            return [constructor(of: type)]
        }
        if callee.count >= 2, let imported = index.importDeclaration(alias: head),
            let exports = libraries.exports(forImportPath: imported.path)
        {
            let name = callee[1...].joined(separator: ".")
            if let export = exports.first(where: { $0.name == name && $0.parameters != nil }) {
                return [librarySignature(export, alias: head)]
            }
            return []
        }
        return PineSymbolMetadata.table[callee.joined(separator: ".")] ?? []
    }

    /// The help for the call under the caret, or nil outside a call or when the callee is unknown.
    static func help(
        for context: PineCompletionContext, analysis: PineEditorAnalysisSnapshot,
        libraries: PineLibraryExportProviding
    ) -> PineSignatureHelp? {
        guard context.suppression != .comment, context.suppression != .string,
            context.suppression != .selection, let site = context.callSite
        else { return nil }
        let forms = signatures(
            forCallee: site.callee, scope: context.scope, offset: context.caret, analysis: analysis,
            libraries: libraries)
        guard !forms.isEmpty else { return nil }
        let ranked = forms.enumerated().sorted { lhs, rhs in
            let left = fit(lhs.element, site)
            let right = fit(rhs.element, site)
            return left == right ? lhs.offset < rhs.offset : left > right
        }
        let ordered = ranked.map(\.element)
        let active = ordered[0]
        var parameter: Int?
        if let name = site.activeName {
            parameter = active.parameters.firstIndex { $0.name == name }
        } else if site.argumentIndex < active.parameters.count {
            parameter = site.argumentIndex
        }
        return PineSignatureHelp(
            signatures: ordered, activeSignature: 0, activeParameter: parameter, callee: site.calleeName,
            opener: site.opener)
    }

    /// How well a form fits the call so far: every named argument must be one of its parameters,
    /// and it must have a parameter for the argument under the caret.
    private static func fit(_ signature: PineSymbolSignature, _ site: PineCallSite) -> Int {
        let names = Set(signature.parameters.map(\.name))
        var score = 0
        if site.namedArguments.isSubset(of: names) { score += 2 }
        if site.argumentIndex < signature.parameters.count { score += 1 }
        return score
    }

    // MARK: - Sources

    static func userSignature(_ declaration: PineSourceSymbolIndex.Declaration) -> PineSymbolSignature {
        PineSymbolSignature(
            name: declaration.name,
            parameters: (declaration.parameters ?? []).map {
                .init(
                    name: $0.name, type: $0.typeText ?? "any", defaultValue: $0.defaultText,
                    isOptional: $0.defaultText != nil)
            }, returns: "", summary: nil)
    }

    static func constructor(of type: PineSourceSymbolIndex.UserType) -> PineSymbolSignature {
        PineSymbolSignature(
            name: "\(type.name).new",
            parameters: type.fields.map {
                .init(
                    name: $0.name, type: $0.typeText ?? "any", defaultValue: $0.defaultText, isOptional: true)
            }, returns: type.name, summary: nil)
    }

    private static func librarySignature(_ export: PineLibraryExport, alias: String) -> PineSymbolSignature {
        PineSymbolSignature(
            name: "\(alias).\(export.name)",
            parameters: (export.parameters ?? []).map {
                .init(
                    name: $0.name, type: $0.typeText ?? "any", defaultValue: $0.defaultText,
                    isOptional: $0.defaultText != nil)
            }, returns: "", summary: nil)
    }

    /// Methods of the variable's user type that take it as their first parameter.
    private static func methods(
        named name: String, on variable: PineSourceSymbolIndex.Declaration, in index: PineSourceSymbolIndex
    ) -> [PineSymbolSignature] {
        guard let type = variable.typeName else { return [] }
        return index.declarations.filter {
            $0.kind == .method && $0.name == name && $0.parameters?.first?.typeText == type
        }.map { declaration in
            // The receiver is the first parameter; a call does not write it.
            var signature = userSignature(declaration)
            signature = PineSymbolSignature(
                name: "\(variable.name).\(declaration.name)", parameters: Array(signature.parameters.dropFirst()),
                returns: "", summary: nil)
            return signature
        }
    }
}
