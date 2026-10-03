import Foundation

extension PineSymbolMetadata {
    /// `input` and `input.*`.
    static let inputEntries = """
        input(defval: const any, title: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input any :: A setting whose type follows defval.
        input.int(defval: const int, title: const string = na, minval: const int = na, maxval: const int = na, step: const int = 1, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input int :: An integer setting.
        input.float(defval: const float, title: const string = na, minval: const float = na, maxval: const float = na, step: const float = 1, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input float :: A decimal setting.
        input.bool(defval: const bool, title: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input bool :: A checkbox setting.
        input.string(defval: const string, title: const string = na, options: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input string :: A text or dropdown setting.
        input.color(defval: const color, title: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input color :: A color setting.
        input.source(defval: series float, title: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na) -> series float :: A setting that picks a price series such as close.
        input.time(defval: const int, title: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input int :: A date and time setting.
        input.session(defval: const string, title: const string = na, options: const string = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input string :: A trading session setting.
        input.enum(defval: const any, title: const string = na, options: const any = na, tooltip: const string = na, inline: const string = na, group: const string = na, confirm: const bool = false) -> input any :: A dropdown setting over an enum.
        """
}
