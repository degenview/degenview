# Task: Implement TradingView-Compatible Webhooks in DegenView

Implement production-quality webhook support in the existing DegenView macOS application.

The implementation should reproduce TradingView's webhook behavior as closely as practical while fitting DegenView's existing alert architecture.

Webhooks must work with:

1. Existing regular price alerts.
2. Pine Script `alert()` alerts.
3. Pine Script `alertcondition()` subscriptions where supported by the existing Pine alert system.
4. Pine strategy order-fill alerts where supported by the existing broker/alert architecture.

Users must be able to configure multiple reusable webhook endpoints globally from DegenView's main Settings window.

Each alert can target zero, one, or multiple configured webhook endpoints.

Webhook endpoints must be testable directly from Settings.

The HTTP payload/message behavior should match TradingView's behavior rather than introducing a DegenView-specific webhook format.

---

# 1. IMPORTANT: inspect the existing architecture first

Do not begin by creating a generic alert framework.

DegenView already has two mature alert systems with deliberately different execution models.

Relevant architecture:

```text
Model/
├── PriceAlertModels.swift

ViewModel/
├── AlertStore.swift
├── PineAlertStore.swift
└── PineAlertCoordinator.swift

View/
├── PriceAlertEditor.swift
├── AlertsCenterView.swift
├── AlertRuleRow.swift
├── AlertHistoryRow.swift
├── AlertHistoryList.swift
├── GlobalAlertBanner.swift
├── PineAlertEditor.swift
├── PineAlertListView.swift
├── AppSettingsView.swift
├── NotificationSettingsView.swift
├── SettingsPage.swift
├── SettingsField.swift
└── SettingsStatusBadge.swift

Pine/
├── Language/
├── Runtime/
│   ├── PineRuntimeSession*.swift
│   ├── PineExecutionHost.swift
│   ├── PineExecutionController.swift
│   └── PineExecutionScheduler.swift
├── Broker/
├── Model/
└── View/

Service/
├── LocalPriceAlertEngine.swift
├── MarketQuoteCoordinator.swift
├── AppDatabase.swift
├── AppDatabase+Schema.swift
└── AlertRuntimePersistence.swift
```

Search the repository for the actual implementations of:

```text
PineAlertDispatcher
PineAlertChannel
PineAlertRouter
PineAlertSubscription
PineAlertFrequencyGuard
PineAlertEvent
PineExecutionUpdate.alerts
alert_runtime.lock
alert_command
pine_alert_event
pine_alert_subscription
```

Understand those implementations before changing anything.

---

# 2. Preserve the existing alert architecture

Do NOT merge price alerts and Pine alerts into a new generic alert engine.

They intentionally have different execution models.

## Price alerts

Price alerts are evaluated by one `AlertRuntimeHost`.


The GUI sends alert mutations through:

```text
alert_command
```

and the runtime owner evaluates the persisted alert snapshot.

Therefore:

**price-alert webhooks must work even when the main DegenView application is not the active alert-runtime owner.**

Do not implement price-alert webhook delivery exclusively inside `AlertStore`.

The login-item agent must be capable of performing webhook delivery.

## Pine alerts

Pine alerts are app-only.

The existing flow is approximately:

```text
Pine runtime
    ↓
PineExecutionUpdate.alerts
    ↓
ChartViewModel.pineAlertHandler
    ↓
PineAlertCoordinator
    ↓
PineAlertRouter
    ↓
PineAlertFrequencyGuard
    ↓
pine_alert_event
    ↓
PineAlertDispatcher
    ↓
PineAlertChannel
```

Existing channels include things such as:

```text
macOS notification
in-app banner
```

Webhook delivery should integrate naturally into this existing channel architecture.

Conceptually:

```text
PineAlertDispatcher
        │
        ├── NotificationPineAlertChannel
        ├── BannerPineAlertChannel
        └── WebhookPineAlertChannel
                       │
                       ▼
              WebhookDeliveryService
```

Do not create a second Pine alert pipeline.

---

# 3. Shared webhook infrastructure

Although price alerts and Pine alerts have different pipelines, they should share the actual webhook infrastructure.

Create reusable components for:

```text
Webhook endpoint configuration
Webhook endpoint persistence
Webhook URL validation
Webhook HTTP transport
Webhook message rendering
Webhook delivery results
Webhook delivery history
Webhook URL redaction
```

Conceptually:

```text
                     WebhookEndpointStore
                              │
                              ▼
                    WebhookDeliveryService
                       ▲              ▲
                       │              │
              Price Alert         Pine Alert
               Runtime             Channel
```

Do NOT introduce a new global `AlertDeliveryCoordinator` merely to force both existing alert systems through the same abstraction.

The shared boundary should be webhook delivery, not alert evaluation.

---

# 4. TradingView-compatible HTTP behavior

Webhook delivery uses:

```text
HTTP POST
```

The HTTP request body must be the final rendered alert message.

Do NOT introduce a mandatory DegenView envelope.

For example, do NOT automatically transform:

```text
BUY BTCUSDT
```

into:

```json
{
    "event": "alert",
    "message": "BUY BTCUSDT"
}
```

TradingView sends the configured alert message itself as the HTTP body.

DegenView should do the same.

---

# 5. JSON vs plain-text behavior

Match TradingView's behavior.

After all placeholders/message substitutions have been resolved:

```text
Final message
     ↓
Is the entire message valid JSON?
     │
     ├── yes
     │    ↓
     │ application/json
     │
     └── no
          ↓
       text/plain
```

Example:

```json
{"action":"buy","symbol":"BTCUSDT"}
```

should be sent as:

```http
Content-Type: application/json
```

while:

```text
BUY BTCUSDT
```

should be:

```http
Content-Type: text/plain
```

Use proper JSON parsing.

Do NOT determine JSON validity with checks such as:

```swift
message.hasPrefix("{")
```

Do not pretty-print or reserialize valid JSON.

The outbound body must remain the user's rendered message.

Placeholder substitution must happen BEFORE JSON detection.

For example:

```json
{"symbol":"{{ticker}}","price":{{close}}}
```

becomes:

```json
{"symbol":"BTCUSDT","price":100000.5}
```

and only then should JSON validity be determined.

---

# 6. Webhook endpoint model

Support multiple reusable webhook endpoints.

Create a model following existing DegenView model conventions.

Conceptually:

```swift
struct WebhookEndpoint: Identifiable, Codable, Equatable, Sendable {
    let id: UUID

    var name: String
    var description: String
    var url: String
    var isEnabled: Bool

    var createdAt: Date
    var updatedAt: Date
}
```

Adapt the exact fields/types to existing architecture.

Requirements:

- stable UUID
- user-visible name
- optional description
- URL
- enabled state
- created timestamp
- updated timestamp

Alerts reference endpoint IDs.

Do NOT identify webhook configurations by URL or name.

Example:

```text
Trading Bot
Production execution endpoint
https://example.com/webhook/...

Discord
Development alerts
https://example.com/hooks/...
```

---

# 7. Webhook persistence

DegenView uses one SQLite database:

```text
degenview.sqlite
```

through:

```text
AppDatabase
GRDB
DatabasePool
WAL
```

Follow the existing persistence architecture.

IMPORTANT:

The repository currently uses a single idempotent:

```text
createSchema
```

There are no database migrations.

Do NOT introduce a migration framework solely for webhooks.

Extend:

```text
AppDatabase+Schema.swift
```

and/or create an appropriate focused database extension such as:

```text
AppDatabase+Webhooks.swift
```

if consistent with the repository organization.

Potential tables:

```text
webhook_endpoint
webhook_delivery
```

Use the project's existing singular/plural naming convention after inspecting current tables.

---

# 8. Webhook endpoint persistence design

Conceptually:

```text
webhook_endpoint

id
name
description
url_reference / secure storage reference
is_enabled
created_at
updated_at
```

However, inspect the project's existing credential handling before deciding how `url` is stored.

Webhook URLs frequently contain secret credentials in:

```text
path components
query parameters
```

Example:

```text
https://example.com/webhook/<secret-token>
```

Treat the complete webhook URL as potentially sensitive.

---

# 9. Keychain/security

DegenView already stores sensitive credentials such as Alpaca and CoinMarketCap API credentials in macOS Keychain.

Inspect and reuse that infrastructure where appropriate.

Investigate:

```text
KeychainPolicy
Alpaca credential storage
CoinMarketCap credential storage
```

Prefer storing sensitive webhook URL material in Keychain if doing so integrates cleanly with both:

```text
main app
login-item alert agent
```

The price-alert agent must be able to access whatever secure storage mechanism is selected.

Do NOT blindly adopt Keychain if the login-item agent cannot access the same item under the existing entitlements/access-group configuration.

Determine the correct architecture from the repository.

Document the final security decision.

Never write full webhook URLs containing secrets to production logs.

---

# 10. KeychainPolicy compatibility

The existing architecture supports:

```text
KeychainPolicy.isDisabled
```

during XCTest or when:

```text
DEGENVIEW_NO_KEYCHAIN=1
```

If webhook endpoints use Keychain, follow the same policy.

Tests must not trigger unexpected Keychain prompts.

---

# 11. Settings UI

Webhook configuration belongs in DegenView's main Settings window.

The current settings architecture includes:

```text
AppSettingsView.swift
AppearanceSettingsView.swift
AlpacaSettingsView.swift
CoinMarketCapSettingsView.swift
NotificationSettingsView.swift
SettingsPage.swift
SettingsField.swift
SettingsStatusBadge.swift
SettingsCardRow.swift
```

Add a dedicated page such as:

```text
WebhookSettingsView.swift
```

and expose it through `AppSettingsView`.

Follow the existing visual design and settings-page conventions.

Do NOT put all webhook UI directly into `AppSettingsView`.

---

# 12. Webhook Settings page

Provide a native macOS management interface.

Conceptually:

```text
Webhooks

Send alert events to external HTTP endpoints.

┌─────────────────────────────────────────────┐
│ ● Trading Bot                              │
│   Production execution                     │
│   https://example.com/••••••               │
│                               Test      ⋯   │
├─────────────────────────────────────────────┤
│ ○ Discord                                  │
│   Development notifications                │
│   https://discord.com/••••••               │
│                               Test      ⋯   │
└─────────────────────────────────────────────┘

+ Add Webhook
```

Support:

- create
- edit
- delete
- enable
- disable
- test

Follow DegenView's existing sheet/card conventions.

---

# 13. Endpoint editor

The endpoint editor should contain:

```text
Name
Description
Webhook URL
Enabled
```

Validate before saving.

Provide useful validation messages.

Do not expose internal networking errors directly.

---

# 14. URL validation

Centralize webhook URL validation.

At minimum:

- require HTTP or HTTPS
- require a host
- reject malformed URLs
- reject unsupported schemes

Reject:

```text
file://
ftp://
javascript:
```

For close TradingView compatibility, support TradingView's documented webhook port restrictions:

```text
HTTP  → port 80
HTTPS → port 443
```

If no explicit port is provided, use the normal scheme default.

Centralize this policy rather than scattering it across UI/network code.

---

# 15. SSRF / unsafe destinations

Because webhook destinations are user controlled, explicitly define behavior for:

```text
localhost
127.0.0.1
::1
link-local addresses
private LAN addresses
```

Do not accidentally allow or reject these through inconsistent checks.

For a desktop trading application, local endpoints may legitimately be useful for:

```text
local trading bridges
development servers
automation software
```

Therefore inspect the intended DegenView security model and choose an explicit policy.

If local endpoints are supported, document it.

Do not permit unsupported URL schemes regardless.

---

# 16. WebhookDeliveryService

Create a reusable asynchronous transport service.

Conceptually:

```swift
actor WebhookDeliveryService {
    func deliver(
        message: String,
        to endpoint: WebhookEndpoint
    ) async -> WebhookDeliveryResult
}
```

It should own:

```text
endpoint resolution
URL validation
URLRequest construction
POST method
Content-Type
body encoding
timeout
network request
HTTP response classification
duration measurement
safe error mapping
```

Use:

```swift
URLSession
```

Do not add a third-party networking framework solely for this feature.

---

# 17. Testability

The HTTP layer must be injectable/mockable.

Tests must be able to inspect:

```text
URL
HTTP method
headers
body
timeout
```

without contacting the Internet.

Use the project's preferred dependency-injection style.

An injectable `URLSession`/transport protocol is acceptable.

Avoid global `URLSession.shared` hardcoded throughout the feature.

---

# 18. Timeout

Match TradingView's webhook behavior as closely as practical.

Use approximately:

```text
3 seconds
```

as the webhook request timeout.

Centralize the value.

For example:

```swift
static let requestTimeout: TimeInterval = 3
```

Do not scatter magic numbers throughout the code.

---

# 19. HTTP result semantics

Treat:

```text
200...299
```

as successful delivery.

Treat non-2xx responses as failed deliveries.

Examples:

```text
400 → failure
401 → failure
404 → failure
429 → failure
500 → failure
503 → failure
```

A network request completing does not automatically mean webhook delivery succeeded.

---

# 20. Retries

Do NOT implement automatic retries by default.

Webhook endpoints are frequently connected to trading automation.

Automatically retrying:

```text
BUY
```

could result in duplicate downstream orders.

Initial behavior should therefore be:

```text
one alert trigger
      ↓
one attempt per selected webhook endpoint
```

Record failures.

Design the transport so retry policies could be added explicitly in the future.

---

# 21. Multiple destinations

An alert may target:

```text
zero endpoints
one endpoint
multiple endpoints
```

Store stable endpoint IDs.

Conceptually:

```swift
webhookEndpointIDs: Set<UUID>
```

Adapt to existing Codable/database models.

Delivery to endpoints should be independent.

For:

```text
Webhook A
Webhook B
Webhook C
```

if A fails:

```text
A → failed
B → still attempted
C → still attempted
```

Where appropriate, perform independent network requests concurrently.

---

# 22. Disabled/deleted endpoints

If an endpoint is disabled:

```text
isEnabled == false
```

skip delivery.

Do not consider this a failure of the alert itself.

If an endpoint referenced by an old alert has been deleted, handle the dangling ID safely.

Do not crash.

The UI may optionally warn that an alert references a deleted webhook.

---

# 23. Webhook test feature

Every endpoint must be testable from Settings.

Provide:

```text
Test Webhook
```

Use the REAL production `WebhookDeliveryService`.

Do not implement a separate networking path for tests.

Default test payload:

```text
DegenView webhook test
```

which should be sent as:

```text
text/plain
```

Optionally provide an editable test payload.

If the user enters:

```json
{"event":"DegenView webhook test"}
```

the normal content-type detection should produce:

```text
application/json
```

---

# 24. Test UI feedback

Show states such as:

```text
Not tested
Testing…
Delivered
Failed
Timed out
```

For successful delivery:

```text
Delivered
HTTP 200 · 143 ms
```

For failures:

```text
Failed
HTTP 401 · 98 ms
```

or:

```text
Timed out
3.0 s
```

Use existing:

```text
SettingsStatusBadge
```

and related settings UI conventions where appropriate.

Do not show raw `NSError` dumps.

---

# 25. WebhookDeliveryResult

Model results explicitly.

Conceptually:

```swift
struct WebhookDeliveryResult: Sendable {
    let endpointID: UUID
    let timestamp: Date
    let duration: TimeInterval

    let statusCode: Int?
    let succeeded: Bool
    let error: WebhookDeliveryError?
}
```

Use typed errors such as:

```text
invalidURL
unsupportedScheme
unsupportedPort
disabledEndpoint
timeout
networkFailure
httpFailure
missingEndpoint
```

Do not use localized UI strings as domain state.

---

# 26. Delivery history

Persist webhook delivery attempts where useful.

Conceptually store:

```text
id
endpoint ID
alert/event ID
source
timestamp
status code
duration
success
typed error
```

Avoid storing secrets.

Do not persist complete destination URLs in delivery-history rows.

Do not persist HTTP response bodies by default.

Be conservative about persisting complete outbound payloads because alert payloads may themselves contain credentials or sensitive information.

---

# 27. Delivery history and alerts UI

Where practical, integrate webhook delivery state with existing alert history.

For example:

```text
BTCUSDT crossed above $100,000
Triggered 14:32:07

Notification       ✓
Trading Bot        ✓ 200 · 143 ms
Discord            ✕ 401 · 98 ms
```

Keep:

```text
alert trigger
```

separate from:

```text
delivery result
```

An alert successfully triggering does NOT become a failed alert simply because one webhook destination failed.

---

# 28. Price-alert integration

Integrate directly with the existing:

```text
PriceAlertModels.swift
AlertStore.swift
LocalPriceAlertEngine.swift
AlertRuntimePersistence.swift
PriceAlertEditor.swift
AlertsCenterView.swift
```

Do not create another price-alert evaluator.

Extend existing price alert configuration to support:

```text
Webhook endpoint IDs
Webhook message/template
```

Existing price alerts without webhook configuration must behave exactly as before.

Safe default:

```text
webhookEndpointIDs = []
```

---

# 29. Price-alert UI

Extend the existing `PriceAlertEditor`.

Provide delivery configuration similar to:

```text
Notifications

☑ macOS notification

Webhooks

☑ Trading Bot
☐ Discord
☑ Automation Server

Message

BTCUSDT crossed {{close}}
```

Use the existing compact alert editor design rather than introducing a visually unrelated form.

If the existing alert model already has a message/title concept, reuse it.

---

# 30. Price alerts must work in the agent

This requirement is mandatory.

Because price alerts can run under the login-item alert agent:

```text
AlertRuntimeHost
      ↓
LocalPriceAlertEngine
```

the runtime owner must be able to:

1. resolve configured webhook endpoints
2. render the message
3. perform HTTP delivery
4. persist delivery results

without requiring `AlertStore` or SwiftUI.

Keep webhook networking and persistence in code compiled by both relevant targets.

Do not introduce dependencies from shared alert runtime code into app-only SwiftUI code.

---

# 31. Preserve cross-process ownership

Do not accidentally cause BOTH:

```text
main app
login-item agent
```

to send the same price-alert webhook.

Only the current alert runtime owner should initiate webhook delivery for price alerts.

Preserve the existing:

```text
alert_runtime.lock
```

ownership model.

Add tests around this behavior if the architecture permits.

---

# 32. Price-alert message templates

Support TradingView-style placeholders where DegenView has authoritative values.

Prioritize:

```text
{{ticker}}
{{exchange}}
{{open}}
{{high}}
{{low}}
{{close}}
{{volume}}
{{time}}
{{interval}}
```

Only expose values DegenView actually knows at alert-trigger time.

Do not fabricate values.

Create a centralized message renderer rather than implementing replacement logic inside UI or networking code.

Conceptually:

```swift
struct AlertMessageRenderer {
    func render(
        template: String,
        context: AlertMessageContext
    ) -> String
}
```

---

# 33. Placeholder correctness

Do not implement placeholder replacement as a series of unsafe global substring replacements.

Recognize exact placeholders.

For example:

```text
{{close}}
```

should resolve.

Malformed text such as:

```text
foo{{close
```

should not accidentally mutate unrelated text.

Define behavior for unknown placeholders based on TradingView compatibility.

Add tests.

---

# 34. Pine integration

Do not change Pine semantics merely to support webhooks.

Pine scripts must NOT contain webhook URLs.

Do NOT introduce:

```pine
webhook("https://...")
```

Do NOT introduce:

```pine
alert("BUY", url="...")
```

Pine generates alert events.

DegenView's alert subscription determines delivery channels.

---

# 35. Existing Pine alert pipeline

Reuse:

```text
PineExecutionUpdate.alerts
PineAlertCoordinator
PineAlertRouter
PineAlertFrequencyGuard
PineAlertDispatcher
PineAlertChannel
PineAlertSubscription
pine_alert_event
pine_alert_subscription
```

Do not replace them.

Webhook delivery should become another independent channel.

Conceptually:

```text
Pine alert event
       ↓
PineAlertRouter
       ↓
frequency guard
       ↓
persist PineAlertEvent
       ↓
PineAlertDispatcher
       ├── notification
       ├── banner
       └── webhook
```

---

# 36. Pine webhook channel

Implement an appropriate channel conforming to the existing:

```text
PineAlertChannel
```

abstraction.

Conceptually:

```swift
struct WebhookPineAlertChannel: PineAlertChannel {
    ...
}
```

Use the actual protocol shape in the repository.

The channel should:

1. inspect the subscription's selected webhook endpoint IDs
2. resolve enabled endpoints
3. obtain the final Pine alert message
4. asynchronously dispatch each endpoint through `WebhookDeliveryService`
5. persist delivery results

Do not duplicate Pine frequency logic inside the webhook channel.

By the time an event reaches `PineAlertDispatcher`, frequency/dedupe decisions should remain owned by the existing Pine alert pipeline.

---

# 37. Pine `alert()`

Preserve existing Pine runtime behavior.

For:

```pine
if ta.crossover(fast, slow)
    alert(
        '{"action":"buy","symbol":"' + syminfo.ticker + '"}',
        alert.freq_once_per_bar_close
    )
```

the runtime evaluates the dynamic string.

The existing Pine alert system determines whether the event is admitted.

The webhook channel sends the resulting message.

Do not reinterpret or rebuild the Pine expression at delivery time.

---

# 38. Pine alert frequencies

Preserve the existing frequency guard.

Continue supporting whatever DegenView currently implements for:

```pine
alert.freq_all
alert.freq_once_per_bar
alert.freq_once_per_bar_close
```

Do not implement webhook-specific frequency tracking.

Webhook delivery must respect the exact same admitted Pine events as other Pine alert channels.

---

# 39. Historical Pine execution

DegenView already distinguishes realtime execution from rebuild/history.

The current architecture says:

```text
PineExecutionUpdate.alerts
```

contains realtime executions reaching the alert coordinator.

Preserve that design.

Historical rebuilds must send:

```text
ZERO external webhooks
```

This is mandatory.

Loading a script over years of historical candles must never replay historical webhook calls.

Add an explicit regression test proving this.

---

# 40. Realtime rollback

DegenView models TradingView-like realtime Pine rollback.

Do not make webhook delivery bypass the existing Pine event/frequency/dedupe pipeline.

An intrabar Pine execution that is later rolled back must not cause arbitrary duplicate webhook side effects outside the semantics already enforced by:

```text
PineAlertFrequencyGuard
PineAlertRouter
Pine alert dedupe keys
```

Study this carefully before changing runtime code.

Do not add networking directly to `PineRuntimeSession`.

---

# 41. `alertcondition()`

Inspect DegenView's current `alertcondition()` implementation.

If already supported, integrate webhook endpoint selection into the resulting `PineAlertSubscription`.

Preserve TradingView's conceptual model:

```pine
alertcondition(
    bullish,
    "Bullish crossover",
    "Bullish crossover on {{ticker}} at {{close}}"
)
```

declares an available condition.

It does NOT perform HTTP.

The user creates a persistent Pine alert subscription from the UI.

That subscription selects:

```text
condition
delivery channels
webhook endpoints
```

Do not create a webhook merely because `alertcondition()` exists in source.

If `alertcondition()` is not fully supported today, do not rewrite the Pine compiler as part of this feature.

Document the compatibility gap.

---

# 42. PineAlertEditor

Extend the existing:

```text
PineAlertEditor.swift
```

rather than creating a separate webhook-specific Pine alert editor.

Allow the user to select webhook endpoints for a Pine alert subscription.

Conceptually:

```text
Create Alert

Condition
────────────────────────
Any alert() function call

Delivery
────────────────────────
☑ Notification

Webhooks
────────────────────────
☑ Trading Bot
☐ Discord
☑ Local Automation
```

Follow existing DegenView alert UI conventions.

---

# 43. Pine subscription persistence

Webhook endpoint selections belong on the persistent:

```text
PineAlertSubscription
```

or an appropriate associated persisted model.

Store stable endpoint IDs.

Do not copy endpoint URLs into each subscription.

Renaming an endpoint should not break subscriptions.

Disabling an endpoint should stop delivery globally without rewriting every subscription.

---

# 44. Strategy order-fill alerts

Inspect the current Pine broker and Pine alert implementation before adding anything.

DegenView's strategy broker emulator already tracks:

```text
orders
triggers
fills
trades
equity
```

If strategy order-fill alert events already exist or naturally integrate with `PineAlertDispatcher`, support them.

The correct semantic point is:

```text
strategy order submitted
        ↓
broker emulator
        ↓
actual simulated fill
        ↓
order-fill alert event
        ↓
PineAlertDispatcher
        ↓
webhook channel
```

Do NOT fire the webhook merely because:

```pine
strategy.entry(...)
```

was evaluated.

Fire on the simulated fill event.

---

# 45. Strategy alert messages

Where the current compiler/runtime supports it, preserve TradingView-compatible strategy alert message semantics.

Inspect support for:

```pine
strategy.entry(..., alert_message = ...)
strategy.order(..., alert_message = ...)
strategy.exit(..., alert_message = ...)
strategy.close(..., alert_message = ...)
```

and placeholders such as:

```text
{{strategy.order.action}}
{{strategy.order.price}}
{{strategy.order.id}}
{{strategy.order.contracts}}
{{strategy.order.alert_message}}

{{strategy.position_size}}
{{strategy.market_position}}
{{strategy.market_position_size}}

{{strategy.prev_market_position}}
{{strategy.prev_market_position_size}}
```

Only implement placeholders for data the broker emulator can authoritatively provide.

Do not fabricate strategy values.

---

# 46. `//@strategy_alert_message`

Investigate whether the existing Pine annotation infrastructure supports:

```pine
//@strategy_alert_message
```

If support can be added cleanly without a major compiler redesign, implement TradingView-compatible behavior.

Otherwise document it as a remaining compatibility gap.

Do not destabilize the compiler for this optional feature.

---

# 47. TradingView alert snapshots

TradingView running Pine alerts conceptually use a snapshot of relevant script/configuration state.

DegenView already persists:

```text
PineAlertSubscription
chart
symbol
timeframe
source hash
```

Study the existing source-hash/re-arm behavior.

Do not replace it with a new snapshot system unless necessary.

Preserve the existing behavior where script changes pause/re-arm or invalidate subscriptions according to current DegenView semantics.

Webhook endpoint IDs are delivery configuration and should fit naturally into the subscription.

---

# 48. Pine alert history

Pine events are already persisted in:

```text
pine_alert_event
```

Do not duplicate the entire event in a webhook-specific table.

Webhook delivery history should reference the existing Pine alert event ID where possible.

Conceptually:

```text
pine_alert_event
      │
      ├── notification
      ├── banner
      └── webhook_delivery
             endpoint_id
             status
             duration
```

---

# 49. Price alert history

Likewise, preserve existing price-alert history.

Associate webhook deliveries with the existing alert trigger/history identity where practical.

Do not create duplicate "webhook alert events" representing the same price trigger.

---

# 50. Concurrency

Webhook networking must never block:

```text
main thread
chart rendering
market quote processing
LocalPriceAlertEngine
PineExecutionHost
PineRuntimeSession
PineExecutionController
PineAlertCoordinator
```

Network delivery must happen asynchronously.

The deterministic alert event should be committed before or independently from remote delivery.

A slow webhook server must not stall alert evaluation.

---

# 51. Delivery ordering

Do not make correctness depend on HTTP requests completing in alert order.

For example:

```text
Alert A
Alert B
```

may complete remotely in a different order due to network latency.

Persist trigger timestamps/event identities.

If ordering guarantees are required by existing architecture, implement them deliberately.

Do not hold Pine execution waiting for network completion merely to preserve HTTP completion order.

---

# 52. Cancellation

Be careful with unstructured tasks.

A webhook initiated from an admitted alert should not disappear merely because a SwiftUI view closes.

Delivery ownership belongs to an appropriate long-lived service/runtime object.

Do not tie production webhook delivery to:

```text
sheet lifetime
view task
temporary ViewModel
```

Settings test requests may be cancellable with the settings UI.

Actual alert deliveries should follow runtime ownership.

---

# 53. App termination

Do not attempt to build a complex durable delivery queue unless required by existing architecture.

However, avoid obviously dropping delivery merely because a view disappears.

Document current guarantees around app/agent termination.

Do not introduce retries or queued redelivery that could duplicate downstream trades.

---

# 54. Redirect behavior

Define HTTP redirect behavior explicitly.

Standard safe HTTP redirects are acceptable.

However, URL validation/security policy should not be trivially bypassed by redirecting from:

```text
https://safe.example.com
```

to an otherwise forbidden destination.

Implement this at the appropriate `URLSession` delegate/transport layer if necessary.

Keep the solution proportional to the desktop app threat model.

---

# 55. Logging

Add useful diagnostics without exposing secrets.

Good:

```text
Webhook delivery failed
endpointID=...
status=401
duration=0.12
```

Bad:

```text
Webhook delivery failed:
https://example.com/webhook/SUPER_SECRET_TOKEN
```

Provide a URL redaction helper if needed.

Never log:

```text
full secret-bearing URL
authorization material
sensitive payload
```

in normal production logs.

---

# 56. Payload handling

Do not modify user payloads.

Do not:

```text
pretty-print JSON
wrap JSON
append newline characters
inject DegenView metadata
reorder JSON keys
escape the complete body again
```

unless required by correct UTF-8 HTTP body construction.

Send the final rendered string as UTF-8.

---

# 57. TradingView-compatible placeholders

Research the exact TradingView alert placeholders relevant to:

```text
regular alerts
Pine alertcondition messages
strategy order-fill alerts
```

Implement only placeholders for which DegenView has authoritative values.

At minimum investigate:

```text
{{ticker}}
{{exchange}}
{{open}}
{{high}}
{{low}}
{{close}}
{{volume}}
{{time}}
{{timenow}}
{{interval}}
```

and strategy placeholders listed earlier.

Keep the placeholder catalog centralized.

Document unsupported placeholders.

---

# 58. Important distinction: `alert()` message

Do not incorrectly apply regular alert placeholder rendering to dynamic Pine `alert()` messages unless TradingView does so.

For:

```pine
alert(
    "RSI: " + str.tostring(rsi),
    alert.freq_once_per_bar_close
)
```

the Pine runtime has already produced the final dynamic message:

```text
RSI: 72.45
```

That message should flow through the existing Pine alert pipeline.

Do not run arbitrary second-stage substitutions over it unless required by TradingView compatibility.

---

# 59. Important distinction: `alertcondition()`

For:

```pine
alertcondition(
    bullish,
    "Bullish",
    "{{ticker}} crossed at {{close}}"
)
```

the message is a template associated with the alert condition.

Resolve TradingView-compatible placeholders at trigger time using authoritative market/bar context.

Keep this distinct from `alert()` dynamic strings.

---

# 60. Webhook settings vs notification settings

Do not overload:

```text
NotificationSettingsView
```

with endpoint management.

Webhook endpoint management deserves its own Settings page because endpoints contain:

```text
name
description
URL
enabled state
test functionality
status
```

However, reuse shared settings components.

---

# 61. Alerts Center

Where useful, show webhook delivery status in:

```text
AlertsCenterView
AlertHistoryRow
PineAlertListView
```

Do not turn every alert row into an overly complex network-debugging panel.

A compact representation is enough:

```text
Webhooks  2/3 delivered
```

with details available through expansion/popover/details UI if consistent with the app.

---

# 62. Endpoint deletion UX

Before deleting an endpoint referenced by alerts, determine whether references exist.

Prefer a confirmation such as:

```text
Delete "Trading Bot"?

This webhook is currently used by 4 alerts.
Deleting it will disable webhook delivery for those references.
```

Then either:

1. remove the endpoint ID from referencing alert configurations, or
2. safely tolerate dangling references.

Prefer explicit cleanup if it can be performed transactionally.

Do not silently corrupt subscriptions.

---

# 63. Endpoint enable/disable semantics

Disabling an endpoint should be a global kill switch.

Example:

```text
Trading Bot
Enabled: OFF
```

All price/Pine alerts referencing it should skip delivery immediately.

The alerts themselves remain active.

Re-enabling it restores future delivery without editing every alert.

---

# 64. Test endpoint semantics

Testing a disabled endpoint should still be possible if the user explicitly presses:

```text
Test
```

because the user may be testing before enabling it.

Production alert delivery must respect:

```text
isEnabled
```

The test action may deliberately bypass enabled-state filtering while still performing all URL/security validation.

Make this distinction explicit in code.

---

# 65. No automatic paper trading integration

DegenView has a separate paper-trading system.

Do NOT automatically connect webhook delivery to:

```text
PaperTradingStore
paper orders
paper account
```

Webhooks are external alert delivery.

Likewise:

```pine
strategy.entry()
```

continues to operate against the Pine broker emulator, not the persistent DegenView paper account.

Any future webhook→paper-trading automation should be a separate feature.

---

# 66. No direct broker execution

Do not implement broker/exchange execution as part of this task.

Webhook payloads may be consumed by an external trading service, but DegenView's responsibility ends at HTTP delivery.

Do not add special Binance/Alpaca order behavior to the webhook service.

---

# 67. Tests: webhook transport

Add comprehensive tests for the shared transport.

Test:

### Plain text

Input:

```text
BUY BTCUSDT
```

Expected:

```text
POST
Content-Type: text/plain
body = exact UTF-8 bytes for "BUY BTCUSDT"
```

### JSON

Input:

```json
{"action":"buy"}
```

Expected:

```text
Content-Type: application/json
```

### Invalid JSON

Input:

```text
{"action":"buy"
```

Expected:

```text
Content-Type: text/plain
```

### JSON array

```json
["BUY","BTCUSDT"]
```

Verify correct JSON detection according to the JSON parsing implementation.

### Unicode

```json
{"message":"価格 🚀"}
```

Verify UTF-8 correctness.

---

# 68. Tests: URL validation

Test:

```text
https://example.com/hook
http://example.com/hook
https://example.com:443/hook
http://example.com:80/hook

ftp://example.com
file:///tmp/foo
javascript:foo

malformed URLs
missing host
unsupported explicit ports
```

Also test the chosen local/private-network policy.

---

# 69. Tests: HTTP responses

Test:

```text
200
201
204
400
401
404
429
500
503
```

Verify:

```text
2xx → success
everything else → failure
```

---

# 70. Tests: timeout

Verify the configured timeout is approximately three seconds.

Verify timeout produces the typed timeout result.

Do not make the test actually wait three seconds if a mock transport can simulate it.

---

# 71. Tests: multiple endpoints

Configure:

```text
A → succeeds
B → fails
C → succeeds
```

Verify all three are attempted.

Verify failure of B does not suppress C.

---

# 72. Tests: disabled endpoints

Verify production alert delivery skips disabled endpoints.

Verify explicit Settings testing can test a disabled endpoint.

---

# 73. Tests: endpoint identity

Rename:

```text
Trading Bot
```

to:

```text
Production Bot
```

while retaining the same UUID.

Existing alerts must continue targeting the endpoint.

---

# 74. Tests: price alerts

Add integration tests proving:

1. Existing price-alert crossing behavior is unchanged.
2. Price alerts with zero webhook endpoints behave exactly as before.
3. A price alert with one endpoint sends one webhook.
4. Multiple endpoints are independently attempted.
5. Disabled endpoints are skipped.
6. Placeholder rendering uses the trigger's authoritative market data.
7. Delivery failure does not alter the alert trigger result.

---

# 75. Tests: cross-process price alerts

Where feasible, test the architecture around:

```text
alert_runtime.lock
```

At minimum ensure webhook delivery is initiated by the runtime owner, not independently by `AlertStore`.

There must be no code path where both GUI and agent naturally send the same webhook for one price trigger.

---

# 76. Tests: Pine `alert()`

Verify an admitted:

```pine
alert("BUY", alert.freq_once_per_bar_close)
```

produces one webhook delivery when the subscription selects one endpoint.

Verify the webhook channel does not implement its own frequency semantics.

---

# 77. Tests: Pine historical execution

MANDATORY regression test:

```text
historical Pine rebuild
        ↓
0 external webhook requests
```

Even if historical bars contain thousands of `alert()` calls.

---

# 78. Tests: Pine realtime execution

Verify realtime admitted alerts do generate webhook deliveries.

Test behavior across:

```text
freq_all
freq_once_per_bar
freq_once_per_bar_close
```

according to existing `PineAlertFrequencyGuard` semantics.

---

# 79. Tests: Pine channel independence

If:

```text
notification succeeds
webhook fails
banner succeeds
```

the Pine event must remain valid.

One channel must not suppress another.

---

# 80. Tests: `alertcondition()`

If supported, test:

```pine
alertcondition(
    close > open,
    "Bullish",
    "{{ticker}} {{close}}"
)
```

Verify:

- declaration creates/selects a condition through existing architecture
- no HTTP happens merely from declaration
- active subscription causes delivery when triggered
- placeholders resolve correctly

---

# 81. Tests: strategy fills

If strategy order-fill alerts are supported:

```text
order created → no fill webhook yet
fill occurs   → webhook
```

Test `alert_message` where supported.

Do not weaken broker-emulator determinism.

---

# 82. Tests: persistence

Test:

```text
create endpoint
restart/reload
endpoint remains

rename endpoint
references remain valid

disable endpoint
references remain but delivery stops

delete endpoint
references handled safely
```

Follow existing AppDatabase testing conventions.

---

# 83. Tests: secrets

Verify URL redaction.

Example:

```text
https://example.com/webhook/abc123secret
```

must not appear verbatim in diagnostic descriptions intended for logging.

If Keychain is used, tests must respect:

```text
KeychainPolicy.isDisabled
```

and must not cause Keychain UI.

---

# 84. Performance

Do not perform unnecessary database queries for every individual Pine execution if no admitted alert exists.

Webhook endpoint resolution occurs after an alert has actually triggered/admitted.

Cache endpoint configuration where appropriate, but keep cross-process correctness in mind for price alerts.

Do not cache secrets indefinitely without considering endpoint edits/deletions.

---

# 85. Threading and actors

Respect existing actor isolation.

Do not fix concurrency errors by adding indiscriminate:

```swift
@MainActor
```

to networking/runtime types.

UI state belongs on the main actor.

HTTP delivery should not.

Price alert evaluation and Pine execution should remain on their existing serialized execution paths.

Do not create actor reentrancy that changes alert ordering/dedupe behavior.

---

# 86. Model boundaries

The current architecture requires:

```text
Pine/Model
Model/Script
```

to remain free of compiler/runtime types because the alert agent target compiles them.

Preserve that boundary.

Do not introduce runtime/compiler dependencies into shared models merely for webhooks.

Shared webhook models used by the alert agent must likewise avoid app-only/Pine-runtime dependencies.

---

# 87. Database failure behavior

DegenView intentionally avoids replacing failed persisted state with empty state.

Preserve existing database failure semantics.

Do not silently interpret:

```text
failed to load webhook endpoints
```

as:

```text
there are no webhook endpoints
```

if doing so could cause destructive writes.

Follow existing `AppDatabase` patterns.

---

# 88. Existing behavior must remain unchanged by default

After this implementation, a user who never configures a webhook should observe no behavioral change.

Specifically:

```text
existing price alerts continue working
existing Pine notifications continue working
existing Pine banners continue working
existing Pine frequency guarding remains unchanged
existing Pine event persistence remains unchanged
existing strategy simulation remains unchanged
paper trading remains unchanged
```

Webhook delivery is opt-in.

---

# 89. Scope exclusions

Do NOT implement:

- generic replacement alert framework
- Pine `webhook()` builtin
- webhook URLs inside Pine source
- direct broker trading
- automatic paper trading
- automatic retry queues
- arbitrary HTTP methods
- GET webhooks
- arbitrary custom HTTP headers unless required later
- OAuth
- webhook receiving/server functionality
- cloud webhook relay
- background Pine execution in the alert agent
- moving the Pine compiler/runtime into the alert agent
- a third-party networking dependency solely for this feature

Keep the implementation focused.

---

# 90. Suggested architecture

The final design should resemble:

```text
                         GLOBAL CONFIGURATION

                         WebhookSettingsView
                                │
                                ▼
                         WebhookEndpointStore
                                │
                      AppDatabase / Keychain
                                │
             ┌──────────────────┴──────────────────┐
             │                                     │
             ▼                                     ▼

       PRICE ALERTS                           PINE ALERTS

 AlertRuntimeHost                       PineExecutionUpdate.alerts
 app OR login agent                              │
        │                                        ▼
        ▼                               PineAlertCoordinator
 LocalPriceAlertEngine                           │
        │                                        ▼
        │                                  PineAlertRouter
        │                                        │
        │                                        ▼
        │                              PineAlertFrequencyGuard
        │                                        │
        │                                        ▼
        │                                 pine_alert_event
        │                                        │
        │                                        ▼
        │                                PineAlertDispatcher
        │                                   │    │    │
        │                                   │    │    └─ Banner
        │                                   │    └──── Notification
        │                                   │
        │                                   └───────── WebhookPineAlertChannel
        │                                                   │
        └──────────────────────────┐                        │
                                   ▼                        ▼
                            WebhookDeliveryService
                                   │
                              HTTP POST
                                   │
                                   ▼
                         configured endpoint(s)
```

Do not follow this diagram blindly if the actual implementation has better existing extension points.

Inspect the code first.

---

# 91. Implementation priority

Implement in this order:

1. Inspect existing price/Pine alert implementations.
2. Define shared webhook endpoint/result models.
3. Implement secure endpoint persistence.
4. Implement URL validation.
5. Implement testable `WebhookDeliveryService`.
6. Add transport tests.
7. Add Settings Webhooks page.
8. Add endpoint testing.
9. Integrate webhook endpoint selection into price alerts.
10. Make price webhook delivery work through `AlertRuntimeHost`/agent ownership.
11. Integrate webhook selection into `PineAlertSubscription`.
12. Add webhook implementation of `PineAlertChannel`.
13. Add delivery-history persistence/UI.
14. Add TradingView-compatible placeholder rendering.
15. Add/verify `alertcondition()` integration.
16. Add strategy order-fill webhook integration where supported.
17. Complete regression/integration tests.

Do not start by modifying the Pine runtime.

---

# 92. Before coding

Inspect the following files/directories:

```text
Model/PriceAlertModels.swift

ViewModel/AlertStore.swift
ViewModel/PineAlertStore.swift
ViewModel/PineAlertCoordinator.swift

View/PriceAlertEditor.swift
View/AlertsCenterView.swift
View/AlertRuleRow.swift
View/AlertHistoryRow.swift
View/PineAlertEditor.swift
View/PineAlertListView.swift

View/AppSettingsView.swift
View/NotificationSettingsView.swift
View/SettingsPage.swift
View/SettingsField.swift
View/SettingsStatusBadge.swift
View/SettingsCardRow.swift

Service/LocalPriceAlertEngine.swift
Service/MarketQuoteCoordinator.swift
Service/AlertRuntimePersistence.swift
Service/AppDatabase.swift
Service/AppDatabase+Schema.swift

Pine/Runtime/
Pine/Broker/
Pine/Model/
```

Search globally for:

```text
PineAlertDispatcher
PineAlertChannel
PineAlertRouter
PineAlertSubscription
PineAlertFrequencyGuard
PineAlertEvent

alert(
alertcondition(
alert_message
strategy_alert_message

alert_runtime.lock
alert_command
pine_alert_event
pine_alert_subscription

URLSession
KeychainPolicy
SecItem
```

Understand the current behavior before introducing new types.

---

# 93. TradingView compatibility research

Before implementing compatibility-sensitive behavior, verify current TradingView documentation for:

```text
webhook POST behavior
JSON Content-Type behavior
request timeout
allowed ports
alert placeholders
Pine alert() semantics
alertcondition() semantics
strategy order-fill alert placeholders
strategy alert_message
//@strategy_alert_message
```

Do not rely on assumptions where TradingView documents the behavior.

Keep a concise compatibility note in the implementation summary describing any deliberate deviations.

Do not copy TradingView documentation text into source comments unnecessarily.

---

# 94. Deliverables

Implement the feature directly in the repository.

Return production-quality, compilable Swift.

Do not return only pseudocode or a proposed architecture.

After implementation provide a report containing:

### Files

- files added
- files modified

### Architecture

Explain:

- endpoint storage
- secure URL storage
- shared webhook transport
- price-alert integration
- Pine-alert channel integration
- cross-process app/agent behavior

### TradingView compatibility

Document:

- HTTP method
- Content-Type selection
- timeout
- port restrictions
- supported placeholders
- `alert()` behavior
- `alertcondition()` behavior
- strategy order-fill behavior
- known incompatibilities

### Safety

Explain:

- URL validation
- local/private-network policy
- URL redaction
- Keychain behavior
- redirect policy
- retry policy

### Persistence

Explain:

- new tables
- idempotent `createSchema` changes
- endpoint deletion behavior
- delivery history

Do NOT describe database changes as "migrations" unless the repository architecture itself has changed to support migrations.

### Tests

List tests added for:

- transport
- JSON/plain-text detection
- URL validation
- timeout
- multiple endpoints
- disabled endpoints
- persistence
- price alerts
- cross-process ownership
- Pine alerts
- historical Pine execution
- realtime Pine execution
- strategy fills
- placeholder rendering
- Unicode
- secret redaction

### Regression confirmation

Explicitly confirm:

```text
existing price alerts still work without webhooks
existing Pine notifications still work
existing Pine banners still work
Pine historical rebuild sends no external webhooks
strategy simulation behavior is unchanged
paper trading behavior is unchanged
```

---

# 95. Final design principles

Use these rules when making architectural decisions:

**1. Preserve existing alert pipelines.**

Price alerts and Pine alerts should not be rewritten into a generic alert framework.

**2. Share transport, not evaluation.**

Both systems should share endpoint configuration, persistence, message rendering, and HTTP delivery.

**3. Pine never performs HTTP directly.**

The Pine runtime emits events. Alert subscriptions determine delivery.

**4. Webhook endpoints are global reusable resources.**

Alerts reference stable endpoint IDs.

**5. The message is the payload.**

Do not invent a mandatory DegenView JSON envelope.

**6. Rendering precedes Content-Type detection.**

Resolve templates/placeholders first, then determine whether the final message is valid JSON.

**7. External side effects are realtime only.**

Historical Pine execution must never send webhooks.

**8. Network failures do not change alert semantics.**

A failed webhook does not mean the alert failed to trigger.

**9. No implicit retries.**

Trading-related webhook duplication is potentially dangerous.

**10. Price webhooks must work from the login-item agent.**

Do not accidentally make this an app-only feature.

**11. Secrets are secrets.**

Treat complete webhook URLs as potentially credential-bearing.

**12. Follow the repository's persistence model.**

Use the existing idempotent schema creation approach; do not invent migrations.

**13. Webhooks do not equal trading execution.**

Do not connect them automatically to DegenView Paper Trading or broker APIs.

**14. TradingView compatibility takes precedence over inventing cleaner but incompatible Pine semantics.**

**15. Inspect before changing.**

If the repository already has an appropriate abstraction, extend it rather than creating a parallel one.

