// usage: winlist <pid>   prints "<window id> <x> <y> <width> <height>" for the pid's first on-screen window
import CoreGraphics
import Foundation

let pid = Int(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for window in windows where (window[kCGWindowOwnerPID as String] as? Int) == pid && (window[kCGWindowLayer as String] as? Int) == 0 {
    let b = window[kCGWindowBounds as String] as! [String: Any]
    print(window[kCGWindowNumber as String]!, Int(b["X"] as! Double), Int(b["Y"] as! Double), Int(b["Width"] as! Double), Int(b["Height"] as! Double))
    break
}
