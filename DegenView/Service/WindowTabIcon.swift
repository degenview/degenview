import AppKit

/// The icon a window shows at the start of its tab in the native tab bar.
///
/// The system draws the tab and offers no image slot, so the icon travels inside the tab's
/// attributed title as an SF Symbol text attachment. Dragging, reordering, merging and the
/// close button stay entirely AppKit's.
enum WindowTabIcon: CaseIterable {
    case charts
    case portfolio
    case scriptManager

    init(kind: ChartTabKind) {
        switch kind {
        case .charts: self = .charts
        case .portfolio: self = .portfolio
        }
    }

    var symbolName: String {
        switch self {
        case .charts: "chart.xyaxis.line"
        case .portfolio: "briefcase"
        case .scriptManager: "curlybraces"
        }
    }

    /// The tab label: the icon, a small gap, then `title`.
    func attributedTitle(_ title: String, fontSize: CGFloat = NSFont.systemFontSize(for: .small)) -> NSAttributedString {
        let font = NSFont.systemFont(ofSize: fontSize)
        let result = NSMutableAttributedString()
        if let image = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: fontSize, weight: .medium))
        {
            let attachment = NSTextAttachment()
            attachment.image = image
            // Sits on the text baseline; symbols are drawn slightly above it otherwise.
            attachment.bounds = CGRect(x: 0, y: -2, width: image.size.width, height: image.size.height)
            result.append(NSAttributedString(attachment: attachment))
            result.append(NSAttributedString(string: " "))
        }
        result.append(NSAttributedString(string: title))
        result.addAttribute(.font, value: font, range: NSRange(location: 0, length: result.length))
        return result
    }
}

/// Keeps one window's tab label carrying its icon.
///
/// The tab object is rebuilt when a window joins or leaves a tab group, and setting
/// `window.title` — a rename, SwiftUI's `navigationTitle` — resets its attributed title too, so
/// the icon is re-applied on a title change, on a group change, and whenever the coordinator
/// refreshes its tab bars. The decorator lives as long as its window.
final class WindowTabDecorator: NSObject {
    private static var associationKey: UInt8 = 0
    private weak var window: NSWindow?
    private let icon: WindowTabIcon
    private var observations: [NSKeyValueObservation] = []

    private init(window: NSWindow, icon: WindowTabIcon) {
        self.window = window
        self.icon = icon
        super.init()
        observations = [
            window.observe(\.title, options: [.initial]) { [weak self] _, _ in self?.apply(force: true) },
            // The tab bar builds its items a beat after the window joins a group and ignores a
            // label set before that, so the icon is written again once it has.
            window.observe(\.tabGroup, options: []) { [weak self] _, _ in
                self?.apply(force: true)
                DispatchQueue.main.async { self?.apply(force: true) }
            },
        ]
    }

    /// Starts decorating `window`, replacing any earlier icon on it.
    static func decorate(_ window: NSWindow, with icon: WindowTabIcon) {
        let decorator = WindowTabDecorator(window: window, icon: icon)
        objc_setAssociatedObject(window, &associationKey, decorator, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    }

    /// Re-applies the icon if `window` was decorated and its tab lost it.
    static func refresh(_ window: NSWindow) {
        (objc_getAssociatedObject(window, &associationKey) as? WindowTabDecorator)?.apply(force: false)
    }

    private func apply(force: Bool) {
        guard let window else { return }
        let label = icon.attributedTitle(window.title)
        // A refresh that finds the label already in place would only make the bar relayout.
        if !force, window.tab.attributedTitle?.isEqual(to: label) == true { return }
        window.tab.attributedTitle = label
    }
}
