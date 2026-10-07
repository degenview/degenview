# Webhooks

Alerts can POST their message to external HTTP endpoints, the way TradingView webhooks do. Nothing
is sent until a webhook is configured and an alert selects it.

## Using them

1. **Settings ▸ Webhooks ▸ Add Webhook.** Name, optional description, URL, enabled.
   **Test** sends the test message (default `{"event":"DegenView webhook test"}`, so as JSON) through the same code alerts use, even if the webhook is disabled.
2. **Price alerts:** the alert editor has a Webhooks section: tick endpoints and optionally write the
   message (default `{{ticker}} alert: {{close}}`).
3. **Script alerts:** tick endpoints when creating the alert, or later from the row menu
   (**Webhooks…**). The message is whatever the script's `alert()` / `alertcondition()` produced.
4. The history rows show `Webhooks 2/3 delivered`; click for each endpoint's result.

Disabling an endpoint pauses it everywhere; alerts keep their selection. Deleting one asks first
(with a usage count) and removes its id from every alert that used it.

## HTTP behaviour (TradingView compatible)

| | |
|---|---|
| Method | `POST` by default, configurable per endpoint: `POST` or `PUT`. Both send the message as the body |
| Body | the final rendered message, byte for byte (UTF-8). No envelope, no reformatting, no newline added |
| Content-Type | `application/json; charset=utf-8` if the whole message parses as a JSON object or array, else `text/plain; charset=utf-8`. Placeholders are resolved first |
| Timeout | 3 seconds (`WebhookDeliveryService.requestTimeout`) |
| Success | any `200…299`; everything else, including 3xx, is a failed delivery |
| Retries | none. One attempt per trigger per endpoint, so a slow or failing server can never cause a duplicate order |
| Redirects | never followed (a 3xx is a failure), so a validated URL cannot be redirected somewhere unvalidated |

Deliberate deviations from TradingView: any port is allowed (TradingView: 80/443 only), and
loopback / LAN / private addresses are allowed, because DegenView is a desktop app and local trading
bridges and dev servers are a legitimate target. Only `http` and `https` are accepted; `file:`,
`ftp:`, `javascript:` etc. are rejected. Bare JSON scalars (`123`, `true`) are treated as text.

## Placeholders

`{{ticker}}`, `{{exchange}}`, `{{open}}`, `{{high}}`, `{{low}}`, `{{close}}`, `{{volume}}`,
`{{time}}` (bar open, ISO-8601 UTC), `{{timenow}}`, `{{interval}}` (`1`, `60`, `D`…).

Rendering is a single pass over exact `{{name}}` tokens: unknown names, unclosed `{{`, and
`{{ spaced }}` stay as written, and substituted values are never rescanned. A value DegenView did
not have at trigger time is left as the literal placeholder, never invented.

- **Price alerts** read the candle the quote came from. OHLCV is in the asset's own quote currency
  (not converted to the alert's currency). `{{interval}}` is the polling candle (1m, 1h for Alpaca),
  not a chart timeframe.
- **`alertcondition()`** messages are templates, resolved when the condition fires.
- **`alert()`** messages are not substituted: the script already built the final text.
- Not supported: `{{plot_N}}`, `{{strategy.*}}`.

## Secrets and headers

A webhook can have **one secret** (an API token or key), kept in the macOS Keychain, and any number of
**custom headers**. You place the secret with the token **`{{secret}}`** (spacing and case are
forgiven: `{{ Secret }}`), in the URL or in any header value:

| Where | Example | What is sent |
|---|---|---|
| URL query | `https://example.com/hook?token={{secret}}` | `?token=` followed by the percent-encoded secret |
| URL path | `https://example.com/hook/{{secret}}` | the percent-encoded secret as one path segment |
| Header | `X-AUTH-TOKEN: {{secret}}` | the secret as is |
| Header | `Authorization: Bearer {{secret}}` | `Bearer ` and the secret as is |

You rarely need to type it: **Use secret** in the editor adds `?token={{secret}}` to the URL, a Bearer
`Authorization` header, an `X-API-Key` header or a blank custom header. A preview under the URL shows
the address with the secret masked, and a line under the secret says where it is used. If a header that
looks like a credential (`Authorization`, `*key*`, `*token*`…) holds a plain value, the editor offers
**Move to Secret** so it ends up in the Keychain instead of the database.

- **Encoding.** In the URL the secret is percent-encoded with the RFC 3986 unreserved set only
  (`A-Z a-z 0-9 - . _ ~`), so `/ ? & = # + %`, spaces and non-ASCII are all escaped and the one rule is
  right in a path, a query value or a fragment. In a header it is not encoded.
- **Not allowed:** the secret in the scheme, user info, host or port (`secretInHost`); in the message
  body (`{{secret}}` in an alert message stays literal); a secret containing line breaks or control
  characters; a secret with non-ASCII characters used in a header.
- **Header names** must be valid HTTP tokens (letters, digits and ``! # $ % & ' * + - . ^ _ ` | ~``), up
  to 64 characters, unique ignoring case, at most 10 headers. DegenView owns `Content-Type`,
  `Content-Length`, `Host`, `Connection`, `Transfer-Encoding`, `Expect`, `Upgrade`, `TE`, `Trailer` and
  `Keep-Alive`; they can't be set. Values are printable ASCII, up to 2048 characters.
- **Storage.** The secret is one Keychain item per webhook (`com.cryptocharts.webhook`, shared access
  group). The URL and header templates, which only contain `{{secret}}`, are in the `webhook_endpoint`
  row. The agent reads the Keychain per delivery and never prompts: a missing or unreadable secret is a
  failed delivery (`secretUnavailable`). **Debug builds have no Keychain access group**, so the agent
  cannot read a secret the app stored: price-alert webhooks that use a secret need a Release build
  from the agent (the app still delivers when it owns the runtime).
- Headers sit collapsed in the editor until needed, and open automatically when a webhook already has
  some.

## Safety

- **Where the URL lives.** In plain text in the `webhook_endpoint` row of `degenview.sqlite`, next to
  the endpoint's name, so the login-item agent can read it in every build. Anyone who can read that file
  can read the URLs, so put tokens in the **secret** (Keychain), not typed into the URL.
- **Redaction.** Rows and logs show `https://host/•••`; the editor shows the full URL as plain text. Log
  lines carry only the endpoint id, status, error kind and duration. The resolved URL and header values
  (which contain the secret) are never logged or stored. `webhook_delivery` stores no URL, header,
  payload or response body.
- **Unreadable config is not empty config.** If `webhook_endpoint` cannot be read, the store
  disables edits and delivery is skipped; nothing is written over it.

## Who sends

- **Price alerts:** only the process holding `alert_runtime.lock` (agent, or the app as fallback).
  `AlertStore` never sends. Before each request a row is claimed in `webhook_delivery` (unique per
  source, trigger and endpoint), so even a handoff between processes cannot send twice.
  Catch-up triggers and triggers older than 60 s are never sent. The master "Price Alert Delivery"
  switch silences webhooks too; "macOS Notifications" does not.
- **Pine alerts:** `WebhookPineAlertChannel` behind `PineAlertDispatcher`, after the router,
  frequency guard and persisted dedupe key. It adds no frequency rules. Historical bars never reach
  it (`PineExecutionUpdate.alerts` is realtime only, and the router filters again). Pine scripts
  never contain URLs.
- Delivery runs off the evaluation path in its own tasks; a slow server cannot stall alerts. If the
  process quits mid-request the request may be lost and is never replayed.

## Known gaps

- Strategy order-fill alerts, `alert_message` on `strategy.*`, `{{strategy.*}}` and
  `//@strategy_alert_message`: the broker emulator has no fill events yet.
- Script alerts are per indicator, not per `alertcondition()`, so one subscription covers every
  `alert()` / `alertcondition()` call in the script.
- `alertcondition()` placeholders now resolve in every channel (notification and banner text too).
- `freq_all` script alerts have no dedupe key by existing design, so a webhook on one posts on
  every realtime execution that raises it.
