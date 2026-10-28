# Chart snapshots

A camera in each chart's header takes a picture of that chart and does one of five things with it,
as TradingView's does: **Download image**, **Copy image**, **Copy link**, **Open in new tab**,
**Tweet image**. The first two work offline. The other three upload the image to a host the user
sets in Settings ▸ Snapshots; DegenView hosts nothing.

## Pipeline

```
click / shortcut ─► ChartWorkspace::begin_snapshot(chart_id, action)           UI thread
    SnapshotRequest { chart_id, action, options }   fixed here; later setting changes don't touch it
    ChartCard::snapshot_scene  ─ a scratch egui Context at the image's scale ─►  paint_snapshot
    ctx.tessellate ─► Scene { meshes, textures }                               (one frame, 6–13 ms)
        │
        ▼ work_queue::SNAPSHOTS (worker, no workspace or store lock)
    snapshot_job::run
      Download   choose destination ─► rasterize ─► PNG ─► atomic write
      Copy image rasterize ─► Outcome::CopyImage
      hosted     rasterize ─► PNG ─► upload ─► Outcome::Hosted
        │
        ▼ channel, polled each frame                                            UI thread
    poll_snapshot: ctx.copy_image / copy_text / open_url, then the message
```

- **One renderer.** `ChartCard::body_inner` and `paint_snapshot` both lay the chart out with
  `frame_layout` and draw it with `paint_frame` (`ui/chart_card/frame.rs`). What only the live chart
  has (drawing selection toolbar, pane buttons, interactive alert tags and paper markers, the
  navigation pill, replay picking) is the `Overlays` hook, called at the same points, so layering
  is unchanged. A snapshot is the live frame minus everything a pointer can reach.
- **Consistent frame.** The chart is painted from the card's own state and the workspace's data in
  one UI-thread pass, so no part can come from another moment. The slow work never touches the
  workspace. Live candles arriving later do not change an image already drawn.
- **Read-only.** `paint_snapshot` takes `&self`: it reads each script's last run (never reruns
  Pine), builds constructed bars without caching them, and moves no viewport, pane or replay state.
- **Rasterizer.** `service/snapshot_raster.rs` draws egui's tessellated meshes on the CPU the way
  the wgpu shader does (gamma-space premultiplied blending, bilinear/nearest texture sampling,
  top-left fill rule, 8-bit rounding per draw). No window, GPU or readback is involved, so it works
  anywhere, at any scale, in tests. Text and lines are laid out at the image's scale: 2× is drawn
  again, not enlarged.
- **Sizes.** The image is the chart's plot area (what the card drew last frame) at 1× or 2×
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

The host is any HTTPS service that implements this protocol (`http://` is accepted for a host on
this computer only, for development).

```
POST <host address>
Content-Type: image/png
Authorization: Bearer <token>        only if a token is set
User-Agent: DegenView/<version>

<PNG bytes, at most 24 MiB>
```

Answer `200` or `201` with at most 16 KiB of JSON:

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
- The first upload to a host asks ("To share this chart, DegenView uploads its image to <host>…").
  The answer is remembered per `host[:port]` in the `snapshot.prefs` setting and can be forgotten.
- Hosted rows are disabled, with the reason, until a host is set. Nothing is uploaded to any
  default.
- The token is in the Keychain (`es.nico.degenview.snapshots`), read only when an upload needs it,
  or `DEGENVIEW_SNAPSHOT_TOKEN`. On Linux it is in the Secret Service (see [Flatpak](flatpak.md)). Where there is no secret store
  (Windows) it cannot be saved: use the environment variable or a token-less host.
- Tweet image opens X's composer (`https://x.com/intent/post?text=…&url=<snapshot link>`) in the
  browser. **The image is not attached to the post**, only its link: attaching needs an authorised
  X API integration, which this is not. No X account or credentials are requested.

## Platforms

macOS is the platform this was built and checked on. Download uses the system Save panel
(`osascript`, as the portfolio export does) there; Windows and Linux save to the Downloads folder
under a free name and reveal the file (no native panel: that needs a new dependency). Copy image
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
