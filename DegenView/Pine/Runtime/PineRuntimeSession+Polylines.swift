import Foundation

/// `chart.point` and `polyline`. A point is an ordinary object (type name `chart.point`, fields `index`,
/// `time`, `price`), so field reads and assignment, arrays of points and `copy()` need nothing more.
extension PineRuntimeSession {
    private static let chartPointType = "chart.point"

    func chartPointCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let parameters: [String]
        switch call.name {
        case "chart.point.from_index": parameters = ["index", "price"]
        case "chart.point.from_time": parameters = ["time", "price"]
        case "chart.point.now": parameters = ["price"]
        default: parameters = ["time", "index", "price"]
        }
        let b = try bind(call, parameters, &context)
        var index = b["index"]
        var time = b["time"]
        if call.name == "chart.point.now" {
            index = .int(working.barIndex)
            time = .int(PineTime.milliseconds(context.bar.openTime))
        } else if let milliseconds = time.intValue, index == nil || index == .na {
            index = .int(barIndex(forTime: milliseconds, at: context.bar))
        }
        let price = b["price"] ?? (call.name == "chart.point.now" ? .float(context.bar.closePrice) : .na)
        return newInstance(
            PineObject(
                typeName: Self.chartPointType,
                fields: ["index": index ?? .na, "time": time ?? .na, "price": price]))
    }

    func polylineCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        switch call.name {
        case "polyline.new": return try newPolyline(call, &context)
        case "polyline.delete":
            let values = try allArguments(call, &context)
            if case .ref(.polyline, let id)? = values.first { working.polylines[id] = nil }
            return .void
        default: throw call.unknownFunction
        }
    }

    private func newPolyline(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "points", "curved", "closed", "xloc", "line_color", "fill_color", "line_style", "line_width",
            ], &context)
        guard case .ref(.array, let arrayID)? = b["points"], let items = working.arrays[arrayID] else {
            return .na
        }
        let timed = b["xloc"].textValue == "xloc.bar_time"
        var points: [PinePolylineOutput.Point] = []
        for item in items {
            guard case .ref(.object, let id) = item, let object = working.instances[id],
                object.typeName == Self.chartPointType, let price = object.fields["price"]?.number
            else { continue }
            let index: Int?
            if timed, let milliseconds = object.fields["time"].intValue {
                index = barIndex(forTime: milliseconds, at: context.bar)
            } else {
                index = object.fields["index"].intValue
            }
            if let index { points.append(.init(index: index, price: price)) }
        }
        let lineColor = b["line_color"].colorValue(fallback: Self.defaultColor)
        let fillColor = b["fill_color"].colorValue(fallback: nil)
        let closed = b["closed"]?.bool ?? false
        return store(\.polylines, kind: .polyline, limit: program.declaration.maxPolylinesCount) { id in
            PinePolylineOutput(
                id: id, points: points, closed: closed, lineColor: lineColor, fillColor: fillColor,
                style: .parse(b["line_style"].textValue, absent: .solid), width: b["line_width"].intValue ?? 1)
        }
    }
}
