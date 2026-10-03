import SwiftUI

private struct SavedLayoutFocusedKey: FocusedValueKey {
    typealias Value = SavedLayoutController
}

extension FocusedValues {
    /// The focused tab's saved-layout controller, so menu commands reach the right tab without
    /// a global key monitor.
    var savedLayout: SavedLayoutController? {
        get { self[SavedLayoutFocusedKey.self] }
        set { self[SavedLayoutFocusedKey.self] = newValue }
    }
}
