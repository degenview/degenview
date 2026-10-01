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

    /// Pine variables DegenView does not implement. Reading one gives `na` rather than failing the script:
    /// they describe an exchange, a session or a chart window the engine has no model of.
    private static let reservedVariables: Set<String> = [
        "weekofyear", "session.isfirstbar", "session.isfirstbar_regular", "session.islastbar",
        "session.islastbar_regular", "session.ismarket", "session.ispremarket",
        "session.ispostmarket", "syminfo.session", "syminfo.mincontract", "syminfo.basecurrency",
        "syminfo.description", "syminfo.volumetype", "chart.left_visible_bar_time",
        "chart.right_visible_bar_time",
    ]

    /// Whether `name` is a Pine variable this release does not implement (it evaluates to `na`).
    static func isUnimplementedVariable(_ name: String) -> Bool { reservedVariables.contains(name) }

    /// Pine's named constants that DegenView's tables do not own. Each evaluates to its own name, so the
    /// property is only that a name outside this catalog (a typo) is an error rather than a string.
    private static let reservedConstants: Set<String> = {
        var names: Set<String> = [
            "session.regular", "session.extended", "xloc.bar_index", "xloc.bar_time", "yloc.price",
            "yloc.abovebar", "yloc.belowbar", "barmerge.gaps_on", "barmerge.gaps_off",
            "barmerge.lookahead_on", "barmerge.lookahead_off", "scale.left", "scale.right", "scale.none",
            "adjustment.none", "adjustment.splits", "adjustment.dividends", "backadjustment.inherit",
            "backadjustment.off", "backadjustment.on", "settlement_as_close.inherit",
            "settlement_as_close.off", "settlement_as_close.on", "font.family_default",
            "font.family_monospace", "text.align_left", "text.align_center", "text.align_right",
            "text.align_top", "text.align_bottom", "text.format_none", "text.format_bold",
            "text.format_italic", "text.wrap_none", "text.wrap_auto",
        ]
        for style in [
            "line", "linebr", "stepline", "stepline_diamond", "steplinebr", "histogram", "columns",
            "circles", "cross", "area", "areabr",
        ] {
            names.insert("plot.style_\(style)")
        }
        for style in ["solid", "dotted", "dashed"] {
            names.insert("plot.linestyle_\(style)")
            names.insert("hline.style_\(style)")
        }
        for code in [
            "AUD", "BTC", "CAD", "CHF", "CNY", "ETH", "EUR", "GBP", "HKD", "INR", "JPY", "KRW", "MYR",
            "NOK", "NONE", "NZD", "RUB", "SEK", "SGD", "TRY", "USD", "USDT", "ZAR",
        ] {
            names.insert("currency.\(code)")
        }
        return names
    }()

    private static let reservedFunctions: Set<String> = [
        "fixnan", "plotarrow", "plotbar",
    ]

    private static let reservedNamespaces: Set<String> = [
        "request", "session", "chart", "log", "runtime", "ticker", "barmerge", "xloc", "yloc", "scale", "adjustment",
        "backadjustment", "settlement_as_close", "font", "text", "currency", "plot", "hline",
    ]
}
