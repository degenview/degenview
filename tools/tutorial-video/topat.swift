import CoreGraphics
import Foundation

// topat X Y: prints "pid id" of the topmost normal window under the point, nothing when there is none.
// The recorder clicks only when that window is the app's own.
let x = CGFloat(Double(CommandLine.arguments[1])!)
let y = CGFloat(Double(CommandLine.arguments[2])!)
guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else { exit(1) }
for window in windows where (window[kCGWindowLayer as String] as? Int ?? 0) == 0 {
    let b = window[kCGWindowBounds as String] as? [String: CGFloat] ?? [:]
    let frame = CGRect(x: b["X"] ?? 0, y: b["Y"] ?? 0, width: b["Width"] ?? 0, height: b["Height"] ?? 0)
    if frame.contains(CGPoint(x: x, y: y)) {
        print(window[kCGWindowOwnerPID as String] as? Int ?? 0, window[kCGWindowNumber as String] as? Int ?? 0)
        exit(0)
    }
}
