# Chart snapshots

A camera in the toolbar, right of the chart settings button, takes a picture of the focused chart and does one of five things with it,
as TradingView's does: **Download image**, **Copy image**, **Copy link**, **Open in new tab**,
**Tweet image**. The first two work offline. The other three upload the image to a **provider**
the user turns on in Settings ▸ Snapshots (ImgBB, or a host of their own); DegenView hosts nothing,
and nothing is uploaded until a provider is on.

The menu ends with a separator and **Configure Snapshots…**, which opens Settings on the Snapshots
page (`Pick::Configure` → `WorkspaceAction::OpenSnapshotSettings` → `SettingsHost::open_snapshots`).
It is never disabled: it is where a provider is turned on.

## Pipeline

```
click / shortcut ─► ChartWorkspace::begin_snapshot(chart_id, action)           UI thread
    SnapshotRequest { chart_id, action, options }   fixed here; later setting changes don't touch it
    ChartCard::snapshot_scene  ─ a scratch egui Context at the image's scale ─►  paint_snapshot
    ctx.tessellate ─► Scene { meshes, textures }                               (one frame, 6–13 ms)
        │
        ▼ work_queue::SNAPSHOTS (worker, no workspace or store lock)
    snapshot::job::run
      Download   choose destination ─► rasterize ─► PNG ─► atomic write
      Copy image rasterize ─► Outcome::CopyImage
      hosted     rasterize ─► PNG ─► upload ─► Outcome::Hosted
        │
        ▼ channel, polled each frame                                            UI thread
    poll_snapshot: ctx.copy_image / copy_text / open_url, then the message
```

- **One renderer.** `ChartCard::body_inner` and `paint_snapshot` both lay the chart out with
  `frame_layout` and draw it with `paint_frame` (`ui/chart/card/frame.rs`). What only the live chart
  has (drawing selection toolbar, pane buttons, interactive alert tags and paper markers, the
  navigation pill, replay picking) is the `Overlays` hook, called at the same points, so layering
  is unchanged. A snapshot is the live frame minus everything a pointer can reach.
- **Progress.** A hosted action in flight shows a notice over the bottom of the window: the candle
  indicator (`kit::loading`) above "Uploading to {provider}…" (`ui/chart/notice.rs`), and the camera
  button runs a small one. A result replaces it.
- **Consistent frame.** The chart is painted from the card's own state and the workspace's data in
  one UI-thread pass, so no part can come from another moment. The slow work never touches the
  workspace. Live candles arriving later do not change an image already drawn.
- **Read-only.** `paint_snapshot` takes `&self`: it reads each script's last run (never reruns
  Pine), builds constructed bars without caching them, and moves no viewport, pane or replay state.
- **Rasterizer.** `service/snapshot/raster.rs` draws egui's tessellated meshes on the CPU the way
  the wgpu shader does (gamma-space premultiplied blending, bilinear/nearest texture sampling,
  top-left fill rule, 8-bit rounding per draw). No window, GPU or readback is involved, so it works
  anywhere, at any scale, in tests. Text and lines are laid out at the image's scale: 2× is drawn
  again, not enlarged.
- **Sizes.** The image is the chart's plot area (what the card drew last frame, less the strip the
  header covers, `ChartCard::snapshot_size`) at 1× or 2×
  (Settings, default 2×). At most 8192 px a side and 32 Mi pixels (128 MiB RGBA); a larger chart
  is drawn at a smaller scale, and one too large for ¼× is refused. Custom export sizes are not
  offered.

## What is in the image

Everything the chart draws: canvas background (solid or gradient), grid, watermark (including
" · Replay"), the main series of all 18 chart types (constructed bars for price-based ones),
built-in indicators, every script's plots, shapes, labels, lines, boxes, tables, fills and strategy
trades in their layers, script panes with their scales and separators, drawings (visible ones;
hidden, other-timeframe and unreadable ones stay out; no handles or selection), the last-price
line and tag, both axes, and the chart's price alert lines.

The text, note and content tools paint as on screen: boxes and tables, link cards, pictures, and the
note of every pin that is open (`SnapshotInput.open_notes`). A picture is handed to the snapshot already
decoded (frozen on the UI thread, since the scratch context has no store); one that is not ready is
painted as the placeholder the chart shows. A snapshot never reads the store or the network.

Left out: crosshair, hover, selection and drag previews, tooltips, the selection toolbar, pane
buttons, the navigation pill, the replay badge and pick marker, the locked cursor line, the "refresh
failed" warning, the header, and everything outside the chart (title bar, sidebars, menus, the
camera menu itself). Replay draws only the revealed candles.

Not supported: the CoinMarketCap "Latest" gauges (not a plot) and portfolio cards.

### Orders, executions and positions

Settings ▸ Snapshots ▸ *Include orders, executions and positions in snapshots*, **off by default**:
positions and orders show sizes, entry prices and open profit, so sharing a chart must not expose
them unless asked. Even on, they are drawn only if the chart itself shows them. Executions are not
drawn on charts anywhere in the app today, so there is nothing to include.

## Hosted sharing

Hosted actions go to one **provider** at a time, chosen on Settings ▸ Snapshots, where each provider
is a row like the ones on Settings ▸ Brokers: a header (mark, name, how it is doing, a switch, a
chevron) that folds its setup. **None is on by default**; turning one on turns the other off, and a
provider that is on but not set up (no key yet) says what is missing in the camera menu and opens
unfolded. Turning a provider off keeps what was entered for it.

Two providers ship (`src/service/snapshot/providers/`, listed in that order):

| Provider | Id | What it needs | Notes |
| --- | --- | --- | --- |
| ImgBB | `imgbb` | The user's own ImgBB API key (secret), and how long ImgBB keeps an image (1 day to 180 days, or never; default **30 days**) | The link is the image's direct URL. Has a **Test** button |
| Host your own | `self_hosted` | The upload address, and an optional access token (secret) | Speaks the protocol below. <https://github.com/degenview/snapshots> is a server that does |

### A provider is data and two pure functions

Everything else (the upload job, the consent question, the camera menu, the Settings rows, iCloud
Sync, the secret store) works from `ProviderDescriptor` and the `SnapshotProvider` trait, so
**adding a provider is one file and one line in `registry()`**, with no change to the page or the
flow:

1. A `ProviderDescriptor`: `id` (saved, never changes), `name`, `summary`, `logo`, the numbered
   `setup_steps`, `links`, a `notice` (what the user should know before an image goes there), and
   the `fields`. A `FieldSpec` is a `Text`, a `Secret` or a `Choice`, with its hint, whether it is
   required, an environment variable that stands in for a secret, and a `validate` function. The
   row is drawn from this alone. Field titles must be unique across providers (they identify the
   widget); a test enforces it, and the other rules of a consistent descriptor.
2. `impl SnapshotProvider`: `consent_key` (what the user agrees to upload to), `request` (builds the
   HTTP request for a PNG; a `Purpose::Test` upload is a tiny image the provider should not keep),
   `parse` (reads a 200/201 answer), and optionally `readiness`, `label`, `status_line` and
   `classify_failure` (a provider's own error body; the default maps what every host says).
3. Add it to `registry()`.

`service::net::multipart::Form` builds a `multipart/form-data` body for hosts that want one. The
transport (`snapshot::upload`) knows nothing of providers: it checks the size and that the bytes are
a PNG, sends the provider's request once with no redirects, and hands the answer back.

Saved shape (`snapshot.prefs`): `provider` (the id that is on, or an explicit `null` for off),
`providers` (each provider's non-secret settings by id), `include_trading`, `scale`, `consented`.
A save from before providers has one top-level `host`: if it is still a valid address it reads as
`provider = self_hosted` with that address, so nobody loses their setup, and the next change writes
the new shape (an address the page would not have accepted is not carried over). A `host` that
arrives later (an older Mac syncing) is ignored once a provider has been written, so it cannot turn
a provider off or back on. An id this build does not know is kept, inactive; the camera menu says
to choose another, and so does a badge on the page.

**Sync.** `provider` and `providers` sync; consent and secrets never do. A record is applied per
provider (a Mac that never set a provider up cannot erase this Mac's settings for it), and a value
nobody has rewritten since the update still goes out in the shape an older Mac wrote, a top-level
`host`, exactly as stored: reading it as this version does would make a Mac that merely received
it send it back as a new edit. The running window holds the choices in memory, so every change is
made on what is stored (a change synced in meanwhile is not undone by it) and the Settings page
reads them again each second; the camera menu of a window that has not opened Settings or changed a
setting shows a synced provider after the next launch.

### ImgBB

The call is the one ImgBB documents (<https://api.imgbb.com>): a `POST` of the PNG as a binary file
in a multipart form to `https://api.imgbb.com/1/upload`, with the `key` and `expiration` (seconds,
60 to 15552000; left out for "never") **in the query string, as the documentation shows them**. The
key is therefore in the URL: it travels over TLS, and the app never prints, logs or shows a request
URL (transport errors are an enum, not text). The answer's `data.url` is shared (its direct image
link), with `data.delete_url` and, when ImgBB reports a lifetime, when it expires. **Test** uploads a
one-pixel image asking for `expiration=60`, the shortest life ImgBB allows (whether ImgBB then
forgets it is not verified; the image is public until it does). It asks nothing first, and works
while the provider is off: the click is the request.

Verified against ImgBB's documentation: the endpoint, the parameters and their ranges, the success
answer's fields. **Not stated by it and so assumed**: the error answer's shape (read as
`{"error": {"message": …}}`, where a message about the key becomes "refused the credentials";
anything else is read by its HTTP status), and rate limits. Not run against ImgBB with a real key
by the author of this change.

### The protocol of a host of your own

The host is any HTTPS service that implements this protocol (`http://` is accepted for a host on
this computer only, for development).

```
POST <host address>
Content-Type: image/png
Authorization: Bearer <token>        only if a token is set
User-Agent: DegenView/<version>

<PNG bytes, at most 24 MiB>
```

Answer `200` or `201` with at most 16 KiB of JSON (a longer answer is no link; a failure page of any
size is still the failure its status names):

```json
{ "url": "https://…", "delete_url": "https://…", "expires_at": "2027-01-01T00:00:00Z" }
```

Only `url` is required: an `https://` link (no credentials, no spaces, at most 2048 characters)
that anyone can open to see the image. `delete_url` and `expires_at` are optional and informational.
Other answers: `401`/`403` token refused, `413` image too large, `429` slow down (`Retry-After` in
seconds is shown), `5xx` host failure. Redirects are never followed, so the image only goes to the
address the user approved. One request is one attempt; the app keeps the PNG so *Try again* does
not redraw.

A host should: require a token (never an open upload), check the file really is a PNG and its size
and dimensions, store it under a random unguessable name, serve it as `image/png` with
`Content-Disposition: inline` and `X-Content-Type-Options: nosniff`, rate-limit and set quotas,
purge on a schedule it states (TradingView's own links do not expire, but DegenView promises no
permanence: the host decides), log requests without image content, and may offer deletion. DegenView
ships no server; its tests use a mock server on this computer.

### Consent and privacy

- Download and Copy image never touch the network.
- The first upload to a destination asks ("To share this chart, DegenView uploads its image to
  {label}. {the provider's notice}"). The answer is remembered per **consent key** (`api.imgbb.com`,
  or `host[:port]` for a host of the user's own) in the `snapshot.prefs` setting, never synced, and
  can be forgotten. Agreeing to one destination is not agreeing to another. A host of the user's
  own with a saved access token says that the token goes with the image, because the token is not
  tied to the address and a changed address would send it somewhere new.
- Hosted rows are disabled, with the reason, until a provider is on and set up. Nothing is uploaded
  to any default. The one upload that is not a chart is a provider's **Test** button, which sends a
  one-pixel image when the user clicks it.
- A provider's secrets are in the system's secure storage (`es.nico.degenview.snapshots`, account
  `"{provider}.{field}-v1"`; the self-hosted token keeps its older `host-token-v1`), read only on
  the upload's worker thread, or the field's environment variable (`DEGENVIEW_IMGBB_KEY`,
  `DEGENVIEW_SNAPSHOT_TOKEN`). The window never waits for the storage: whether a secret is saved is
  asked on a thread of its own the first time and cached, the camera menu says "Checking your saved
  key" until the answer is in, and the cache is never locked while the storage is. They are never in
  the settings, in SQLite or synced; on Linux they are
  in the Secret Service (see [Flatpak](flatpak.md)), on Windows in Credential Manager (see
  [Windows secret storage](flatpak.md#windows-secret-storage)). Where the store is unavailable one
  cannot be saved: use the environment variable.
- ImgBB links are public to anyone who has them, and ImgBB's terms and privacy policy apply; the row
  says so and links both. Orders and positions stay off the image unless the user includes them.
- Tweet image opens X's composer (`https://x.com/intent/post?text=…&url=<snapshot link>`) in the
  browser. **The image is not attached to the post**, only its link: attaching needs an authorised
  X API integration, which this is not. No X account or credentials are requested.

## Platforms

macOS is the platform this was built and checked on. Download uses the system Save panel
(`osascript`, as the portfolio export does) there and the XDG file chooser portal on Linux
(`rfd`, see [Flatpak](flatpak.md)); Windows saves to the Downloads folder under a free name and
reveals the file (no native panel yet). Copy image
uses egui's clipboard (arboard), which cannot report a failure, so "Image copied" means the copy
was handed to the system.

## Shortcuts

⌥⌘S Download image, ⇧⌘S Copy image, ⌥S Copy link (Ctrl+Alt+S, Ctrl+Shift+S, Alt+S elsewhere),
on the focused chart, unless a text field has the keyboard (⌥S types a letter on a Mac). The menu
shows a shortcut only where one works. "Open in new tab" and "Tweet image" have none.

## TradingView: verified and not

Checked against TradingView's pages (How to share a snapshot, Chart snapshots, How to display
trades on chart snapshots):

- Verified: the camera is at the top of the chart; the menu offers Download image, Copy image,
  Copy link, Open in new tab, Tweet image; link actions need the integrator's own server accepting a
  `POST` and answering with the image URL; TradingView's links do not expire; trades are included
  by a setting, "Orders, executions and positions in screenshots".
- **Not verified**: the keyboard shortcuts above (the pages mention shortcuts but give none; they
  come from the task description), the setting's default, and the menu's exact look (no reference
  screenshot was available, so the layout follows the described structure and the app's own menus).
- TradingView puts the trading setting inside the snapshot menu; here it is in Settings.

## Verifying

`cargo test` covers the model, rasterizer, drift against the live chart (plain, all 18 types,
indicators, script panes, script labels/lines/boxes/shapes/tables, drawings, alerts and the
account, shape for shape), exclusions, no state
mutation, PNG, atomic saving, the upload client against a mock server, the job, the workspace flow
and the menu. `cargo test --release snapshot_cost -- --ignored --nocapture` prints stage timings
(a 500-candle chart with indicators, two script panes, a drawing and the account, release build:
freeze + tessellate 6–13 ms on the UI thread, rasterize 14 ms at 1× and 44 ms at 2×, PNG 11 ms and
28 ms, 91 KiB and 222 KiB files; the 2× image is 6 MiB raw).
Checked by hand on a release build against live Binance data: the camera and menu render, ⇧⌘S puts
a 2724 × 1446 PNG on the clipboard (read back as `PNGf`), a first ⌥S asks before uploading, and
after agreeing the image reaches a local mock host as an `image/png` POST and its link is on the
clipboard; the uploaded PNG holds no crosshair, hover strip or navigation pill although the pointer
was over the chart. Not checked by hand: the Save panel, a real remote host, X's composer, both
themes and window edges, Windows and Linux. To repeat: the camera menu in both themes and near a
window edge, each action on a real chart with
several indicators, a script pane and drawings, with replay, in a second window, and against a
host.

The Settings page and its provider rows are drawn headless by
`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_snapshots_page -- --ignored`, which
writes the page in both themes at the Settings window's width and at its narrowest, with nothing on,
ImgBB needing a key, ImgBB active, a host of your own, and both rows unfolded. The upload client is
tested against a mock server for both providers (the self-hosted wire format is unchanged, asserted
byte for byte; ImgBB's request line, multipart body and error answers). Not checked: ImgBB itself
with a real key, the rows in a real window, and a real Keychain/Secret Service/Credential Manager
round trip of an ImgBB key.
