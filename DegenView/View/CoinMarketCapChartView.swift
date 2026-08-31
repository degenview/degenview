import SwiftUI

struct CoinMarketCapChartView: View {
    @ObservedObject var viewModel: ChartViewModel
    let chartHeight: CGFloat
    let cardHeight: CGFloat?
    let onRemove: () -> Void
    let onChanged: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            header
            if viewModel.isFetching && !hasData {
                skeleton
            } else if let error = viewModel.errorMessage, !hasData {
                errorState(error)
            } else {
                content
            }
        }
        .padding(10)
        .frame(height: cardHeight ?? chartHeight + ChartLayout.cardChrome)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .accessibilityElement(children: .contain)
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(viewModel.coinMarketCapChart?.type.title ?? "CoinMarketCap").font(.headline).lineLimit(1)
                Text("Data: CoinMarketCap").font(.caption2).foregroundStyle(.secondary)
            }
            Spacer()
            if let date = viewModel.lastUpdated {
                Text(date, style: .relative).font(.caption2).foregroundStyle(.secondary)
            }
            Button {
                Task { await viewModel.fetchCoinMarketCap(force: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain).help("Refresh")
            Menu {
                settingsMenu
            } label: {
                Image(systemName: "slider.horizontal.3")
            }.menuStyle(.borderlessButton)
            Button(role: .destructive, action: onRemove) { Image(systemName: "xmark") }.buttonStyle(.plain)
        }
    }

    @ViewBuilder private var settingsMenu: some View {
        if let config = viewModel.coinMarketCapChart {
            if config.type == .altcoinSeasonHistorical {
                Picker(
                    "Range",
                    selection: Binding(
                        get: { config.altcoinRange },
                        set: { value in
                            viewModel.updateCMCConfig { $0.altcoinRange = value }
                            onChanged()
                        })
                ) { ForEach(CMCAltcoinRange.allCases) { Text($0.label).tag($0) } }
            } else if config.type == .fearAndGreedHistorical {
                Picker(
                    "Range",
                    selection: Binding(
                        get: { config.fearGreedRange },
                        set: { value in
                            viewModel.updateCMCConfig { $0.fearGreedRange = value }
                            onChanged()
                        })
                ) { ForEach(CMCFearGreedRange.allCases) { Text($0.rawValue).tag($0) } }
            }
            if config.type.isHistorical {
                Toggle(
                    "Show area fill",
                    isOn: Binding(
                        get: { config.showAreaFill },
                        set: { value in
                            viewModel.updateCMCConfig { $0.showAreaFill = value }
                            onChanged()
                        }))
                Toggle(
                    "Show threshold zones",
                    isOn: Binding(
                        get: { config.showThresholdZones },
                        set: { value in
                            viewModel.updateCMCConfig { $0.showThresholdZones = value }
                            onChanged()
                        }))
            } else {
                Toggle(
                    "Show supporting statistics",
                    isOn: Binding(
                        get: { config.showSupportingStatistics },
                        set: { value in
                            viewModel.updateCMCConfig { $0.showSupportingStatistics = value }
                            onChanged()
                        }))
            }
            Divider()
            Text(CoinMarketCapCredentialStore.isConfigured ? "Authentication: API Key" : "Authentication: Public API")
        }
    }

    @ViewBuilder private var content: some View {
        switch viewModel.coinMarketCapChart?.type {
        case .altcoinSeasonHistorical: metricHistory(altcoinPoints, title: "CMC Altcoin Season Index")
        case .fearAndGreedHistorical: metricHistory(fearPoints, title: "CMC Crypto Fear and Greed Index")
        case .altcoinSeasonLatest: altcoinLatest
        case .fearAndGreedLatest: fearGreedLatest
        case nil: EmptyView()
        }
    }

    private var altcoinPoints: [MetricPoint] {
        viewModel.cmcAltcoinHistory.compactMap {
            guard let date = CMCDateParser.parse($0.timestamp) else { return nil }
            let label = $0.altcoinIndex <= 25 ? "Bitcoin Season" : $0.altcoinIndex >= 75 ? "Altcoin Season" : "Neutral"
            return MetricPoint(
                date: date, value: $0.altcoinIndex, classification: label, marketCap: $0.altcoinMarketcap)
        }
    }
    private var fearPoints: [MetricPoint] {
        viewModel.cmcFearGreedHistory.compactMap {
            guard let date = CMCDateParser.parse($0.timestamp) else { return nil }
            return MetricPoint(date: date, value: $0.value, classification: $0.valueClassification, marketCap: nil)
        }
    }

    private func metricHistory(_ points: [MetricPoint], title: String) -> some View {
        VStack(spacing: 5) {
            HStack(alignment: .lastTextBaseline) {
                Text(points.last.map { String(Int($0.value)) } ?? "—").font(
                    .system(size: 30, weight: .bold, design: .rounded))
                Text("/ 100").foregroundStyle(.secondary)
                Text(points.last?.classification ?? "").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                Spacer()
                rangePicker
            }
            MetricHistoryPlot(
                points: points, showArea: viewModel.coinMarketCapChart?.showAreaFill ?? true,
                showZones: viewModel.coinMarketCapChart?.showThresholdZones ?? true
            )
            .accessibilityLabel(
                "\(title), historical index from 0 to 100. Latest \(Int(points.last?.value ?? 0)), \(points.last?.classification ?? "unavailable")."
            )
        }
    }

    @ViewBuilder private var rangePicker: some View {
        if let config = viewModel.coinMarketCapChart, config.type == .altcoinSeasonHistorical {
            Picker(
                "",
                selection: Binding(
                    get: { config.altcoinRange },
                    set: { value in
                        viewModel.updateCMCConfig { $0.altcoinRange = value }
                        onChanged()
                    })
            ) {
                ForEach(CMCAltcoinRange.allCases) { Text($0.label).tag($0) }
            }.pickerStyle(.segmented).frame(width: 150)
        } else if let config = viewModel.coinMarketCapChart {
            Picker(
                "",
                selection: Binding(
                    get: { config.fearGreedRange },
                    set: { value in
                        viewModel.updateCMCConfig { $0.fearGreedRange = value }
                        onChanged()
                    })
            ) {
                ForEach(CMCFearGreedRange.allCases) { Text($0.rawValue).tag($0) }
            }.pickerStyle(.segmented).frame(width: 230)
        }
    }

    private var altcoinLatest: some View {
        Group {
            if let value = viewModel.cmcAltcoinLatest {
                GeometryReader { geometry in
                    let compact = geometry.size.height < 190
                    let veryCompact = geometry.size.height < 145
                    VStack(spacing: veryCompact ? 2 : compact ? 4 : 7) {
                        Text("\(Int(value.altcoinIndex))")
                            .font(.system(size: veryCompact ? 25 : compact ? 29 : 34, weight: .bold, design: .rounded))
                            + Text(" / 100")
                            .font(.system(size: veryCompact ? 11 : 13, weight: .regular))
                            .foregroundColor(.secondary)
                        Text(value.classification)
                            .font(.system(size: veryCompact ? 12 : compact ? 14 : 16, weight: .semibold))
                            .lineLimit(1)
                            .minimumScaleFactor(0.8)
                        SeasonScale(value: value.altcoinIndex, compact: compact)
                        if viewModel.coinMarketCapChart?.showSupportingStatistics ?? true {
                            HStack(spacing: veryCompact ? 3 : 8) {
                                statistic(
                                    "Yearly High", value: "\(Int(value.yearlyHigh))",
                                    detail: shortDate(value.yearlyHighDate), compact: compact)
                                Divider()
                                statistic(
                                    "Yearly Low", value: "\(Int(value.yearlyLow))",
                                    detail: shortDate(value.yearlyLowDate), compact: compact)
                                Divider()
                                statistic(
                                    "Altcoin Market Cap",
                                    value: value.altcoinMarketcap.map { PriceFormatter.compact($0) } ?? "—",
                                    detail: nil, compact: compact)
                            }
                            .frame(maxHeight: compact ? 44 : 52)
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                }
                .accessibilityElement(children: .ignore).accessibilityLabel(
                    "CMC Altcoin Season Index, \(Int(value.altcoinIndex)) out of 100, \(value.classification).")
            }
        }
    }

    private var fearGreedLatest: some View {
        Group {
            if let value = viewModel.cmcFearGreedLatest {
                GeometryReader { geometry in
                    let compact = geometry.size.height < 170
                    let valueWidth: CGFloat = compact ? 92 : 112
                    let gap: CGFloat = compact ? 8 : 14
                    let gaugeWidth = max(80, (geometry.size.width - valueWidth - gap - 16) * 0.88)
                    HStack(alignment: .center, spacing: compact ? 8 : 14) {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(alignment: .lastTextBaseline, spacing: 3) {
                                Text("\(Int(value.value))")
                                    .font(.system(size: compact ? 35 : 44, weight: .bold, design: .rounded))
                                Text("/ 100")
                                    .font(.system(size: compact ? 11 : 13))
                                    .foregroundStyle(.secondary)
                            }
                            Text(value.valueClassification)
                                .font(.system(size: compact ? 14 : 17, weight: .semibold))
                                .lineLimit(1)
                                .minimumScaleFactor(0.75)
                        }
                        .frame(width: valueWidth, alignment: .leading)

                        SentimentGauge(value: value.value)
                            .frame(width: gaugeWidth, height: geometry.size.height * 0.9)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                    .padding(.horizontal, compact ? 4 : 8)
                }
                .accessibilityElement(children: .ignore).accessibilityLabel(
                    "CMC Crypto Fear and Greed Index, \(Int(value.value)) out of 100, \(value.valueClassification).")
            }
        }
    }

    private func statistic(_ title: String, value: String, detail: String?, compact: Bool) -> some View {
        VStack(spacing: compact ? 0 : 2) {
            Text(title).font(compact ? .system(size: 10) : .caption).foregroundStyle(.secondary)
                .lineLimit(1).minimumScaleFactor(0.65)
            Text(value).font(compact ? .system(size: 13, weight: .semibold) : .headline)
                .lineLimit(1).minimumScaleFactor(0.65)
            if let detail, !compact { Text(detail).font(.caption2).foregroundStyle(.secondary) }
        }.frame(maxWidth: .infinity)
    }
    private func shortDate(_ text: String) -> String {
        CMCDateParser.parse(text)?.formatted(.dateTime.month(.abbreviated).day()) ?? text
    }
    private var hasData: Bool {
        !viewModel.cmcAltcoinHistory.isEmpty || !viewModel.cmcFearGreedHistory.isEmpty
            || viewModel.cmcAltcoinLatest != nil || viewModel.cmcFearGreedLatest != nil
    }
    private var skeleton: some View {
        RoundedRectangle(cornerRadius: 12).fill(.secondary.opacity(0.1)).overlay {
            VStack {
                RoundedRectangle(cornerRadius: 5).fill(.secondary.opacity(0.12)).frame(width: 100, height: 38)
                RoundedRectangle(cornerRadius: 12).fill(.secondary.opacity(0.08)).padding()
            }
        }.redacted(reason: .placeholder)
    }
    private func errorState(_ error: String) -> some View {
        ContentUnavailableView(
            "Unable to load CoinMarketCap data", systemImage: "wifi.exclamationmark", description: Text(error)
        ).overlay(alignment: .bottom) { Button("Retry") { Task { await viewModel.fetchCoinMarketCap(force: true) } } }
    }
}

private struct MetricPoint: Identifiable {
    let id = UUID()
    let date: Date
    let value: Double
    let classification: String
    let marketCap: Double?
}

private struct MetricHistoryPlot: View {
    let points: [MetricPoint]
    let showArea: Bool
    let showZones: Bool
    @State private var hover: CGPoint?
    var body: some View {
        GeometryReader { geo in
            Canvas { context, size in
                let inset = EdgeInsets(top: 10, leading: 10, bottom: 22, trailing: 34)
                let rect = CGRect(
                    x: inset.leading, y: inset.top, width: size.width - inset.leading - inset.trailing,
                    height: size.height - inset.top - inset.bottom)
                if showZones {
                    context.fill(
                        Path(CGRect(x: rect.minX, y: rect.minY, width: rect.width, height: rect.height * 0.25)),
                        with: .color(.green.opacity(0.06)))
                    context.fill(
                        Path(
                            CGRect(
                                x: rect.minX, y: rect.minY + rect.height * 0.75, width: rect.width,
                                height: rect.height * 0.25)), with: .color(.blue.opacity(0.06)))
                }
                for tick in [0.0, 25, 50, 75, 100] {
                    let y = rect.maxY - CGFloat(tick / 100) * rect.height
                    var p = Path()
                    p.move(to: CGPoint(x: rect.minX, y: y))
                    p.addLine(to: CGPoint(x: rect.maxX, y: y))
                    context.stroke(
                        p, with: .color(.secondary.opacity(tick == 25 || tick == 75 ? 0.35 : 0.15)),
                        style: StrokeStyle(lineWidth: 1, dash: tick == 25 || tick == 75 ? [4, 3] : []))
                    context.draw(
                        Text("\(Int(tick))").font(.caption2).foregroundColor(.secondary),
                        at: CGPoint(x: size.width - 15, y: y))
                }
                guard points.count > 1 else { return }
                let mapped = points.enumerated().map {
                    CGPoint(
                        x: rect.minX + CGFloat($0.offset) / CGFloat(points.count - 1) * rect.width,
                        y: rect.maxY - CGFloat($0.element.value / 100) * rect.height)
                }
                var line = Path()
                line.move(to: mapped[0])
                mapped.dropFirst().forEach { line.addLine(to: $0) }
                if showArea {
                    var area = line
                    area.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
                    area.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
                    area.closeSubpath()
                    context.fill(
                        area,
                        with: .linearGradient(
                            Gradient(colors: [.blue.opacity(0.25), .blue.opacity(0.01)]),
                            startPoint: CGPoint(x: 0, y: rect.minY), endPoint: CGPoint(x: 0, y: rect.maxY)))
                }
                context.stroke(line, with: .color(.blue), style: StrokeStyle(lineWidth: 2, lineJoin: .round))
            }
            .overlay { if let hover, !points.isEmpty { tooltip(in: geo.size, location: hover) } }
            .onContinuousHover { phase in if case .active(let p) = phase { hover = p } else { hover = nil } }
        }
    }
    private func tooltip(in size: CGSize, location: CGPoint) -> some View {
        let index = Int(((location.x - 10) / max(1, size.width - 44) * CGFloat(max(0, points.count - 1))).rounded())
            .clamped(to: 0...max(0, points.count - 1))
        let p = points[index]
        return VStack(alignment: .leading, spacing: 2) {
            Text(p.date.formatted(.dateTime.month(.abbreviated).day().year())).font(.caption.bold())
            Text("Index  \(Int(p.value))").font(.caption)
            Text(p.classification).font(.caption).foregroundStyle(.secondary)
            if let cap = p.marketCap { Text("Market Cap  \(PriceFormatter.compact(cap))").font(.caption2) }
        }.padding(7).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 7)).position(
            x: min(max(location.x, 80), size.width - 80), y: max(40, location.y - 45))
    }
}

private struct SeasonScale: View {
    let value: Double
    var compact = false
    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                LinearGradient(colors: [.orange, .gray, .blue], startPoint: .leading, endPoint: .trailing)
                    .frame(height: compact ? 7 : 9).clipShape(Capsule())
                    .overlay(alignment: .leading) {
                        Triangle().fill(.primary).frame(width: compact ? 9 : 11, height: compact ? 7 : 8)
                            .offset(
                                x: CGFloat(value / 100) * (geo.size.width - (compact ? 9 : 11)), y: compact ? -7 : -9)
                    }
                HStack(spacing: 3) {
                    Text("0  Bitcoin Season").lineLimit(1)
                    Spacer(minLength: 2)
                    Text("Neutral").lineLimit(1)
                    Spacer(minLength: 2)
                    Text("Altcoin Season  100").lineLimit(1)
                }
                .font(.system(size: compact ? 8 : 10))
                .minimumScaleFactor(0.65)
                .foregroundStyle(.secondary)
                .offset(y: compact ? 11 : 14)
            }
        }.frame(height: compact ? 26 : 34)
    }
}
private struct SentimentGauge: View {
    let value: Double
    var body: some View {
        GeometryReader { geo in
            Canvas { c, s in
                let center = CGPoint(x: s.width / 2, y: s.height * 0.88)
                let r = min(s.width / 2 - 18, s.height * 0.78)
                let colors: [Color] = [.red, .orange, .yellow, .green, .mint]
                for i in 0..<5 {
                    var p = Path()
                    p.addArc(
                        center: center, radius: r, startAngle: .degrees(180 + Double(i) * 36),
                        endAngle: .degrees(180 + Double(i + 1) * 36), clockwise: false)
                    c.stroke(
                        p, with: .color(colors[i]), style: StrokeStyle(lineWidth: max(8, r * 0.13), lineCap: .butt))
                }
                let a = Double.pi * (1 + value / 100)
                let end = CGPoint(x: center.x + cos(a) * r * 0.72, y: center.y + sin(a) * r * 0.72)
                var needle = Path()
                needle.move(to: center)
                needle.addLine(to: end)
                c.stroke(needle, with: .color(.primary), style: StrokeStyle(lineWidth: 3, lineCap: .round))
                c.fill(
                    Path(ellipseIn: CGRect(x: center.x - 5, y: center.y - 5, width: 10, height: 10)),
                    with: .color(.primary))
                c.draw(
                    Text("0").font(.caption).foregroundColor(.secondary), at: CGPoint(x: center.x - r, y: center.y + 12)
                )
                c.draw(
                    Text("100").font(.caption).foregroundColor(.secondary),
                    at: CGPoint(x: center.x + r, y: center.y + 12))
            }
        }.animation(.easeOut(duration: 0.6), value: value)
    }
}
private struct Triangle: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        p.move(to: CGPoint(x: r.midX, y: r.maxY))
        p.addLine(to: CGPoint(x: r.minX, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX, y: r.minY))
        p.closeSubpath()
        return p
    }
}
