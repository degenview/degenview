import CoreGraphics
import Foundation

// Lists windows whose owner name contains the argument (all windows when empty), one per line:
//   id pid owner x y width height layer onscreen title
let filter = CommandLine.arguments.count > 1 ? CommandLine.arguments[1].lowercased() : ""
guard let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] else { exit(1) }
for window in windows {
    let owner = (window[kCGWindowOwnerName as String] as? String) ?? ""
    if !filter.isEmpty && !owner.lowercased().contains(filter) { continue }
    let bounds = window[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
    let fields: [Any] = [
        window[kCGWindowNumber as String] as? Int ?? 0,
        window[kCGWindowOwnerPID as String] as? Int ?? 0,
        owner.replacingOccurrences(of: " ", with: "_"),
        Int(bounds["X"] ?? 0), Int(bounds["Y"] ?? 0), Int(bounds["Width"] ?? 0), Int(bounds["Height"] ?? 0),
        window[kCGWindowLayer as String] as? Int ?? 0,
        (window[kCGWindowIsOnscreen as String] as? Bool ?? false) ? 1 : 0,
        (window[kCGWindowName as String] as? String) ?? "",
    ]
    print(fields.map { "\($0)" }.joined(separator: " "))
}
