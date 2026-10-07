import Foundation

extension PineRuntimeSession {
    /// `alert(message, freq)` and `alertcondition(condition, title, message)`.
    func alertCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        if call.name == "alert" {
            let b = try bind(call, ["message", "freq"], &context)
            let message = b["message"].map { PineFormat.format($0, nil, mintick: mintick) } ?? ""
            var frequency = PineAlertFrequency.oncePerBar
            if let name = b["freq"].textValue {
                guard let resolved = PineAlertFrequency(pineName: name) else {
                    throw PineDiagnostic.error(
                        "PINE4009", .runtime, "alert() frequency must be an alert.freq_* constant, not '\(name)'.",
                        call.range)
                }
                frequency = resolved
            }
            recordAlert(message, frequency: frequency, site: site, context)
        } else {
            let b = try bind(call, ["condition", "title", "message"], &context)
            guard b["condition"]?.bool == true else { return .void }
            // Unlike alert(), an alertcondition() message is a template: its {{placeholders}} resolve
            // here, when the condition fires, from the bar and symbol the script is running on.
            let template = b["message"].textValue ?? b["title"].textValue ?? ""
            recordAlert(
                AlertMessageRenderer.render(template, context: messageContext(context)), frequency: .all,
                site: site, context)
        }
        return .void
    }

    /// What `{{ticker}}`, `{{close}}` and the rest mean for the bar being executed. A historical bar
    /// uses its own open time for `{{timenow}}`, so replaying history stays deterministic.
    private func messageContext(_ context: PineRuntimeContext) -> AlertMessageContext {
        let bar = context.bar
        return AlertMessageContext(
            ticker: symbol.ticker.isEmpty ? nil : symbol.ticker, exchange: exchangePrefix,
            interval: Self.intervalLabel(barSeconds: barSeconds), open: bar.openPrice, high: bar.highPrice,
            low: bar.lowPrice, close: bar.closePrice, volume: bar.volume, time: bar.openTime,
            now: context.flags.isRealtime ? Date() : bar.openTime)
    }

    /// TradingView's name for a bar length: minutes (`60`), then `D`, `W`, `M`.
    static func intervalLabel(barSeconds: Double) -> String? {
        guard barSeconds >= 60 else { return nil }
        let day = 86_400.0
        guard barSeconds >= day else { return String(Int(barSeconds / 60)) }
        let days = Int((barSeconds / day).rounded())
        switch days {
        case 1: return "D"
        case 7: return "W"
        case 28...31: return "M"
        case 88...93: return "3M"
        case 360...370: return "12M"
        default: return "\(days)D"
        }
    }

    private func recordAlert(
        _ message: String, frequency: PineAlertFrequency, site: Int, _ context: PineRuntimeContext
    ) {
        if frequency == .oncePerBarClose, !context.flags.isConfirmed { return }
        if frequency == .oncePerBar, !oncePerBarLedger.insert(site).inserted { return }
        let event = PineAlertEvent(
            id: allocate(), site: site, bar: working.barIndex, time: context.bar.openTime, message: message,
            frequency: frequency, isRealtime: context.flags.isRealtime, isConfirmed: context.flags.isConfirmed)
        working.alerts.append(event)
        if working.alerts.count > Self.alertLimit {
            working.alerts.removeFirst(working.alerts.count - Self.alertLimit)
        }
        if event.isRealtime { emit(event) }
    }
}
