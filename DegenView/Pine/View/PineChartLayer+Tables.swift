import SwiftUI

/// `table.new` grids, sized to their cell text and pinned to a plot corner/edge. Tables are
/// never value-anchored, so callers draw them outside the series clip.
extension PineChartLayer {
    private static let tableMargin: CGFloat = 8
    private static let cellPadding = CGSize(width: 12, height: 6)

    /// A table's cells measured, with the column widths and row heights they imply.
    private struct TableLayout {
        var widths: [CGFloat]
        var heights: [CGFloat]
        var cells: [(cell: PineTableCell, text: GraphicsContext.ResolvedText)]

        var size: CGSize { CGSize(width: widths.reduce(0, +), height: heights.reduce(0, +)) }

        /// Where `cell` sits, covering every column and row it spans.
        func rect(of cell: PineTableCell, origin: CGPoint) -> CGRect {
            let lastColumn = min(cell.column + cell.columnSpan, widths.count)
            let lastRow = min(cell.row + cell.rowSpan, heights.count)
            return CGRect(
                x: origin.x + widths[..<cell.column].reduce(0, +),
                y: origin.y + heights[..<cell.row].reduce(0, +),
                width: widths[cell.column..<lastColumn].reduce(0, +),
                height: heights[cell.row..<lastRow].reduce(0, +))
        }
    }

    func drawTables(_ context: inout GraphicsContext, plot: ChartPlot) {
        for table in pine.tables where table.columns > 0 && table.rows > 0 && !table.cells.isEmpty {
            let layout = measure(table, &context)
            let area = plot.plotRect.insetBy(dx: Self.tableMargin, dy: Self.tableMargin)
            let origin = origin(of: layout.size, at: table.position, in: area)
            draw(table, layout, at: origin, &context)
        }
    }

    private func measure(_ table: PineTableOutput, _ context: inout GraphicsContext) -> TableLayout {
        var layout = TableLayout(
            widths: Array(repeating: 0, count: table.columns),
            heights: Array(repeating: 0, count: table.rows), cells: [])
        for cell in table.cells {
            let text = context.resolve(
                Text(cell.text)
                    .font(.system(size: cell.textSize.fontSize))
                    .foregroundColor(Color(pineRGBA: cell.textColor)))
            let size = text.measure(in: CGSize(width: 400, height: 200))
            let width = size.width + Self.cellPadding.width
            let height = size.height + Self.cellPadding.height
            if cell.columnSpan == 1 { layout.widths[cell.column] = max(layout.widths[cell.column], width) }
            if cell.rowSpan == 1 { layout.heights[cell.row] = max(layout.heights[cell.row], height) }
            layout.cells.append((cell, text))
        }
        // A merged cell only widens the grid when its text does not fit the columns it spans.
        for (cell, text) in layout.cells where cell.columnSpan > 1 || cell.rowSpan > 1 {
            let size = text.measure(in: CGSize(width: 400, height: 200))
            let last = min(cell.column + cell.columnSpan, table.columns) - 1
            let lastRow = min(cell.row + cell.rowSpan, table.rows) - 1
            let spanWidth = layout.widths[cell.column...last].reduce(0, +)
            let spanHeight = layout.heights[cell.row...lastRow].reduce(0, +)
            layout.widths[last] += max(0, size.width + Self.cellPadding.width - spanWidth)
            layout.heights[lastRow] += max(0, size.height + Self.cellPadding.height - spanHeight)
        }
        return layout
    }

    private func origin(of size: CGSize, at position: PineTablePosition, in area: CGRect) -> CGPoint {
        let x: CGFloat =
            switch position.horizontal {
            case .left: area.minX
            case .center: area.midX - size.width / 2
            case .right: area.maxX - size.width
            }
        let y: CGFloat =
            switch position.vertical {
            case .top: area.minY
            case .middle: area.midY - size.height / 2
            case .bottom: area.maxY - size.height
            }
        return CGPoint(x: x, y: y)
    }

    private func draw(
        _ table: PineTableOutput, _ layout: TableLayout, at origin: CGPoint,
        _ context: inout GraphicsContext
    ) {
        let frame = CGRect(origin: origin, size: layout.size)
        if let background = table.backgroundColor {
            context.fill(Path(frame), with: .color(Color(pineRGBA: background)))
        }
        for (cell, text) in layout.cells {
            let rect = layout.rect(of: cell, origin: origin)
            if let background = cell.backgroundColor {
                context.fill(Path(rect), with: .color(Color(pineRGBA: background)))
            }
            context.draw(text, at: CGPoint(x: rect.midX, y: rect.midY))
        }
        if let border = table.borderColor, table.borderWidth > 0 {
            context.stroke(
                gridLines(layout, in: frame), with: .color(Color(pineRGBA: border)),
                lineWidth: CGFloat(table.borderWidth))
        }
        if let frameColor = table.frameColor, table.frameWidth > 0 {
            context.stroke(
                Path(frame), with: .color(Color(pineRGBA: frameColor)), lineWidth: CGFloat(table.frameWidth))
        }
    }

    /// The internal separators between columns and rows, minus the stretches that run through a
    /// merged cell.
    private func gridLines(_ layout: TableLayout, in frame: CGRect) -> Path {
        var grid = Path()
        let merged = layout.cells.map(\.cell).filter { $0.columnSpan > 1 || $0.rowSpan > 1 }
        func crossesColumn(_ boundary: Int, row: Int) -> Bool {
            merged.contains {
                $0.column < boundary && boundary < $0.column + $0.columnSpan
                    && $0.row <= row && row < $0.row + $0.rowSpan
            }
        }
        func crossesRow(_ boundary: Int, column: Int) -> Bool {
            merged.contains {
                $0.row < boundary && boundary < $0.row + $0.rowSpan
                    && $0.column <= column && column < $0.column + $0.columnSpan
            }
        }
        var x = frame.minX
        for boundary in 1..<max(1, layout.widths.count) {
            x += layout.widths[boundary - 1]
            var y = frame.minY
            for row in layout.heights.indices {
                if !crossesColumn(boundary, row: row) {
                    grid.move(to: CGPoint(x: x, y: y))
                    grid.addLine(to: CGPoint(x: x, y: y + layout.heights[row]))
                }
                y += layout.heights[row]
            }
        }
        var y = frame.minY
        for boundary in 1..<max(1, layout.heights.count) {
            y += layout.heights[boundary - 1]
            var x = frame.minX
            for column in layout.widths.indices {
                if !crossesRow(boundary, column: column) {
                    grid.move(to: CGPoint(x: x, y: y))
                    grid.addLine(to: CGPoint(x: x + layout.widths[column], y: y))
                }
                x += layout.widths[column]
            }
        }
        return grid
    }
}
