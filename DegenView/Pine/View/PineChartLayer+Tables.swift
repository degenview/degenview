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
            layout.widths[cell.column] = max(layout.widths[cell.column], size.width + Self.cellPadding.width)
            layout.heights[cell.row] = max(layout.heights[cell.row], size.height + Self.cellPadding.height)
            layout.cells.append((cell, text))
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
            let rect = CGRect(
                x: origin.x + layout.widths[..<cell.column].reduce(0, +),
                y: origin.y + layout.heights[..<cell.row].reduce(0, +),
                width: layout.widths[cell.column], height: layout.heights[cell.row])
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

    /// The internal separators between columns and rows.
    private func gridLines(_ layout: TableLayout, in frame: CGRect) -> Path {
        var grid = Path()
        var x = frame.minX
        for width in layout.widths.dropLast() {
            x += width
            grid.move(to: CGPoint(x: x, y: frame.minY))
            grid.addLine(to: CGPoint(x: x, y: frame.maxY))
        }
        var y = frame.minY
        for height in layout.heights.dropLast() {
            y += height
            grid.move(to: CGPoint(x: frame.minX, y: y))
            grid.addLine(to: CGPoint(x: frame.maxX, y: y))
        }
        return grid
    }
}
