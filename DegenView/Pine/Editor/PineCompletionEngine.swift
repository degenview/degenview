import Foundation

/// Turns a completion context into candidates. Pure and deterministic: it reads the analysis
/// snapshot, the builtin catalog and what the libraries export, and nothing else. It never
/// compiles, runs a script, touches the network or the database.
///
/// The candidates depend on what the user is writing. After `ta.` only members of `ta` are
/// listed; a bare word lists what is visible at the caret (the script's own names, innermost
/// scope first) and then the builtins, with the script's names hiding builtins of the same name,
/// as they do when the script compiles.
enum PineCompletionEngine {
    static func complete(
        _ context: PineCompletionContext, analysis: PineEditorAnalysisSnapshot,
        libraries: PineLibraryExportProviding = PineNoLibraryExports()
    ) -> [PineCompletionItem] {
        guard context.suppression == nil else { return [] }
        let sources = Sources(context: context, analysis: analysis, libraries: libraries)
        var items: [PineCompletionItem]
        switch context.position {
        case .member(let base): items = sources.members(of: base)
        case .importPath(let typed): items = sources.importPaths(after: typed)
        case .typeExpected: items = sources.types()
        case .statementStart: items = sources.expression(atStatementStart: true)
        case .expression: items = sources.expression(atStatementStart: false)
        }
        switch context.position {
        case .statementStart, .expression: items += sources.argumentNames()
        default: break
        }
        let lowered = context.prefix.lowercased()
        items = items.filter { $0.label.lowercased().hasPrefix(lowered) }
        return PineCompletionRanking.sorted(unique(items), prefix: context.prefix)
    }

    /// One candidate per label: the nearest scope's, or the better-tiered, wins.
    private static func unique(_ items: [PineCompletionItem]) -> [PineCompletionItem] {
        var best: [String: PineCompletionItem] = [:]
        var order: [String] = []
        for item in items {
            if let existing = best[item.label] {
                if item.tier < existing.tier { best[item.label] = item }
            } else {
                best[item.label] = item
                order.append(item.label)
            }
        }
        return order.compactMap { best[$0] }
    }
}

// MARK: - Candidate sources

extension PineCompletionEngine {
    fileprivate struct Sources {
        let context: PineCompletionContext
        let analysis: PineEditorAnalysisSnapshot
        let libraries: PineLibraryExportProviding

        private var index: PineSourceSymbolIndex { analysis.index }
        private var range: NSRange { context.replacementRange }

        init(
            context: PineCompletionContext, analysis: PineEditorAnalysisSnapshot,
            libraries: PineLibraryExportProviding
        ) {
            self.context = context
            self.analysis = analysis
            self.libraries = libraries
        }

        // MARK: Expressions

        func expression(atStatementStart: Bool) -> [PineCompletionItem] {
            let visible = index.visibleDeclarations(scope: context.scope, atOffset: context.caret)
            var items = visible.compactMap(userItem)
            // A name the script declares hides the builtin of the same name.
            let hidden = Set(visible.filter(\.isValue).map(\.name))

            for imported in index.imports {
                guard let alias = imported.alias, !hidden.contains(alias) else { continue }
                items.append(
                    make(
                        path: alias, label: alias, kind: .library, origin: .library(alias: alias),
                        style: .namespace, detail: imported.path, tier: .library))
            }
            let typeOnly = atStatementStart ? PineSymbolCatalog.typeNames.filter { isTypeOnly($0) } : []
            for member in PineSymbolCatalog.members(of: "")
            where !hidden.contains(member.label) && !typeOnly.contains(member.label) {
                items.append(builtinItem(member))
            }
            if atStatementStart {
                items += types(onlyBuiltin: true)
                items += keywords(PineSymbolCatalog.statementKeywords)
            } else {
                items += keywords(PineSymbolCatalog.expressionKeywords)
            }
            return items
        }

        /// `int`, `float`, `bool`, `string`: types with no namespace of their own.
        private func isTypeOnly(_ name: String) -> Bool {
            PineSymbolCatalog.members(of: "").first { $0.label == name }?.kind != .namespace
        }

        func types(onlyBuiltin: Bool = false) -> [PineCompletionItem] {
            var items: [PineCompletionItem] = []
            for name in PineSymbolCatalog.typeNames.sorted() where !onlyBuiltin || isTypeOnly(name) {
                items.append(
                    make(path: name, label: name, kind: .type, origin: .builtin, style: .identifier, tier: .type))
            }
            if !onlyBuiltin {
                for declaration in index.visibleDeclarations(scope: context.scope, atOffset: context.caret)
                where declaration.kind == .type || declaration.kind == .enumeration {
                    items.append(
                        make(
                            path: declaration.name, label: declaration.name, kind: .type, origin: .user,
                            style: .identifier, tier: .scriptGlobal))
                }
            }
            return items
        }

        private func keywords(_ words: [String]) -> [PineCompletionItem] {
            words.map {
                make(path: $0, label: $0, kind: .keyword, origin: .builtin, style: .identifier, tier: .keyword)
            }
        }

        // MARK: Users' own names

        private func userItem(_ declaration: PineSourceSymbolIndex.Declaration) -> PineCompletionItem? {
            let tier: PineCompletionRanking.Tier =
                declaration.scope == context.scope && declaration.scope != 0
                ? .innermostScope : (declaration.scope == 0 ? .scriptGlobal : .outerScope)
            switch declaration.kind {
            case .function, .method:
                let signature = PineSignatureResolver.userSignature(declaration)
                return make(
                    path: declaration.name, label: declaration.name, kind: .function, origin: .user,
                    style: .callable, detail: signature.label, signatures: [signature], tier: tier)
            case .parameter:
                return make(
                    path: declaration.name, label: declaration.name, kind: .parameter, origin: .user,
                    style: .identifier, detail: declaration.typeName, tier: tier)
            case .variable, .loopVariable:
                let isLocal = declaration.scope != 0 || declaration.kind == .loopVariable
                return make(
                    path: declaration.name, label: declaration.name, kind: isLocal ? .local : .variable,
                    origin: .user, style: .identifier, detail: declaration.typeName, tier: tier)
            case .type, .enumeration:
                return make(
                    path: declaration.name, label: declaration.name, kind: .type, origin: .user,
                    style: .namespace, detail: declaration.kind == .type ? "type" : "enum", tier: tier)
            }
        }

        // MARK: Builtins

        private func builtinItem(_ member: PineCatalogMember) -> PineCompletionItem {
            let name = member.qualifiedName
            let tier: PineCompletionRanking.Tier = context.isMember ? .member : .builtin
            switch member.kind {
            case .function:
                let signatures = PineSymbolMetadata.table[name] ?? []
                return make(
                    path: name, label: member.label, kind: .function, origin: .builtin, style: .callable,
                    detail: signatures.first?.label ?? "\(name)(…)", documentation: signatures.first?.summary,
                    signatures: signatures, tier: tier)
            case .variable:
                return make(
                    path: name, label: member.label, kind: .variable, origin: .builtin, style: .identifier,
                    detail: typeText(of: name), documentation: PineSymbolMetadata.variableSummaries[name],
                    tier: tier)
            case .constant:
                return make(
                    path: name, label: member.label, kind: .constant, origin: .builtin, style: .identifier,
                    detail: typeText(of: name), documentation: PineSymbolMetadata.variableSummaries[name],
                    tier: tier)
            case .namespace:
                return make(
                    path: name, label: member.label, kind: .namespace, origin: .builtin, style: .namespace,
                    tier: context.isMember ? .member : .namespace)
            }
        }

        /// `series float`, `constant color`, or nil when the type checker does not know the name.
        private func typeText(of name: String) -> String? {
            guard case .known(let type, let qualifier) = PineBuiltinTypes.identifier(name) else { return nil }
            return [qualifier.map { String(describing: $0) }, type.rawValue].compactMap { $0 }.joined(
                separator: " ")
        }

        // MARK: Members

        func members(of base: [String]) -> [PineCompletionItem] {
            guard let head = base.first else { return [] }
            let declaration = index.resolve(head, scope: context.scope, atOffset: context.caret)
            if base.count == 1 {
                if let declaration {
                    guard declaration.isValue, let type = declaration.typeName else { return [] }
                    return instanceMembers(of: type)
                }
                if let imported = index.importDeclaration(alias: head) { return libraryMembers(of: imported) }
                if let type = index.types[head] { return typeMembers(of: type) }
                if let enumeration = index.enums[head] {
                    return enumeration.members.map {
                        make(
                            path: "\(head).\($0)", label: $0, kind: .enumMember, origin: .user,
                            style: .identifier, detail: head, tier: .member)
                    }
                }
            } else if declaration == nil, base.count == 2, let imported = index.importDeclaration(alias: head) {
                return libraryTypeMembers(of: imported, typeName: base[1])
            }
            // A name the script declares hides a builtin namespace of the same name.
            guard declaration == nil else { return [] }
            return PineSymbolCatalog.members(of: base.joined(separator: ".")).map(builtinItem)
        }

        /// Fields of a user type, and the methods declared with the type as their receiver.
        private func instanceMembers(of typeName: String) -> [PineCompletionItem] {
            var items: [PineCompletionItem] = []
            for field in index.types[typeName]?.fields ?? [] {
                items.append(
                    make(
                        path: "\(typeName).\(field.name)", label: field.name, kind: .field, origin: .user,
                        style: .identifier, detail: field.typeText, tier: .member))
            }
            let methods = index.declarations.filter {
                $0.kind == .method && $0.parameters?.first?.typeText == typeName
            }
            for method in methods {
                let signature = PineSignatureResolver.userSignature(method)
                let shown = PineSymbolSignature(
                    name: method.name, parameters: Array(signature.parameters.dropFirst()), returns: "", summary: nil)
                items.append(
                    make(
                        path: "\(typeName).\(method.name)", label: method.name, kind: .function, origin: .user,
                        style: .callable, detail: shown.label, signatures: [shown], tier: .member))
            }
            return items
        }

        /// `Point.` offers the constructor.
        private func typeMembers(of type: PineSourceSymbolIndex.UserType) -> [PineCompletionItem] {
            let signature = PineSignatureResolver.constructor(of: type)
            return [
                make(
                    path: "\(type.name).new", label: "new", kind: .function, origin: .user, style: .callable,
                    detail: signature.label, signatures: [signature], tier: .member)
            ]
        }

        private func libraryMembers(of imported: PineSourceSymbolIndex.Import) -> [PineCompletionItem] {
            guard let alias = imported.alias, let exports = libraries.exports(forImportPath: imported.path)
            else { return [] }
            return exports.map { export in
                let origin = PineCompletionOrigin.library(alias: alias)
                switch export.kind {
                case .function, .method:
                    let signature = PineSymbolSignature(
                        name: "\(alias).\(export.name)",
                        parameters: (export.parameters ?? []).map {
                            .init(
                                name: $0.name, type: $0.typeText ?? "any", defaultValue: $0.defaultText,
                                isOptional: $0.defaultText != nil)
                        }, returns: "", summary: nil)
                    return make(
                        path: "\(alias).\(export.name)", label: export.name, kind: .libraryMember, origin: origin,
                        style: .callable, detail: export.signatureText, signatures: [signature], tier: .member)
                case .variable:
                    return make(
                        path: "\(alias).\(export.name)", label: export.name, kind: .libraryMember, origin: origin,
                        style: .identifier, detail: alias, tier: .member)
                case .type, .enumeration:
                    return make(
                        path: "\(alias).\(export.name)", label: export.name, kind: .libraryMember, origin: origin,
                        style: .namespace, detail: export.kind == .type ? "type" : "enum", tier: .member)
                }
            }
        }

        /// `lib.Point.` offers the constructor of an exported type.
        private func libraryTypeMembers(
            of imported: PineSourceSymbolIndex.Import, typeName: String
        ) -> [PineCompletionItem] {
            guard let alias = imported.alias, let exports = libraries.exports(forImportPath: imported.path),
                exports.contains(where: { $0.name == typeName && $0.kind == .type })
            else { return [] }
            return [
                make(
                    path: "\(alias).\(typeName).new", label: "new", kind: .libraryMember,
                    origin: .library(alias: alias), style: .callable, detail: "\(alias).\(typeName).new(…)",
                    tier: .member)
            ]
        }

        // MARK: Imports

        /// `import |` offers `user/`; `import user/|` offers the libraries on this machine.
        func importPaths(after typed: String) -> [PineCompletionItem] {
            if typed.isEmpty {
                return [
                    make(
                        path: "user/", label: "user", kind: .library, origin: .builtin, style: .identifier,
                        detail: "Your Script Manager libraries", tier: .library)
                ]
            }
            guard typed == "user/" else { return [] }
            return libraries.libraryNames().map {
                make(
                    path: "user/\($0)", label: $0, kind: .library, origin: .user, style: .identifier,
                    detail: "user/\($0)/1", tier: .library)
            }
        }

        // MARK: Named arguments

        /// `title = ` and the like, from the parameters of the call the caret is in.
        func argumentNames() -> [PineCompletionItem] {
            guard let site = context.callSite, site.isAtArgumentStart else { return [] }
            let forms = PineSignatureResolver.signatures(
                forCallee: site.callee, scope: context.scope, offset: context.caret, analysis: analysis,
                libraries: libraries)
            var seen = Set<String>()
            var items: [PineCompletionItem] = []
            for form in forms {
                for (position, parameter) in form.parameters.enumerated() {
                    // Positional arguments already given fill the leading parameters.
                    guard position >= site.positionalBefore, !site.namedArguments.contains(parameter.name),
                        seen.insert(parameter.name).inserted
                    else { continue }
                    items.append(
                        make(
                            path: "\(site.calleeName).\(parameter.name)", label: parameter.name,
                            kind: .argumentName, origin: .builtin, style: .argumentName,
                            detail: parameter.detail, documentation: form.summary, signatures: [form],
                            tier: .argumentName))
                }
            }
            return items
        }

        // MARK: Items

        private func make(
            path: String, label: String, kind: PineCompletionKind, origin: PineCompletionOrigin,
            style: PineCompletionInsertionStyle, detail: String? = nil, documentation: String? = nil,
            signatures: [PineSymbolSignature] = [], tier: PineCompletionRanking.Tier
        ) -> PineCompletionItem {
            PineCompletionItem(
                id: "\(origin)|\(path)|\(kind)", label: label, insertText: label, kind: kind, origin: origin,
                style: style, detail: detail, documentation: documentation, signatures: signatures,
                replacementRange: range, tier: tier)
        }
    }
}
