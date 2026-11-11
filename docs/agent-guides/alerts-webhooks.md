# Alerts and webhooks guidance

## Alerts

- `AlertHost` is the in-app price-alert owner: it loads persistence, drains `alert_command`, and
  exposes the evaluated snapshot. The embedded login-item alert agent (`host/alert_runtime.rs`) runs the
  same host while the app is closed; exactly one process owns evaluation (a file lock).
- iCloud Sync: only an alert's definition syncs. Remote changes arrive as `alert_command`s, never
  table writes. An alert synced from another Mac is *muted* here (`price_alert_local`,
  `AlertEngine::set_muted`): defined but neither polled nor evaluated, so it notifies from one
  Mac. Keep runtime fields out of `sync::project`'s alert projection.
- Unseen count comes from `alerts.lastSeenAt`, never an event flag; snapshot replacement rewrites
  events. `ui::alert_banner` appears over every workspace.
- New-alert sheets opened from charts/watchlists belong to the requesting `ChartWorkspace` and send
  save/webhook actions back as `WorkspaceAction`s. Only Alerts-window edits host the sheet there.
- While open, a sheet holds `alert_editor::quote_owner` demand and receives the current quote each
  frame, converted by app FX for non-USD currencies.

- The Create Script Alert sheet and the script alert webhooks sheet (`ui::pine_alert_editor`) are laid out
  like the price alert sheet: header, a card for what is armed, row cards, `webhook_picker` for the
  endpoints (`send_section` with no message for a script alert, `endpoint_picker` alone), hairline,
  summary + Cancel/Create footer. Neutral controls only (`kit::sheet_button`, gray badges). "Add in
  Settings" is an outcome (`OpenWebhookSettings`) the host acts on; the sheet stays open. Pictures:
  `cargo test --lib gallery_of_the_script_alert -- --ignored` with `DEGENVIEW_GALLERY_DIR` set.

## Webhooks

- Endpoints are global `store::webhook` documents. URLs and headers are plaintext templates with a
  `{{secret}}` placeholder, never the secret. Settings ▸ Webhooks shows the address in full, as the
  user typed it. Logs and any text that is not the user's own configuration go through
  `webhook_policy::redact`.
- The single Keychain secret enters only at `{{secret}}`: percent-encoded in URLs, raw in headers,
  never in hostnames or bodies. Never log or persist resolved URLs or headers.
- `service::webhook_delivery` is the only transport. Claim the delivery row before sending so each
  trigger reaches each endpoint once. There are no retries or redirects; transport is currently
  osascript-backed.
- Persisted alert/webhook fields must tolerate missing older fields with serde defaults. Existing
  behavior drops undecodable alert rows when the snapshot is rewritten, so compatibility matters.

See [architecture](../architecture/alerts-webhooks.md) for the full flow.
