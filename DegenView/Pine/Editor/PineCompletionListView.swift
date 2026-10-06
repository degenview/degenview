import AppKit

/// The completion popup's content: a compact list over a vibrant background, and below it the
/// selected row's signature and one line of documentation. It only presents what it is given.
final class PineCompletionListView: NSView, NSTableViewDataSource, NSTableViewDelegate {
    static let rowHeight: CGFloat = 22
    static let maximumVisibleRows = 8
    static let width: CGFloat = 420
    private static let footerHeight: CGFloat = 36

    private let background = NSVisualEffectView()
    private let scrollView = NSScrollView()
    private let table = PineCompletionTableView()
    private let footer = NSTextField(wrappingLabelWithString: "")
    private let separator = NSBox()

    private(set) var items: [PineCompletionItem] = []
    private(set) var selectedIndex = 0
    private var prefix = ""

    /// A click on a row: the row and the click count (one selects, two accepts).
    var onClick: ((Int, Int) -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        background.material = .popover
        background.blendingMode = .behindWindow
        background.state = .active
        background.wantsLayer = true
        background.layer?.cornerRadius = 6
        background.layer?.masksToBounds = true

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("completion"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = Self.rowHeight
        table.intercellSpacing = .zero
        table.backgroundColor = .clear
        table.style = .plain
        table.selectionHighlightStyle = .regular
        table.allowsEmptySelection = true
        table.dataSource = self
        table.delegate = self
        table.onClick = { [weak self] row, count in
            self?.select(row)
            self?.onClick?(row, count)
        }

        scrollView.documentView = table
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.borderType = .noBorder

        footer.font = .systemFont(ofSize: 11)
        footer.textColor = .secondaryLabelColor
        footer.maximumNumberOfLines = 3
        footer.lineBreakMode = .byTruncatingTail
        separator.boxType = .separator

        addSubview(background)
        for view in [scrollView, separator, footer] { background.addSubview(view) }
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    // MARK: - Content

    func show(_ items: [PineCompletionItem], prefix: String, selected: Int) {
        self.items = items
        self.prefix = prefix
        table.reloadData()
        select(selected)
        needsLayout = true
    }

    func select(_ index: Int) {
        guard !items.isEmpty else { return }
        selectedIndex = min(max(0, index), items.count - 1)
        table.selectRowIndexes(IndexSet(integer: selectedIndex), byExtendingSelection: false)
        table.scrollRowToVisible(selectedIndex)
        updateFooter()
        needsLayout = true
    }

    /// The footer shows what the row does not: its documentation, and where it comes from. The
    /// signature is in the row itself.
    private func updateFooter() {
        guard items.indices.contains(selectedIndex) else { return }
        let item = items[selectedIndex]
        var lines: [String] = []
        if let documentation = item.documentation { lines.append(documentation) }
        lines.append(Self.origin(of: item))
        footer.stringValue = lines.joined(separator: "\n")
    }

    private static func origin(of item: PineCompletionItem) -> String {
        switch item.origin {
        case .builtin: "built-in \(item.kind.displayName)"
        case .user: "your \(item.kind.displayName)"
        case .library(let alias): "\(item.kind.displayName) from \(alias)"
        }
    }

    // MARK: - Size

    var preferredHeight: CGFloat {
        let rows = min(max(items.count, 1), Self.maximumVisibleRows)
        return CGFloat(rows) * Self.rowHeight + Self.footerHeight + 8
    }

    override func layout() {
        super.layout()
        background.frame = bounds
        let footerTop = Self.footerHeight + 4
        scrollView.frame = NSRect(
            x: 0, y: footerTop, width: bounds.width, height: max(0, bounds.height - footerTop - 4))
        separator.frame = NSRect(x: 8, y: footerTop - 1, width: bounds.width - 16, height: 1)
        footer.frame = NSRect(x: 10, y: 2, width: bounds.width - 20, height: Self.footerHeight - 2)
        table.tableColumns.first?.width = scrollView.contentSize.width
    }

    // MARK: - Table

    func numberOfRows(in tableView: NSTableView) -> Int { items.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("row")
        let reused = tableView.makeView(withIdentifier: identifier, owner: self) as? PineCompletionRowView
        let view = reused ?? PineCompletionRowView()
        view.identifier = identifier
        view.configure(with: items[row], prefix: prefix)
        return view
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        PineCompletionTableRowView()
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { true }
}
