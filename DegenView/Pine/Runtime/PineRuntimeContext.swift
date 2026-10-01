import Foundation

/// How a block of statements ended.
enum PineFlow { case normal, breakLoop, continueLoop }

/// `barstate.*` for one bar.
struct PineBarFlags: Equatable, Sendable {
    var isFirst = false
    var isLast = false
    var isHistory = false
    var isRealtime = false
    var isNew = false
    var isConfirmed = false
    var isLastConfirmedHistory = false

    /// Every `barstate.*` name `value(named:)` resolves.
    static let names: Set<String> = [
        "barstate.isfirst", "barstate.islast", "barstate.ishistory", "barstate.isrealtime",
        "barstate.isnew", "barstate.isconfirmed", "barstate.islastconfirmedhistory",
    ]

    func value(named name: String) -> Bool? {
        switch name {
        case "barstate.isfirst": isFirst
        case "barstate.islast": isLast
        case "barstate.ishistory": isHistory
        case "barstate.isrealtime": isRealtime
        case "barstate.isnew": isNew
        case "barstate.isconfirmed": isConfirmed
        case "barstate.islastconfirmedhistory": isLastConfirmedHistory
        default: nil
        }
    }
}

/// Per-bar, per-call evaluation context.
struct PineRuntimeContext {
    var bar: KlineData
    var flags: PineBarFlags
    /// Names declared `varip` in this bar's run, which persist across realtime ticks.
    var intrabarNames: Set<String> = []
    /// Non-zero inside a user function: the caller's call-site key, so builtins
    /// called in the body keep separate histories per call site.
    var sitePrefix = 0
    var depth = 0

    init(bar: KlineData, flags: PineBarFlags) {
        self.bar = bar
        self.flags = flags
    }

    mutating func record(_ mode: PineDeclarationMode, for name: String) {
        if mode == .intrabar { intrabarNames.insert(name) } else { intrabarNames.remove(name) }
    }
}
