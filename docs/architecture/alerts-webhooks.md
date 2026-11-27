# Alerts and webhooks

Price and Pine alerts have separate trigger pipelines but share global endpoint configuration and
one transport.

```text
Price: AlertEngine → AlertHost ─────────────┐
Pine: Session → PineSubscriptionHost ──────┼→ claim(trigger, endpoint)
                                           └→ webhook_delivery → result row

Settings → webhook_endpoint (SQLite templates)
         → keychain::webhook_secrets (one Keychain secret)
```

A script alert watches one applied instance on one chart. Chart ids start over in every tab, so a
subscription names the tab and the chart (`ChartKey`), and `PineSubscriptionHost::reconcile` reads
every open chart at once. When the alert's tab is not open (closed, or a row saved before alerts
named their tab), it moves only to a chart of the same id that runs the same script source on the
same market, which is what opening its saved layout in another tab gives; otherwise it waits. An
active one also waits while another active alert watches that instance there, so no signal is sent
twice (`PineAlertSubscription::arms`, shared with the creation sheet and Re-arm).

An instance keeps the source it was applied with, so editing a script changes no alert; taking the
indicator off the chart switches the alert to "script changed". Re-arm rebinds it
(`PineAlertSubscription::rearm_on`: instance, source, inputs, name) to the instance it was armed on
if that is still there, else to the one copy of the same saved script on its chart. The host decides
this in `reconcile`; `rearm_block` says why it cannot (no copy, several, already watched by another
active alert, chart not open on its market), and the menu shows that as the tooltip. A change of an
alert's source drops its run, with any compile or runtime failure it held.

`AlertHost` is the in-app price-alert owner: it loads the snapshot, drains `alert_command`, evaluates
crossings using caller-supplied time, and publishes triggers/unseen state. There is no separate
login-item agent yet.

Webhook endpoints are opt-in reusable ids. `webhook_policy` validates/redacts URL and header
templates. `webhook_delivery` claims one row per trigger/endpoint before sending, giving once-only
delivery across processes. It performs no retries or redirects and currently uses osascript-backed
HTTP.

The single Keychain secret replaces `{{secret}}` only while building a request: percent-encoded in
the URL and raw in headers. Stored results contain status, duration, and typed error—never resolved
URL, headers, or body.
