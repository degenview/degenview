# Alerts and webhooks guidance

## Alerts

- `AlertHost` owns in-app price alerts: loads persistence, drains `alert_command`, exposes the
  evaluated snapshot. The embedded login-item alert agent (`host/alert_runtime.rs`) runs the same
  host while the app is closed; exactly one process owns evaluation (a file lock).
- iCloud Sync: only an alert's definition syncs; remote changes arrive as `alert_command`s, never
  table writes. An alert synced from another Mac is *muted* here (`price_alert_local`,
  `AlertEngine::set_muted`): defined, not polled or evaluated, so one Mac notifies. Keep runtime
  fields out of `sync::project`'s alert projection.
- Unseen count comes from `alerts.lastSeenAt`, never an event flag (snapshot replacement rewrites
  events). `ui::alert_banner` shows over every workspace.
- New-alert sheets opened from charts/watchlists belong to the requesting `ChartWorkspace` and send
  save/webhook actions back as `WorkspaceAction`s; only Alerts-window edits host the sheet there.
- An open sheet holds `alert_editor::quote_owner` demand and gets the current quote each frame,
  converted by app FX for non-USD currencies.
- The Create Script Alert and script alert webhooks sheets (`ui::pine_alert_editor`) follow the
  price alert sheet's layout: header, armed-item card, row cards, `webhook_picker` for endpoints
  (`send_section` without message for a script alert, `endpoint_picker` alone), hairline,
  summary + Cancel/Create footer. Neutral controls only (`kit::sheet_button`, gray badges). "Add in
  Settings" is an outcome (`OpenWebhookSettings`) the host acts on; the sheet stays open. Pictures:
  `cargo test --lib gallery_of_the_script_alert -- --ignored` with `DEGENVIEW_GALLERY_DIR` set.

## Webhooks

- Endpoints are global `store::webhook` documents. URLs and headers are plaintext templates with a
  `{{secret}}` placeholder, never the secret. Settings ▸ Webhooks shows the address in full, as
  typed. Logs and any text not the user's own configuration go through `webhook_policy::redact`.
- The single Keychain secret enters only at `{{secret}}`: percent-encoded in URLs, raw in headers,
  never in hostnames or bodies. Never log or persist resolved URLs or headers.
- `service::webhook_delivery` is the only transport (currently osascript-backed). Claim the delivery
  row before sending so each trigger reaches each endpoint once. No retries or redirects.
- Persisted alert/webhook fields need serde defaults for missing older fields: undecodable alert
  rows are dropped when the snapshot is rewritten.

Full flow: [architecture](../architecture/alerts-webhooks.md).
