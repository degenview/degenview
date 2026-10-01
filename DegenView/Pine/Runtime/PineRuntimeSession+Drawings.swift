import Foundation

/// `line.*`, `label.*`, `box.*` and `table.*` objects. They live in the runtime state keyed by
/// reference id, so realtime rollback restores them with the variables that hold the handles.
extension PineRuntimeSession {
    private static let opaqueBlack: UInt32 = 0x0000_00ff

    func drawingCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        switch call.name {
        case "line.new": return try newLine(call, &context)
        case "label.new": return try newLabel(call, &context)
        case "box.new": return try newBox(call, &context)
        case "table.new": return try newTable(call, &context)
        case "linefill.new": return try newLinefill(call, &context)
        case "table.cell": return try setTableCell(call, &context)
        case "table.merge_cells": return try mergeTableCells(call, &context)
        case "table.clear": return try clearTableCells(call, &context)
        default: return try mutateDrawing(call, &context)
        }
    }

    /// Stores a new object, drops the oldest beyond the script's limit, and returns its handle.
    private func store<T>(
        _ objects: WritableKeyPath<PineRuntimeState, [Int: T]>, kind: PineRefKind, limit: Int?,
        _ make: (Int) -> T
    ) -> PineRuntimeValue {
        let id = allocate()
        working[keyPath: objects][id] = make(id)
        let cap = max(1, limit ?? Self.defaultDrawingLimit)
        while working[keyPath: objects].count > cap,
            let oldest = working[keyPath: objects].keys.min()
        {
            working[keyPath: objects][oldest] = nil
        }
        return .ref(kind, id)
    }

    private func requireBarIndex(_ b: [String: PineRuntimeValue], _ range: PineSourceRange) throws {
        if b["xloc"].textValue == "xloc.bar_time" {
            throw PineDiagnostic.error(
                "PINE9004", .unsupported, "xloc.bar_time drawings are not supported yet.", range)
        }
    }

    // MARK: - Creation

    private func newLine(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let b = try bind(
            call, ["x1", "y1", "x2", "y2", "xloc", "extend", "color", "style", "width"], &context)
        try requireBarIndex(b, call.range)
        guard let x1 = b["x1"].intValue, let y1 = b["y1"]?.number, let x2 = b["x2"].intValue,
            let y2 = b["y2"]?.number
        else { return .na }
        return store(\.lines, kind: .line, limit: program.declaration.maxLinesCount) { id in
            PineLineOutput(
                id: id, x1: x1, y1: y1, x2: x2, y2: y2,
                color: b["color"].colorValue(fallback: Self.defaultColor) ?? 0,
                width: b["width"].intValue ?? 1, style: .parse(b["style"].textValue, absent: .solid),
                extend: .parse(b["extend"].textValue, absent: .none))
        }
    }

    private func newLabel(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "x", "y", "text", "xloc", "yloc", "color", "style", "textcolor", "size", "textalign",
                "tooltip",
            ], &context)
        try requireBarIndex(b, call.range)
        guard let x = b["x"].intValue, let y = b["y"]?.number else { return .na }
        return store(\.labels, kind: .label, limit: program.declaration.maxLabelsCount) { id in
            PineLabelOutput(
                id: id, x: x, y: y, text: b["text"].textValue ?? "",
                color: b["color"].colorValue(fallback: Self.defaultColor),
                textColor: b["textcolor"].colorValue(fallback: Self.opaqueBlack) ?? 0,
                style: .parse(b["style"].textValue, absent: .labelDown, unknown: .labelCenter),
                size: .parse(b["size"].textValue, absent: .normal), tooltip: b["tooltip"].textValue)
        }
    }

    private func newBox(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "left", "top", "right", "bottom", "border_color", "border_width", "border_style",
                "extend", "xloc", "bgcolor",
            ], &context)
        try requireBarIndex(b, call.range)
        guard let left = b["left"].intValue, let top = b["top"]?.number, let right = b["right"].intValue,
            let bottom = b["bottom"]?.number
        else { return .na }
        return store(\.boxes, kind: .box, limit: program.declaration.maxBoxesCount) { id in
            PineBoxOutput(
                id: id, left: left, top: top, right: right, bottom: bottom,
                borderColor: b["border_color"].colorValue(fallback: Self.defaultColor),
                borderWidth: b["border_width"].intValue ?? 1,
                backgroundColor: b["bgcolor"].colorValue(fallback: Self.defaultColor))
        }
    }

    private func newTable(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "position", "columns", "rows", "bgcolor", "frame_color", "frame_width", "border_color",
                "border_width",
            ], &context)
        let id = allocate()
        working.tables[id] = PineTableOutput(
            id: id, position: .parse(b["position"].textValue, absent: .topRight),
            columns: max(0, b["columns"].intValue ?? 0), rows: max(0, b["rows"].intValue ?? 0),
            backgroundColor: b["bgcolor"].colorValue(fallback: nil),
            borderColor: b["border_color"].colorValue(fallback: nil),
            borderWidth: b["border_width"].intValue ?? 0,
            frameColor: b["frame_color"].colorValue(fallback: nil),
            frameWidth: b["frame_width"].intValue ?? 0)
        return .ref(.table, id)
    }

    /// `linefill.new(line1, line2, color)`. Both arguments must be live lines.
    private func newLinefill(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(call, ["line1", "line2", "color"], &context)
        guard case .ref(.line, let first)? = b["line1"], case .ref(.line, let second)? = b["line2"],
            working.lines[first] != nil, working.lines[second] != nil
        else { return .na }
        return store(\.linefills, kind: .linefill, limit: nil) { id in
            PineLinefillOutput(
                id: id, line1: first, line2: second,
                color: b["color"].colorValue(fallback: Self.defaultColor) ?? 0)
        }
    }

    private func setTableCell(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "table_id", "column", "row", "text", "width", "height", "text_color", "text_halign",
                "text_valign", "text_size", "bgcolor",
            ], &context)
        guard case .ref(.table, let id)? = b["table_id"], var table = working.tables[id] else {
            return .void
        }
        guard let column = b["column"].intValue, let row = b["row"].intValue,
            (0..<table.columns).contains(column), (0..<table.rows).contains(row)
        else {
            throw PineDiagnostic.error(
                "PINE4013", .runtime,
                "table.cell position is outside the table's \(table.columns)×\(table.rows) grid.",
                call.range)
        }
        table.cells.removeAll { $0.column == column && $0.row == row }
        table.cells.append(
            PineTableCell(
                column: column, row: row, text: b["text"].textValue ?? "",
                textColor: b["text_color"].colorValue(fallback: Self.opaqueBlack) ?? 0,
                backgroundColor: b["bgcolor"].colorValue(fallback: nil),
                textSize: .parse(b["text_size"].textValue, absent: .normal)))
        working.tables[id] = table
        return .void
    }

    /// `table.merge_cells(table, start_column, start_row, end_column, end_row)`: the start cell grows to
    /// cover the range and the cells it covers are dropped.
    private func mergeTableCells(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call, ["table_id", "start_column", "start_row", "end_column", "end_row"], &context)
        guard case .ref(.table, let id)? = b["table_id"], var table = working.tables[id] else {
            return .void
        }
        guard let startColumn = b["start_column"].intValue, let startRow = b["start_row"].intValue,
            let endColumn = b["end_column"].intValue, let endRow = b["end_row"].intValue,
            (0..<table.columns).contains(startColumn), (0..<table.rows).contains(startRow),
            (startColumn..<table.columns).contains(endColumn), (startRow..<table.rows).contains(endRow)
        else {
            throw PineDiagnostic.error(
                "PINE4013", .runtime,
                "table.merge_cells range is outside the table's \(table.columns)×\(table.rows) grid.",
                call.range)
        }
        var anchor =
            table.cells.first { $0.column == startColumn && $0.row == startRow }
            ?? PineTableCell(
                column: startColumn, row: startRow, text: "", textColor: Self.opaqueBlack,
                backgroundColor: nil, textSize: .normal)
        anchor.columnSpan = endColumn - startColumn + 1
        anchor.rowSpan = endRow - startRow + 1
        table.cells.removeAll {
            (startColumn...endColumn).contains($0.column) && (startRow...endRow).contains($0.row)
        }
        table.cells.append(anchor)
        working.tables[id] = table
        return .void
    }

    /// `table.clear(table, start_column, start_row, end_column, end_row)`: drops the cells in the
    /// range. Without a range, every cell goes.
    private func clearTableCells(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call, ["table_id", "start_column", "start_row", "end_column", "end_row"], &context)
        guard case .ref(.table, let id)? = b["table_id"], var table = working.tables[id] else {
            return .void
        }
        let columns = (b["start_column"].intValue ?? 0)...(b["end_column"].intValue ?? table.columns - 1)
        let rows = (b["start_row"].intValue ?? 0)...(b["end_row"].intValue ?? table.rows - 1)
        table.cells.removeAll { columns.contains($0.column) && rows.contains($0.row) }
        working.tables[id] = table
        return .void
    }

    // MARK: - Mutation

    /// `line.set_*`, `label.set_*`, `box.set_*`, getters and `*.delete`. Operations on an `na`
    /// or deleted handle are no-ops (getters return `na`).
    private func mutateDrawing(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let values = try allArguments(call, &context)
        let target = values.first ?? .na
        let a = values.count > 1 ? values[1] : .na
        let b = values.count > 2 ? values[2] : .na
        let member = String(call.name.split(separator: ".", maxSplits: 1).last ?? "")
        if call.name == "table.delete" {
            if case .ref(.table, let id) = target { working.tables[id] = nil }
            return .void
        }
        switch call.name.split(separator: ".").first.map(String.init) {
        case "line":
            return try withObject(\.lines, .line, target, member) { try Self.mutate(&$0, member, a, b, call) }
        case "label":
            return try withObject(\.labels, .label, target, member) {
                try Self.mutate(&$0, member, a, b, call)
            }
        case "box":
            return try withObject(\.boxes, .box, target, member) { try Self.mutate(&$0, member, a, b, call) }
        case "table":
            return try withObject(\.tables, .table, target, member) { try Self.mutate(&$0, member, a, call) }
        case "linefill":
            return try withObject(\.linefills, .linefill, target, member) {
                try Self.mutate(&$0, member, a, call)
            }
        default: throw call.unknownFunction
        }
    }

    /// Looks the handle up, handles `delete`, runs `body` on a copy and writes it back.
    private func withObject<T>(
        _ objects: WritableKeyPath<PineRuntimeState, [Int: T]>, _ kind: PineRefKind,
        _ target: PineRuntimeValue, _ member: String, _ body: (inout T) throws -> PineRuntimeValue
    ) throws -> PineRuntimeValue {
        guard case .ref(kind, let id) = target, var object = working[keyPath: objects][id] else {
            return member.hasPrefix("get_") ? .na : .void
        }
        if member == "delete" {
            working[keyPath: objects][id] = nil
            return .void
        }
        let result = try body(&object)
        working[keyPath: objects][id] = object
        return result
    }

    private static func mutate(
        _ table: inout PineTableOutput, _ member: String, _ a: PineRuntimeValue, _ call: PineCall
    ) throws -> PineRuntimeValue {
        switch member {
        case "set_position": table.position = .parse(a.textValue, absent: table.position)
        case "set_bgcolor": table.backgroundColor = Optional(a).colorValue(fallback: nil)
        case "set_border_color": table.borderColor = Optional(a).colorValue(fallback: nil)
        case "set_frame_color": table.frameColor = Optional(a).colorValue(fallback: nil)
        case "set_border_width": table.borderWidth = a.intValue ?? table.borderWidth
        case "set_frame_width": table.frameWidth = a.intValue ?? table.frameWidth
        default: throw call.unknownFunction
        }
        return .void
    }

    private static func mutate(
        _ fill: inout PineLinefillOutput, _ member: String, _ a: PineRuntimeValue, _ call: PineCall
    ) throws -> PineRuntimeValue {
        switch member {
        case "set_color":
            if case .color(let color) = a { fill.color = color }
            return .void
        case "get_line1": return .ref(.line, fill.line1)
        case "get_line2": return .ref(.line, fill.line2)
        default: throw call.unknownFunction
        }
    }

    private static func mutate(
        _ line: inout PineLineOutput, _ member: String, _ a: PineRuntimeValue, _ b: PineRuntimeValue,
        _ call: PineCall
    ) throws -> PineRuntimeValue {
        switch member {
        case "set_x1": line.x1 = a.intValue ?? line.x1
        case "set_x2": line.x2 = a.intValue ?? line.x2
        case "set_y1": line.y1 = a.number ?? line.y1
        case "set_y2": line.y2 = a.number ?? line.y2
        case "set_xy1":
            line.x1 = a.intValue ?? line.x1
            line.y1 = b.number ?? line.y1
        case "set_xy2":
            line.x2 = a.intValue ?? line.x2
            line.y2 = b.number ?? line.y2
        case "set_color": line.color = Optional(a).colorValue(fallback: nil) ?? 0
        case "set_width": line.width = a.intValue ?? line.width
        case "set_style": line.style = PineLineStyle(pineName: a.textValue) ?? line.style
        case "set_extend": line.extend = PineLineExtend(pineName: a.textValue) ?? line.extend
        case "get_x1": return .int(line.x1)
        case "get_x2": return .int(line.x2)
        case "get_y1": return .float(line.y1)
        case "get_y2": return .float(line.y2)
        default: throw call.unknownFunction
        }
        return .void
    }

    private static func mutate(
        _ label: inout PineLabelOutput, _ member: String, _ a: PineRuntimeValue, _ b: PineRuntimeValue,
        _ call: PineCall
    ) throws -> PineRuntimeValue {
        switch member {
        case "set_x": label.x = a.intValue ?? label.x
        case "set_y": label.y = a.number ?? label.y
        case "set_xy":
            label.x = a.intValue ?? label.x
            label.y = b.number ?? label.y
        case "set_text": label.text = a.textValue ?? ""
        case "set_color": label.color = Optional(a).colorValue(fallback: nil)
        case "set_textcolor": label.textColor = Optional(a).colorValue(fallback: nil) ?? 0
        case "set_style":
            label.style = a.textValue.map { PineLabelStyle(pineName: $0) ?? .labelCenter } ?? label.style
        case "set_size": label.size = PineSize(pineName: a.textValue) ?? label.size
        case "set_tooltip": label.tooltip = a.textValue
        case "get_x": return .int(label.x)
        case "get_y": return .float(label.y)
        case "get_text": return .string(label.text)
        default: throw call.unknownFunction
        }
        return .void
    }

    private static func mutate(
        _ box: inout PineBoxOutput, _ member: String, _ a: PineRuntimeValue, _ b: PineRuntimeValue,
        _ call: PineCall
    ) throws -> PineRuntimeValue {
        switch member {
        case "set_left": box.left = a.intValue ?? box.left
        case "set_right": box.right = a.intValue ?? box.right
        case "set_top": box.top = a.number ?? box.top
        case "set_bottom": box.bottom = a.number ?? box.bottom
        case "set_lefttop":
            box.left = a.intValue ?? box.left
            box.top = b.number ?? box.top
        case "set_rightbottom":
            box.right = a.intValue ?? box.right
            box.bottom = b.number ?? box.bottom
        case "set_bgcolor": box.backgroundColor = Optional(a).colorValue(fallback: nil)
        case "set_border_color": box.borderColor = Optional(a).colorValue(fallback: nil)
        case "set_border_width": box.borderWidth = a.intValue ?? box.borderWidth
        case "get_left": return .int(box.left)
        case "get_right": return .int(box.right)
        case "get_top": return .float(box.top)
        case "get_bottom": return .float(box.bottom)
        default: throw call.unknownFunction
        }
        return .void
    }
}
