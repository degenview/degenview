import Foundation

/// The decisions behind the alert editors' webhook picker, kept out of the views so they can be tested.
enum WebhookPickerLogic {
    struct Summary: Equatable {
        /// Selected endpoints that still exist.
        var selectedCount: Int
        /// Selected ids whose endpoint was deleted; delivery skips them.
        var danglingCount: Int
        /// At least one endpoint is selected and every one is switched off in Settings.
        var allPaused: Bool
    }

    static func summary(selection: [UUID], endpoints: [WebhookEndpoint]) -> Summary {
        let byID = Dictionary(endpoints.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let known = selection.compactMap { byID[$0] }
        return Summary(
            selectedCount: known.count, danglingCount: selection.count - known.count,
            allPaused: !known.isEmpty && known.allSatisfy { !$0.isEnabled })
    }

    /// The host (and port) of a webhook URL and nothing else: never the path, query or user info, and so
    /// never anything that could hold a token. Works on templates that contain `{{secret}}`.
    static func host(of url: String) -> String? {
        let neutral = WebhookSecretPlaceholder.replacing(
            in: url.trimmingCharacters(in: .whitespacesAndNewlines), with: "x")
        guard let components = URLComponents(string: neutral), let host = components.host, !host.isEmpty else {
            return nil
        }
        return components.port.map { "\(host):\($0)" } ?? host
    }

    /// `POST · example.com`, or what is wrong with the endpoint.
    static func subtitle(for endpoint: WebhookEndpoint) -> String {
        guard endpoint.isEnabled else { return "Paused in Settings" }
        guard let host = host(of: endpoint.url) else { return endpoint.method.rawValue }
        return "\(endpoint.method.rawValue) · \(host)"
    }

    /// The price shown in the message example: two decimals for ordinary prices (none for whole-unit
    /// currencies), more only when the price is below 1 and needs them. The live price has been through
    /// currency conversion, so as a raw `Double` it carries noise like `83356.00999999998`.
    static func exampleClose(_ price: Decimal, currency: PortfolioCurrency) -> Double {
        let magnitude = abs(price)
        let digits =
            magnitude >= 1
            ? (currency == .JPY ? 0 : 2)
            : PortfolioCurrency.alertFractionDigits(for: magnitude, wholeUnits: false).upperBound
        var value = price
        var rounded = Decimal()
        NSDecimalRound(&rounded, &value, digits, .plain)
        return NSDecimalNumber(decimal: rounded).doubleValue
    }

    /// Adds `token` after what is typed, with a single space between when one is needed.
    static func appending(_ token: String, to text: String) -> String {
        guard !text.isEmpty else { return token }
        guard let last = text.last, !last.isWhitespace else { return text + token }
        return text + " " + token
    }

    /// The message as an alert would send it, from example values, and how it will be sent. Values the
    /// context doesn't have stay as their `{{placeholder}}`.
    static func examplePreview(template: String, context: AlertMessageContext) -> (text: String, isJSON: Bool) {
        let source =
            template.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? AlertMessageRenderer.defaultPriceAlertTemplate : template
        let text = AlertMessageRenderer.render(source, context: context)
        return (text, WebhookPayload.isJSON(Data(text.utf8)))
    }
}
