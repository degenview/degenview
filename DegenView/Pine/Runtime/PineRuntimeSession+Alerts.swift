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
            recordAlert(
                b["message"].textValue ?? b["title"].textValue ?? "", frequency: .all, site: site, context)
        }
        return .void
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
