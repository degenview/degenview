import Foundation

/// `plot`, `hline`, `plotshape`, `plotchar`, `plotcandle`, `fill`, `bgcolor`, `barcolor`.
/// Each call site owns one output that grows by one sample per bar.
extension PineRuntimeSession {
    static let visualNames: [String] = [
        "plot", "hline", "plotshape", "plotchar", "plotcandle", "bgcolor", "barcolor", "fill",
    ]

    /// Fill transparency when a `fill()` names no color.
    private static let defaultFillTransparency = 90.0
    private static let candleUpColor: UInt32 = 0x26a6_9aff
    private static let candleDownColor: UInt32 = 0xef53_50ff

    func visualCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        switch call.name {
        case "plot": return try plot(call, site, &context)
        case "hline": return try hline(call, site, &context)
        case "plotshape", "plotchar": return try plotMarker(call, site, &context)
        case "fill": return try fill(call, site, &context)
        case "plotcandle": return try plotCandle(call, site, &context)
        default: return try colorSeries(call, site, &context)
        }
    }

    private func display(_ value: PineRuntimeValue?) -> PineDisplay {
        value.intValue.map(PineDisplay.init(rawValue:)) ?? .all
    }

    private func plot(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call, ["series", "title", "color", "linewidth", "style", "trackprice", "histbase"],
            &context)
        let color = b["color"].colorValue(fallback: Self.defaultColor) ?? 0
        var output =
            working.plots[site]
            ?? PinePlotOutput(
                id: site, title: b["title"].textValue, values: [], color: color,
                lineWidth: b["linewidth"].intValue ?? 1, style: Self.plotStyle(b["style"].textValue))
        output.values.append(b["series"]?.number)
        output.colors.append(color)
        output.display = display(b["display"])
        output.histBase = b["histbase"]?.number ?? 0
        working.plots[site] = output
        return .ref(.plot, site)
    }

    private func hline(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(call, ["price", "title", "color"], &context)
        if let price = b["price"]?.number {
            working.hlines[site] = .init(
                id: site, value: price, color: b["color"].colorValue(fallback: Self.defaultColor) ?? 0,
                title: b["title"].textValue)
        }
        return .void
    }

    private func plotMarker(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let isShape = call.name == "plotshape"
        let b = try bind(
            call, ["series", "title", isShape ? "style" : "char", "location", "color"], &context)
        let color = b["color"].colorValue(fallback: Self.defaultColor) ?? 0
        var marker =
            working.markers[site]
            ?? PineMarkerOutput(
                id: site, kind: isShape ? .shape : .character, values: [],
                character: isShape ? nil : b["char"].textValue, color: color,
                location: .parse(b["location"].textValue, absent: .abovebar),
                style: .parse(b["style"].textValue, absent: .triangleup),
                size: .parse(b["size"].textValue, absent: .auto))
        let sample = b["series"] ?? .na
        marker.values.append(sample.bool ?? (sample.number != nil))
        marker.prices.append(sample.number)
        marker.colors.append(color)
        marker.display = display(b["display"])
        working.markers[site] = marker
        return .void
    }

    /// `fill(p1, p2, top_value, bottom_value, top_color, bottom_color)` blends two colors by
    /// price; `fill(p1, p2, color)` is flat. Fills between hlines are not supported.
    private func fill(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let isGradient =
            call.arguments.contains { $0.name == "top_value" }
            || call.arguments.filter { $0.name == nil }.count >= 5
        let b = try bind(
            call,
            isGradient
                ? ["plot1", "plot2", "top_value", "bottom_value", "top_color", "bottom_color", "title"]
                : ["plot1", "plot2", "color", "title"], &context)
        guard case .ref(.plot, let first)? = b["plot1"], case .ref(.plot, let second)? = b["plot2"] else {
            return .void
        }
        var output = working.fills[site] ?? .init(id: site, plotA: first, plotB: second, colors: [])
        if isGradient {
            output.colors.append(nil)
            output.gradients.append(gradient(b))
        } else {
            let fallback = PineBuiltins.withTransparency(Self.defaultColor, Self.defaultFillTransparency)
            output.colors.append(b["color"].colorValue(fallback: fallback))
        }
        working.fills[site] = output
        return .void
    }

    private func gradient(_ b: [String: PineRuntimeValue]) -> PineFillGradient? {
        guard let top = b["top_value"]?.number, let bottom = b["bottom_value"]?.number,
            let topColor = b["top_color"].colorValue(fallback: nil),
            let bottomColor = b["bottom_color"].colorValue(fallback: nil)
        else { return nil }
        return .init(top: top, bottom: bottom, topColor: topColor, bottomColor: bottomColor)
    }

    private func plotCandle(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(
            call,
            [
                "open", "high", "low", "close", "title", "color", "wickcolor", "editable", "show_last",
                "bordercolor",
            ], &context)
        var output = working.candles[site] ?? .init(id: site, title: b["title"].textValue, bars: [])
        output.display = display(b["display"])
        if let open = b["open"]?.number, let high = b["high"]?.number, let low = b["low"]?.number,
            let close = b["close"]?.number
        {
            // An omitted color takes the up/down default; an explicit `na` hides that part.
            let body = b["color"].colorValue(
                fallback: close >= open ? Self.candleUpColor : Self.candleDownColor)
            output.bars.append(
                .init(
                    open: open, high: high, low: low, close: close, color: body,
                    wickColor: b["wickcolor"].colorValue(fallback: body),
                    borderColor: b["bordercolor"].colorValue(fallback: body)))
        } else {
            output.bars.append(nil)
        }
        working.candles[site] = output
        return .void
    }

    /// `bgcolor()` and `barcolor()`: one optional color per bar.
    private func colorSeries(
        _ call: PineCall, _ site: Int, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let b = try bind(call, ["color"], &context)
        let isBackground = call.name == "bgcolor"
        var output =
            (isBackground ? working.backgrounds[site] : working.barColors[site])
            ?? .init(id: site, colors: [])
        output.colors.append(Optional(b["color"] ?? .na).colorValue(fallback: nil))
        if isBackground { working.backgrounds[site] = output } else { working.barColors[site] = output }
        return .void
    }

    static func plotStyle(_ name: String?) -> PinePlotStyle {
        switch name {
        case "plot.style_stepline", "plot.style_steplinebr": .stepline
        case "plot.style_histogram": .histogram
        case "plot.style_columns": .columns
        case "plot.style_area", "plot.style_areabr": .area
        case "plot.style_circles": .circles
        case "plot.style_cross": .cross
        default: .line
        }
    }
}
