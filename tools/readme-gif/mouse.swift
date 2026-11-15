// usage: mouse <pid> park|drag x1 y1 [x2 y2]
// Moves the pointer, or drags from (x1, y1) to (x2, y2). A drag is refused unless a window of <pid>
// is the topmost one at its start, so a stray click can never land in another app.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
let pid = Int(args[1])!
let (x1, y1) = (Double(args[3])!, Double(args[4])!)

func topmostPid(at x: Double, _ y: Double) -> Int {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
    for window in windows where (window[kCGWindowLayer as String] as? Int) == 0 {
        let b = window[kCGWindowBounds as String] as! [String: Any]
        let (bx, by) = (b["X"] as! Double, b["Y"] as! Double)
        if x >= bx && x < bx + (b["Width"] as! Double) && y >= by && y < by + (b["Height"] as! Double) {
            return window[kCGWindowOwnerPID as String] as! Int
        }
    }
    return -1
}

func post(_ type: CGEventType, _ x: Double, _ y: Double) {
    CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: .left)!
        .post(tap: .cghidEventTap)
}

switch args[2] {
case "park":
    post(.mouseMoved, x1, y1)
case "drag":
    guard topmostPid(at: x1, y1) == pid else {
        print("refused: pid \(pid) is not topmost at \(x1),\(y1)")
        exit(2)
    }
    let (x2, y2) = (Double(args[5])!, Double(args[6])!)
    post(.mouseMoved, x1, y1)
    usleep(150_000)
    post(.leftMouseDown, x1, y1)
    usleep(150_000)
    let steps = 24
    for step in 1...steps {
        let t = Double(step) / Double(steps)
        post(.leftMouseDragged, x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)
        usleep(25_000)
    }
    usleep(100_000)
    post(.leftMouseUp, x2, y2)
default:
    print("usage: mouse <pid> park|drag x1 y1 [x2 y2]")
    exit(1)
}
