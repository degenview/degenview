import Foundation

extension PineSymbolMetadata {
    /// `str.*`.
    static let stringEntries = """
        str.contains(source: series string, str: series string) -> series bool :: True when source contains str.
        str.endswith(source: series string, str: series string) -> series bool :: True when source ends with str.
        str.startswith(source: series string, str: series string) -> series bool :: True when source starts with str.
        str.format(formatString: series string, arg0: series any, arg1: series any = na) -> series string :: Fills {0}, {1}… placeholders.
        str.format_time(time: series int, format: series string = na, timezone: series string = na) -> series string :: Formats a UNIX time.
        str.length(string: series string) -> series int :: Number of characters.
        str.lower(source: series string) -> series string :: Lowercase copy.
        str.upper(source: series string) -> series string :: Uppercase copy.
        str.match(source: series string, regex: series string) -> series string :: First match of a regular expression, or an empty string.
        str.repeat(source: series string, repeat: series int, separator: series string = na) -> series string :: Repeats a string.
        str.replace_all(source: series string, target: series string, replacement: series string) -> series string :: Replaces every occurrence.
        str.split(string: series string, separator: series string) -> array<string> :: Splits into an array.
        str.substring(source: series string, begin_pos: series int, end_pos: series int = na) -> series string :: Part of a string.
        str.tonumber(string: series string) -> series float :: Parses a number, or na.
        str.tostring(value: series float, format: series string = na) -> series string :: Converts a value to text.
        str.trim(source: series string) -> series string :: Removes surrounding whitespace.
        """
}
