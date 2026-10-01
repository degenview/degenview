import Foundation

/// What Pine's builtin names are, for tooling that must tell a builtin from a user symbol
/// without compiling (the editor's highlighter).
///
/// This composes the tables the type checker and the runtime already own, so a symbol added
/// there is known here too. Only names Pine defines but DegenView does not implement yet are
/// listed by hand, under `reserved…`; `PineSymbolCatalogTests` guards that nothing the runtime
/// dispatches falls out of the catalog.
enum PineSymbolCatalog {
    // MARK: - Implemented

    /// Runtime values that change per bar or per symbol: `close`, `bar_index`,
    /// `barstate.isconfirmed`, `syminfo.tickerid`, `strategy.position_size`.
    static let variables: Set<String> = {
        var names = PineBuiltinTypes.floatSeries
        names.formUnion(PineBuiltinTypes.intSeries)
        names.formUnion(PineBuiltinTypes.simpleStrings)
        names.formUnion(PineBuiltinTypes.simpleFloats)
        names.formUnion(PineBuiltinTypes.simpleColors)
        names.formUnion(PineBarFlags.names)
        names.formUnion(PineTime.partNames)
        names.formUnion(PineTime.timeframeNames)
        names.formUnion(reservedVariables)
        return names
    }()

    /// Fixed values: `color.red`, `math.pi`, `size.tiny`, `line.style_dotted`, `strategy.long`.
    static let constants: Set<String> = {
        var names = Set(PineBuiltins.colors.keys)
        names.formUnion(PineBuiltins.constants.keys)
        for list in namedConstants { names.formUnion(list) }
        names.formUnion(reservedConstants)
        return names
    }()

    /// Builtins callable without a namespace: `plot`, `indicator`, `nz`, `timestamp`, `year`.
    /// Namespaced calls (`ta.ema`) are not listed: a call on a builtin namespace is a builtin.
    static let functions: Set<String> = {
        var names = Set(PineRuntimeSession.exactHandlers.keys.filter { !$0.contains(".") })
        names.formUnion(PineRuntimeSession.visualNames)
        names.formUnion(PineTime.partNames)
        names.formUnion(reservedFunctions)
        return names
    }()

    /// First segments of every dotted builtin: `ta`, `color`, `barstate`, `strategy`, …
    static let namespaces: Set<String> = {
        var names = Set<String>()
        let dotted = [variables, constants, Set(PineRuntimeSession.exactHandlers.keys)]
        for name in dotted.joined() {
            if let dot = name.firstIndex(of: ".") { names.insert(String(name[..<dot])) }
        }
        for entry in PineRuntimeSession.namespaceHandlers {
            names.insert(String(entry.prefix.dropLast()))
        }
        names.formUnion(reservedNamespaces)
        return names
    }()

    /// Namespaces whose non-call members are values rather than anything callable, so a member
    /// the tables do not list (`strategy.max_drawdown`) is still a variable.
    static let valueNamespaces: Set<String> = [
        "barstate", "syminfo", "timeframe", "session", "chart", "strategy",
    ]

    // MARK: - Words the parser reads by spelling

    /// `const`, `input`, `simple`, `series`.
    static let qualifiers = Set(PineParser.qualifiers.keys)

    /// `line`, `label`, `box`, `table`, `array`, plus the handle and collection types Pine has.
    static let objectTypes = Set(PineParser.objectTypes.keys)

    /// Keywords of Pine that need more than the parser gives them: `type`, `method` and `export` are
    /// parsed but only in their declaration forms; `import` and `enum` are reported as unsupported.
    /// All are highlighted so a script written for full Pine reads correctly.
    static let reservedWords: Set<String> = ["import", "export", "method", "type", "enum", "as"]

    /// `in`, `to`, `by` are keywords only in a `for` header.
    static let forHeaderWords: Set<String> = ["in", "to", "by"]

    // MARK: - Defined by Pine, not implemented yet

    private static let namedConstants: [[String]] = [
        PineAlertFrequency.pineNames, PineMarkerShape.pineNames, PineLineExtend.pineNames,
        PineMarkerLocation.pineNames, PineTablePosition.pineNames, PineLineStyle.pineNames,
        PineSize.pineNames, PineLabelStyle.pineNames,
    ]

    private static let reservedVariables: Set<String> = [
        "hlcc4", "weekofyear",
        "session.isfirstbar", "session.isfirstbar_regular", "session.islastbar",
        "session.islastbar_regular", "session.ismarket", "session.ispremarket",
        "session.ispostmarket", "syminfo.prefix", "syminfo.root", "syminfo.session",
        "syminfo.timezone", "syminfo.pointvalue", "syminfo.mincontract", "syminfo.basecurrency",
        "syminfo.description", "syminfo.volumetype",
    ]

    private static let reservedConstants: Set<String> = [
        "session.regular", "session.extended", "xloc.bar_index", "xloc.bar_time", "yloc.price",
        "yloc.abovebar", "yloc.belowbar", "barmerge.gaps_on", "barmerge.gaps_off",
        "barmerge.lookahead_on", "barmerge.lookahead_off", "plot.style_line",
        "plot.style_stepline", "plot.style_steplinebr", "plot.style_histogram",
        "plot.style_cross", "plot.style_area", "plot.style_areabr", "plot.style_columns",
        "plot.style_circles", "hline.style_solid", "hline.style_dotted", "hline.style_dashed",
    ]

    private static let reservedFunctions: Set<String> = [
        "input", "fixnan", "plotarrow", "plotbar",
    ]

    private static let reservedNamespaces: Set<String> = [
        "request", "session", "chart", "log", "runtime",
        "barmerge", "xloc", "yloc",
    ]
}
