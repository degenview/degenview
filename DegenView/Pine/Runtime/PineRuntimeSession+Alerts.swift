import Foundation

extension PineRuntimeSession {
    /// `alert(message, freq)` and `alertcondition(condition, title, message)`.
    func alertCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        if call.name == "alert" {
            let b = try bind(call, ["message", "freq"], &context)
            let message = b["message"].map { PineFormat.format($0, nil, mintick: mintick) } ?? ""
            recordAlert(
                message, frequency: b["freq"].textValue ?? "alert.freq_once_per_bar", site: site, context)
        } else {
            let b = try bind(call, ["condition", "title", "message"], &context)
            guard b["condition"]?.bool == true else { return .void }
            recordAlert(
                b["message"].textValue ?? b["title"].textValue ?? "", frequency: "alert.freq_all", site: site,
                context)
        }
        return .void
    }

    private func recordAlert(
        _ message: String, frequency: String, site: Int, _ context: PineRuntimeContext
    ) {
        if frequency == "alert.freq_once_per_bar_close", !context.flags.isConfirmed { return }
        if frequency == "alert.freq_once_per_bar",
            working.alerts.contains(where: { $0.site == site && $0.bar == working.barIndex })
        {
            return
        }
        working.alerts.append(
            .init(
                id: allocate(), site: site, bar: working.barIndex, time: context.bar.openTime,
                message: message))
        if working.alerts.count > Self.alertLimit {
            working.alerts.removeFirst(working.alerts.count - Self.alertLimit)
        }
    }
}
