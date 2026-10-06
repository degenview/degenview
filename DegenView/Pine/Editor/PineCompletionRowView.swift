import AppKit

extension PineCompletionKind {
    /// The letter in the row's square icon: quiet, native, and the same set in light and dark.
    var iconLetter: String {
        switch self {
        case .function: "f"
        case .variable: "v"
        case .constant: "c"
        case .parameter: "p"
        case .local: "l"
        case .namespace: "n"
        case .keyword: "k"
        case .type: "t"
        case .field: "d"
        case .enumMember: "e"
        case .library, .libraryMember: "m"
        case .argumentName: "a"
        }
    }

    /// What the row's kind is called, for the accessibility label and the footer.
    var displayName: String {
        switch self {
        case .function: "function"
        case .variable: "variable"
        case .constant: "constant"
        case .parameter: "parameter"
        case .local: "local"
        case .namespace: "namespace"
        case .keyword: "keyword"
        case .type: "type"
        case .field: "field"
        case .enumMember: "enum member"
        case .library: "library"
        case .libraryMember: "library member"
        case .argumentName: "argument"
        }
    }
}

/// One row of the completion list: a kind icon, the name with the typed prefix emphasised, and the
/// signature or type after it.
final class PineCompletionRowView: NSView {
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")

    private static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let boldFont = NSFont.monospacedSystemFont(ofSize: 12, weight: .heavy)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        icon.imageScaling = .scaleProportionallyDown
        icon.contentTintColor = .secondaryLabelColor
        label.lineBreakMode = .byClipping
        label.cell?.truncatesLastVisibleLine = false
        detail.font = .systemFont(ofSize: 11)
        detail.textColor = .secondaryLabelColor
        detail.lineBreakMode = .byTruncatingTail
        detail.alignment = .right
        for view in [icon, label, detail] { addSubview(view) }
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(with item: PineCompletionItem, prefix: String) {
        icon.image = NSImage(
            systemSymbolName: "\(item.kind.iconLetter).square", accessibilityDescription: item.kind.displayName)
        let text = NSMutableAttributedString(
            string: item.label, attributes: [.font: Self.font, .foregroundColor: NSColor.labelColor])
        let matched = (prefix as NSString).length
        if matched > 0, item.label.lowercased().hasPrefix(prefix.lowercased()) {
            text.addAttribute(.font, value: Self.boldFont, range: NSRange(location: 0, length: matched))
        }
        label.attributedStringValue = text
        detail.stringValue = item.detail ?? item.kind.displayName
        setAccessibilityLabel("\(item.label), \(item.kind.displayName)")
    }

    override func layout() {
        super.layout()
        let height = bounds.height
        icon.frame = NSRect(x: 6, y: (height - 16) / 2, width: 16, height: 16)
        let labelWidth = min(ceil(label.attributedStringValue.size().width) + 6, bounds.width * 0.6)
        label.frame = NSRect(x: 28, y: (height - 16) / 2, width: labelWidth, height: 16)
        let detailX = label.frame.maxX + 10
        detail.frame = NSRect(
            x: detailX, y: (height - 14) / 2, width: max(0, bounds.width - detailX - 8), height: 14)
    }
}
