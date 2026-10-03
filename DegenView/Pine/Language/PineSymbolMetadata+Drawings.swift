import Foundation

extension PineSymbolMetadata {
    /// `line.*`, `label.*`, `box.*`, `table.*`, `linefill.*`, `polyline.*` and `chart.point.*`.
    static let drawingEntries = [drawingConstructorEntries, drawingSetterEntries, tableEntries].joined(separator: "\n")

    private static let drawingConstructorEntries = """
        line.new(x1: series int, y1: series float, x2: series int, y2: series float, xloc: series string = xloc.bar_index, extend: series string = extend.none, color: series color = na, style: series string = line.style_solid, width: series int = 1) -> series line :: Draws a line between two points.
        line.new(first_point: chart.point, second_point: chart.point, xloc: series string = xloc.bar_index, extend: series string = extend.none, color: series color = na, style: series string = line.style_solid, width: series int = 1) -> series line :: Draws a line between two chart points.
        label.new(x: series int, y: series float, text: series string = "", xloc: series string = xloc.bar_index, yloc: series string = yloc.price, color: series color = na, style: series string = label.style_label_down, textcolor: series color = na, size: series string = size.normal, textalign: series string = text.align_center, tooltip: series string = na) -> series label :: Draws a text label.
        label.new(point: chart.point, text: series string = "", xloc: series string = xloc.bar_index, yloc: series string = yloc.price, color: series color = na, style: series string = label.style_label_down, textcolor: series color = na, size: series string = size.normal, textalign: series string = text.align_center, tooltip: series string = na) -> series label :: Draws a text label at a chart point.
        box.new(left: series int, top: series float, right: series int, bottom: series float, border_color: series color = na, border_width: series int = 1, border_style: series string = line.style_solid, extend: series string = extend.none, xloc: series string = xloc.bar_index, bgcolor: series color = na, text: series string = "", text_size: series string = size.auto, text_color: series color = na, text_halign: series string = text.align_center, text_valign: series string = text.align_center) -> series box :: Draws a rectangle.
        box.new(top_left: chart.point, bottom_right: chart.point, border_color: series color = na, border_width: series int = 1, border_style: series string = line.style_solid, extend: series string = extend.none, xloc: series string = xloc.bar_index, bgcolor: series color = na, text: series string = "", text_size: series string = size.auto, text_color: series color = na, text_halign: series string = text.align_center, text_valign: series string = text.align_center) -> series box :: Draws a rectangle between two chart points.
        linefill.new(line1: series line, line2: series line, color: series color = na) -> series linefill :: Fills the space between two lines.
        polyline.new(points: array<chart.point>, curved: series bool = false, closed: series bool = false, xloc: series string = xloc.bar_index, line_color: series color = na, fill_color: series color = na, line_style: series string = line.style_solid, line_width: series int = 1) -> series polyline :: Draws a series of connected points.
        polyline.delete(id: series polyline) -> void :: Deletes a polyline.
        chart.point.new(time: series int, index: series int, price: series float) -> chart.point :: A chart point from a time, a bar index and a price.
        chart.point.from_index(index: series int, price: series float) -> chart.point :: A chart point at a bar index.
        chart.point.from_time(time: series int, price: series float) -> chart.point :: A chart point at a time.
        chart.point.now(price: series float = na) -> chart.point :: A chart point at the current bar.
        """

    private static let drawingSetterEntries = """
        line.delete(id: series line) -> void :: Deletes a line.
        line.set_x1(id: series line, x: series int) -> void :: Sets the first x coordinate.
        line.set_y1(id: series line, y: series float) -> void :: Sets the first y coordinate.
        line.set_x2(id: series line, x: series int) -> void :: Sets the second x coordinate.
        line.set_y2(id: series line, y: series float) -> void :: Sets the second y coordinate.
        line.set_xy1(id: series line, x: series int, y: series float) -> void :: Moves the first point.
        line.set_xy2(id: series line, x: series int, y: series float) -> void :: Moves the second point.
        line.set_first_point(id: series line, point: chart.point) -> void :: Moves the first point to a chart point.
        line.set_second_point(id: series line, point: chart.point) -> void :: Moves the second point to a chart point.
        line.set_color(id: series line, color: series color) -> void :: Sets the line color.
        line.set_width(id: series line, width: series int) -> void :: Sets the line width.
        line.set_style(id: series line, style: series string) -> void :: Sets the line style.
        line.set_extend(id: series line, extend: series string) -> void :: Sets how the line extends.
        line.get_x1(id: series line) -> series int :: First x coordinate.
        line.get_y1(id: series line) -> series float :: First y coordinate.
        line.get_x2(id: series line) -> series int :: Second x coordinate.
        line.get_y2(id: series line) -> series float :: Second y coordinate.
        label.delete(id: series label) -> void :: Deletes a label.
        label.set_x(id: series label, x: series int) -> void :: Sets the x coordinate.
        label.set_y(id: series label, y: series float) -> void :: Sets the y coordinate.
        label.set_xy(id: series label, x: series int, y: series float) -> void :: Moves the label.
        label.set_point(id: series label, point: chart.point) -> void :: Moves the label to a chart point.
        label.set_text(id: series label, text: series string) -> void :: Sets the label text.
        label.set_color(id: series label, color: series color) -> void :: Sets the label color.
        label.set_textcolor(id: series label, textcolor: series color) -> void :: Sets the text color.
        label.set_style(id: series label, style: series string) -> void :: Sets the label style.
        label.set_size(id: series label, size: series string) -> void :: Sets the text size.
        label.set_tooltip(id: series label, tooltip: series string) -> void :: Sets the tooltip.
        label.set_textalign(id: series label, textalign: series string) -> void :: Sets the text alignment.
        label.get_x(id: series label) -> series int :: The x coordinate.
        label.get_y(id: series label) -> series float :: The y coordinate.
        label.get_text(id: series label) -> series string :: The label text.
        box.delete(id: series box) -> void :: Deletes a box.
        box.set_left(id: series box, left: series int) -> void :: Sets the left edge.
        box.set_right(id: series box, right: series int) -> void :: Sets the right edge.
        box.set_top(id: series box, top: series float) -> void :: Sets the top edge.
        box.set_bottom(id: series box, bottom: series float) -> void :: Sets the bottom edge.
        box.set_lefttop(id: series box, left: series int, top: series float) -> void :: Moves the top-left corner.
        box.set_rightbottom(id: series box, right: series int, bottom: series float) -> void :: Moves the bottom-right corner.
        box.set_top_left_point(id: series box, point: chart.point) -> void :: Moves the top-left corner to a chart point.
        box.set_bottom_right_point(id: series box, point: chart.point) -> void :: Moves the bottom-right corner to a chart point.
        box.set_bgcolor(id: series box, color: series color) -> void :: Sets the fill color.
        box.set_border_color(id: series box, color: series color) -> void :: Sets the border color.
        box.set_border_width(id: series box, width: series int) -> void :: Sets the border width.
        box.set_border_style(id: series box, style: series string) -> void :: Sets the border style.
        box.set_text(id: series box, text: series string) -> void :: Sets the box text.
        box.set_text_color(id: series box, text_color: series color) -> void :: Sets the text color.
        box.set_text_size(id: series box, text_size: series string) -> void :: Sets the text size.
        box.set_text_halign(id: series box, text_halign: series string) -> void :: Sets the horizontal text alignment.
        box.set_text_valign(id: series box, text_valign: series string) -> void :: Sets the vertical text alignment.
        box.get_left(id: series box) -> series int :: The left edge.
        box.get_right(id: series box) -> series int :: The right edge.
        box.get_top(id: series box) -> series float :: The top edge.
        box.get_bottom(id: series box) -> series float :: The bottom edge.
        linefill.delete(id: series linefill) -> void :: Deletes a line fill.
        linefill.set_color(id: series linefill, color: series color) -> void :: Sets the fill color.
        linefill.get_line1(id: series linefill) -> series line :: The first line.
        linefill.get_line2(id: series linefill) -> series line :: The second line.
        """

    private static let tableEntries = """
        table.new(position: series string, columns: series int, rows: series int, bgcolor: series color = na, frame_color: series color = na, frame_width: series int = 0, border_color: series color = na, border_width: series int = 0) -> series table :: Creates a table.
        table.cell(table_id: series table, column: series int, row: series int, text: series string = "", width: series float = 0, height: series float = 0, text_color: series color = na, text_halign: series string = text.align_center, text_valign: series string = text.align_center, text_size: series string = size.normal, bgcolor: series color = na) -> void :: Sets a table cell.
        table.merge_cells(table_id: series table, start_column: series int, start_row: series int, end_column: series int, end_row: series int) -> void :: Merges a block of cells.
        table.clear(table_id: series table, start_column: series int = 0, start_row: series int = 0, end_column: series int = na, end_row: series int = na) -> void :: Clears cells.
        table.delete(table_id: series table) -> void :: Deletes a table.
        table.set_position(table_id: series table, position: series string) -> void :: Moves the table.
        table.set_bgcolor(table_id: series table, bgcolor: series color) -> void :: Sets the table background.
        table.set_border_color(table_id: series table, border_color: series color) -> void :: Sets the border color.
        table.set_frame_color(table_id: series table, frame_color: series color) -> void :: Sets the frame color.
        table.set_border_width(table_id: series table, border_width: series int) -> void :: Sets the border width.
        table.set_frame_width(table_id: series table, frame_width: series int) -> void :: Sets the frame width.
        """
}
