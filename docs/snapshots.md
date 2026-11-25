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
    ctx.tessellate ─► Scene { meshes, textures }                               (one frame, 4–9 ms)
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
- **Overrides are the snapshot's alone.** `SnapshotInput.frame` (`SnapshotFrame`) lets a caller draw
  other bars and prices (`viewport`, which may show fewer bars than a chart ever does), script
  outputs frozen earlier (`ChartCard::snapshot_outputs`), no indicators or script outputs at all
  (`include_indicators: false`, panes included), and a `Restyle` of the canvas (no grid or
  watermark, larger axis text). `Default` is the live frame. They are drawn instead of the card's
  own state, which is never written: the live viewport, panes and script extent stay as they were,
  and the scripts' reach past the last bar is recomputed for the viewport drawn.
  `snapshot_geometry` returns the bars, prices and plot rectangle `paint_snapshot` would lay out
  for a given size and scale (the same code path), and `ViewGeometry::viewport_for` turns a
  rectangle on that plot into the viewport that shows it, as Zoom In does but without the chart's
  20-bar minimum. `ui/chart/snapshot_scene.rs` is the one scratch-context scene builder
  (`render`); a caller adds a logo or an icon by putting it, decoded, in the `images` it passes
  and reading it back with `asset_host::lookup`.
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
line and tag (with the countdown to the bar's close, frozen at the capture: `ChartParts::countdown`), both axes, and the chart's price alert lines.

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
`providers` (each provider's non-secret settings by id), `include_trading`, `scale`, `studio` (the
Screenshot Studio's look, [below](#the-look-is-remembered)), `consented`.
A save from before providers has one top-level `host`: if it is still a valid address it reads as
`provider = self_hosted` with that address, so nobody loses their setup, and the next change writes
the new shape (an address the page would not have accepted is not carried over). A `host` that
arrives later (an older Mac syncing) is ignored once a provider has been written, so it cannot turn
a provider off or back on. An id this build does not know is kept, inactive; the camera menu says
to choose another, and so does a badge on the page.

**Sync.** `provider`, `providers`, `include_trading`, `scale` and `studio` sync; consent and secrets never do. A record is applied per
provider (a Mac that never set a provider up cannot erase this Mac's settings for it), and a value
nobody has rewritten since the update still goes out in the shape an older Mac wrote, a top-level
`host`, exactly as stored: reading it as this version does would make a Mac that merely received
it send it back as a new edit. The running window holds the choices in memory, so every change is
made on what is stored (a change synced in meanwhile is not undone by it) and the Settings page
reads them again each second; the camera menu of a window that has not opened Settings or changed a
setting shows a synced provider after the next launch, and a Studio opens in the look the app
holds, which is the one stored at launch, the last one saved here, or what the Settings page read
since.

`studio` is portable like `scale`, and follows the same rules. A Mac applies each field a record
carries and keeps its own for one the record does not: an older Mac, which does not know `studio`,
sends records without it and so never erases it. A value that nobody has written a `studio` into
projects exactly as it did before there was one, so installing this version is not an edit; a
`studio` that came from another Mac is stored as it arrived and projects to the same record, so it
is not sent back as a new edit. The older shape (a top-level `host`) never carries one. Each part
of the look is read by itself: a choice a newer version added (a backdrop this one has not got) is
read as that part's default, and never takes the provider or the other parts with it.

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

## Screenshot Studio

The Studio restyles, reframes and exports a picture of a chart or of the whole layout. It opens
from **Select portion** (the camera menu's third row, below) and is a window of the workspace
(`ui/snapshot_studio/window/`, run by `workspace/chart/snapshot/studio/`) over a **composer**
(`ui/snapshot_studio/compose/`), which is a presentation layer around `paint_snapshot` and never a
second chart painter.

```
Capture (frozen once)  +  StudioConfig  ─►  compose::paint ─► Scene ─► raster
   chart(s): candles, drawings,             (one function)    Target::Preview { max_px }  scale to fit, at most 2×
   script outputs, pictures, logo,                            Target::Export(scale)       composition::dimensions
   size, base view                                            the same shapes, another pixels_per_point
```

- **A capture is frozen.** `ChartCapture::new(card, ChartParts, pixels_per_point, portion)` owns
  what the frame reads that can change (the candles cut at the replay, drawings, indicators,
  `FrozenOutputs`, pictures, the logo as decoded pixels) and the **base view**
  (`ViewGeometry::framing_base`): where the capture's plot started, which the Width and Height
  dials and the drag measure from. A portion is a capture the size of the selection that shows the
  bars and prices the selection covered (`viewport_for`), so its base is that view. The card itself is
  only read (alert lines, paper markers and canvas come from it live); a test pins that composing
  leaves it as it was. `Capture::Layout` holds each chart with its rectangle in layout points, and
  `crop` for a selection across charts.
- **One paint function.** `compose::paint(painter, rect, capture, cards, config)` is the only thing
  that lays the image out, and `compose::scene` runs it on a scratch context at the target's scale
  (`snapshot_scene::render`). Layout is in points, text is laid out at the scene's scale, so the
  preview and the export are the same shapes at another `pixels_per_point` (a test compares the shape
  dumps of one painting at 1× and 2×; labels may differ by a fraction of a point).
- **Order:** backdrop (only with a frame, `Background::has_frame`), soft shadow, the chart(s)
  through `paint_snapshot` with `SnapshotFrame { viewport: framing, outputs: frozen,
  include_indicators, restyle }` and `hide_all_drawings` for Include, the rounded corners, the
  Styled title block, the DegenView mark. The chart is painted into `Composition::body()` (below the
  title strip). Its view is `Framing::viewport(base, plot)`, refitted (at most three rounds) to the
  plot the card really lays out there, since the axis width and pane heights are the card's to decide.
- **Backdrops** (`compose/background.rs`) are pure functions of position, `color_at(background, uv)`:
  DegenView's own palettes (Luminous deep teal with a cyan glow, Radiant pink to orange, Spectrum purple
  through magenta to teal, Illuminated deep blue with a bright glow, Dots charcoal with dots every
  14 pt). A gradient is a 96 × 64 vertex-coloured grid (cells under the chart are skipped); dots are
  circles on a fixed grid in points, so they are sharp at any scale. `swatch(background)` says what
  the Studio's swatch row paints, from the same definitions.
- **Rounded chart** (`compose/surround.rs`): the chart is painted square and four corner pieces (radius
  10 pt) are laid over it, coloured by `Surround::surface_at`, the same function the backdrop and
  the shadow are painted from, with a one-pixel feathered inner edge, so there is no seam. The shadow
  is a mesh with its own falloff (not `epaint::Shadow`) for exactly that reason: the corners need to
  know how dark it is. Softness and offset are shares of the margin, so it scales with the image.
- **Styled** (`compose/styled.rs`): the canvas is restyled (`Restyle`: no grid, no watermark, axis text
  at 1.35×). The title block fills the strip with the canvas' top colour and sets the market's logo
  (a monogram until the icon cache has one), its name in bold and `EXCHANGE · timeframe`. Its
  colours follow the canvas, not the app's theme: near black on a light canvas, near white on a dark
  one, the axis text colour for the subline. The mark (the app icon in a circle and "DegenView",
  at 55 % opacity) is at the right end of the title strip, centred on it, so it is never over market
  data (`composition::branding` reserves its place). The market's name always comes first: it is cut
  only when it alone is wider than the strip. The mark takes what is left of the strip, in full if it
  fits beside the title block, as its icon alone if only that does, and not at all otherwise.
- **Layouts** (`compose/layout.rs`): each chart's rectangle is mapped onto the body (a different
  proportion stretches the arrangement; each chart is refitted to its plot, never stretched) and
  painted clipped to its own rectangle, with a hairline on every edge two charts share. Framing is
  not applied. A portion across charts is the same picture with the layout mapped from the
  selection's origin and everything outside clipped, still vector. The title block names the focused
  chart.
- **Price fit.** Horizontally a bar keeps the width it had in the capture (a wider plot shows more
  bars). Vertically the capture's starting price range fills the composition's actual plot height at
  Height 1× whatever the size, so the last-price tag and the newest candle the capture showed are never
  cropped; Height 2× shows half that range (a decision, `framing`; the earlier rule of keeping the
  price per point cropped the range when the title strip made the plot shorter).
- **Known limits.** In a size other than the capture's the candles are taller or shorter than in the
  capture (the price scale is fitted, not kept). Not checked: the logo from the icon cache in a real
  window (tests use the monogram), a layout in a real window.

### Select portion

The camera menu's **Select portion** row (after Copy image, before the hosted rows; no shortcut; it
waits while a snapshot is being made like the others) starts a mode of the workspace, modelled on Zoom
In but owned by the snapshot state (`SnapshotState::selecting`, `workspace/chart/snapshot/select.rs`).
It is the workspace's, not a chart's, because the rectangle may begin on any market chart and cross
into the next.

- **Entering** disarms the drawing tools and Zoom In (without dropping rulers: a measurement may be
  what is being captured); arming a tool or Zoom In afterwards puts the selection down (checked each
  frame in `render_grid`, so every way of arming is covered). Replay start picking waits.
- **Interaction.** A primary press inside any market chart's body (header strip included) begins the
  rectangle; it follows the pointer in any direction and is kept inside the union of the charts'
  snapshot frames (`selection::dragged`). **One press and release is the whole mode**: a release of
  at least 24 pt a side (`selection::released`) captures, a smaller one or a plain click ends the mode
  and captures nothing, silently.
- **Cancel.** Escape (spent on the selection: the workspace's own Escape chain does not also act),
  a second mouse button, the window losing focus, or a skipped frame (another tab was shown) end it.
- **What stands back.** A layer above the grid and below menus and sheets
  (`ui/workspace/portion_overlay.rs`) takes the pointer from everything under it, so headers, alert
  pills, context menus, axes and overlays do not answer. The cards also stand down
  (`ChartCard::set_snapshot_select`, through `pointer_is_held`, the check Zoom In shares): no pan,
  wheel, axis scale, drawing hover, pick or placement, and no crosshair. The cursor is a crosshair.
- **Painting.** Before the drag the grid is lightly dimmed with the hint "Drag to select an area · Esc
  to cancel" at the top of the plots. While dragging, outside the rectangle is dimmed (about 35% black
  in the dark theme, less in the light one) and the rectangle has a 1 pt border in the label colour,
  snapped to the pixel grid. Flat: no animation, no shadow, nothing blue.
- **Nothing is written.** No undo step, no layout save or dirty flag, no drawing, no change of any
  chart's view; the freeze below only reads. Tests pin each.
- **What a release freezes** (`capture_portion`, on the UI thread). The charts' data is read as
  `capture_scene` reads it (`ChartParts`, shared: candles cut at the replay, drawings, indicators, open
  notes, pictures, paper markers per the Settings, and the market's logo if the icon cache has it);
  drawings are left out while all are hidden. The result is `SnapshotState::studio`, a `StudioSession`
  holding the `Capture` and a `StudioConfig` that opens the Studio window ([below](#the-studio-window)).
  - *Within one chart* (the selection overlaps only that chart's snapshot frame): `Capture::Single`, a
    `ChartCapture` with the selection counted from the frame's top-left corner as its portion. The
    snapshot frame is the body less the header strip, and a point of it is the same point of the
    snapshot (`ChartCard::snapshot_frame`). The capture clamps the part of the portion on the plot to
    the bars and prices it covers; the axes are always drawn, so a selection that takes in an axis
    keeps it and one that does not still shows it, inside the same size. Panes are laid out a few points
    differently in the snapshot than under the header, so the match is to the point only without them.
    A sliver under 24 pt of the frame is no selection.
  - *Across charts* (the selection overlaps the frames of several): `Capture::Layout` of the charts it
    reaches, each at its frame's place relative to the corner of the union of all the market charts'
    frames, with the selection as `crop`; `focused` is the focused chart if it is among them, else the
    first. A selection that reaches no chart (only gaps and headers) says so.
- **Not checked.** Everything above is driven headlessly (pointer events over the real grid, the
  overlay in pictures of both themes); not exercised in a real window.

### The Studio window

A modal sheet (`kit::sheet`) over the workspace: as wide as the screen allows up to 1120 pt and 92% of
it, as tall as 92% of the screen up to 900 pt; the preview takes what the rest leaves (a chart
window's smallest size, 960 × 620, leaves it about 280 pt). The window is immediate mode
(`window::show(ctx, id, &Input) -> Output`): it is given the configuration, the latest picture and
a few flags, keeps no state of its own (egui remembers focus and a drag in progress), and answers
with `Action`s, which the workspace applies (`StudioSession`, `apply_studio_action`). Layout, top
to bottom: the title and the close button; the **control groups**; the preview in a rounded well;
the **Width** and **Height** rulers with **Reset view**; the footer.

- **Area**: *Full chart* | *Portion*, and *Entire layout* between them when the workspace has more
  than one market chart. *Portion* goes back to the charts to select again and keeps the look
  (the look is remembered, [below](#the-look-is-remembered), so the next capture opens with it, the
  framing reset; a selection that is cancelled changes nothing). *Full chart* (the focused chart) and *Entire layout* take a new
  frozen picture of the live workspace (`capture_full`, `capture_layout`) and reset the framing.
- **Style**: *Chart* | *Styled*. The first Studio opens **Styled at 3:2** (Vela's documented images
  show Styled; `StudioPrefs::default`), every later one in the look it was last left in. Switching
  to Styled sets 3:2.
- **Size**: Original, 16:9, 3:2, 1:1, 9:16; disabled in Chart (the control shows Original, dimmed,
  with the tooltip "Size applies to the Styled look").
- **Background**: six swatches painted from `background::swatch`, the composer's own definitions
  (None a slashed circle, Dots a dotted disc, four gradient discs); the selected one has a ring;
  hover says the name.
- **Include**: three icon toggles. Indicators (disabled, "No indicators on this chart", when the
  capture has none), Drawings (disabled, "No visible drawings") and the DegenView mark (only in
  Styled, a toggle we added). Where a group row does not fit the window the groups wrap to a
  second line (`toolbar::lines`).
- **Rulers** (`kit::ruler`, generic): a pill of evenly spaced ticks that slides under a fixed
  centre marker, the value written beside it (`1.28×`, `1×`; `framing::multiplier_text`), from 0.5×
  to 2×. Dragging
  the ticks LEFT raises the value; letting go within 0.04 of 1× lands on 1×. Focused: Right/Up
  and Left/Down step 0.02, Page Up/Down 0.1, Home or 0 or a double-click put it at 1×. The widget
  reports events and keeps no value; it is a slider to a screen reader. Disabled, with the tooltip
  "Not available for several charts", for a layout.
- **Preview**: the latest picture, fitted into the well, crisp (it is asked for at the well's size
  in pixels, so a texel is a pixel). Dragging moves the picture inside the frame
  (`Framing::dragged`: the pointer's travel in points of the image, measured from the framing when
  the drag began; right shows earlier bars, down higher prices; the rubber band and the limits are
  the model's, and release settles). When focused the arrows move it a bar or 2% of the shown
  prices (`KEY_BARS`, `KEY_PRICE`), Home or 0 or a double-click put the position back (the dials
  stay). A grab cursor. Nothing of it works on a layout, which says so on hover. The plot the drag
  is measured against is an estimate (`composition::plot_in`: the capture's share of axes and
  panes applied to the composition's body), since the real one is the card's to lay out when it
  paints; it is within a bar or two.
- **Reset view**: shown only while the framing is not the default; resets the dials and the
  position and nothing else.
- **Footer**: the summary (`Selected portion · Styled · 3,884 × 2,557 · PNG`) from the export's
  real dimensions at the Settings' scale, or the reason there is none (`ComposeError::TooLarge`,
  in the warning tone, with both buttons disabled); **Copy image** (our addition, secondary) and
  **Download PNG** (`kit::neutral_button`: a near-white plate in the dark theme, near-black in the
  light one; never the accent).
- **Escape and the close button** close: the session, its textures and any picture in flight are
  dropped. Closing never touches a chart. The Studio also ends when a chart it holds goes away. A
  click on the dimmed area does not close it (a deviation from the other sheets: a stray click
  should not lose a framing).
- **The workspace waits.** While the Studio is open the shortcuts (the snapshot ones included,
  Ctrl+K, undo, tool keys), the clipboard keys, Escape's own chain and the drawing layer's keys do
  not act on the charts under it (`studio_open()` guards `handle_keyboard`, `handle_clipboard`,
  `dialog_open` and the grid's Escape); the modal takes the pointer.

#### The picture pipeline

`ui/snapshot_studio/previews/` holds no workspace and no thread. Each frame the window reports the
room the preview has in pixels; the workspace asks for a `Key` (the configuration, that room,
and `coarse` while a drag or a ruler drag is under way). A change is a new **generation**. The scene
is composed on the UI thread (`compose::scene(.., Target::Preview { max_px })`, 4 to 5 ms) and
rasterized on `work_queue::SNAPSHOTS` through a `Pending`; **at most one raster job is in flight**,
the newest wanted key waits and is started when it ends (**latest wins**, overtaken keys are never
composed), and a picture is uploaded into one `TextureHandle` that every later picture replaces, so
the old one stays on screen until a newer one arrives. A picture is shown if it is newer than the
one shown, which means a drag is followed by the pictures that finish while it goes on and not only
by its last; pictures of before a restart (the capture was replaced by Full chart or Entire layout)
are dropped. While `coarse` the picture is drawn at half the pixels each way (about four times
fewer pixels, [timings](#verifying)) and the full one follows the release. No Pine runs, nothing
is fetched, nothing written.

#### Export

Download PNG and Copy image compose the scene at `Target::Export(scale)` and go through the same
path as the camera menu: `launch` → `job::run(Download | CopyImage, &scene, &name, backend)` on
the snapshot worker, the same `Running` state and `poll_snapshot` messages ("Saved …" and "Image
copied" only after the job reports success, nothing for a cancelled Save panel, the existing
error notice for a failure). The Studio stays open. The name is `snapshot::file_name(symbol,
timeframe, now, offset)`, or `DegenView_Layout_<date>_<time>.png` for a layout. While an export of
the Studio's runs both buttons are disabled and the one pressed shows the candle indicator. The result notice (`ui::chart::notice`) is drawn in the tooltip layer, above the window, so "Saved …" is not hidden by it. If the
Studio is closed meanwhile the job finishes and its notice still shows (a decision: a file may
well be written after a Save panel was answered).

#### The look is remembered

`SnapshotPrefs.studio` (`StudioPrefs { style, size, background, include }`, in `snapshot.prefs`)
holds the look the next Studio opens in; a first one, and an older save with no `studio`, opens
Styled, 3:2, no backdrop, every Include on (`StudioPrefs::default`, which `StudioConfig::new`
builds from).

- **Opening.** A selection builds its `StudioConfig` from the stored look
  (`StudioConfig::remembering(Area::Portion, look)`): the area is how the Studio was opened and
  the framing is the default. Full chart and Entire layout inside the Studio keep the session's
  look.
- **Saving.** After the actions of a frame are applied, the session compares its look with the
  one it last saved (`StudioSession::remembered`); only a difference calls
  `SnapshotHost::set_studio`, which reads what is stored first (as every setter does, so a change
  synced in meanwhile survives) and writes nothing if the stored look is already this one. A frame
  that changes nothing costs one comparison. A save that fails says so once ("Could not save the
  snapshot settings: …") and is not retried every frame; the Studio stays usable and the look
  stays in the window's memory.
- **Never stored:** the area, the framing (dials and position), the selection, the capture and its
  data.
- **Include and what the capture has.** A toggle that is dimmed because the capture has nothing
  to switch (no indicators, no visible drawings, the mark outside Styled) cannot be clicked, so
  the stored choice for it is carried through untouched and applies again to a capture that does
  have them.
- **Windows share it** (one `SnapshotHost`). A look synced in from another Mac reaches the open
  windows when the Settings page next reads the settings, or at launch; a save from a Studio is made
  on what is stored, so it never undoes a provider that was synced in meanwhile.

#### Verified and decided

- Verified (from Vela's docs and images): the groups and their order, Chart and Styled with Size
  disabled in Chart, the backgrounds and their names, Include's availability rules, the
  dials' range, step, snap and reset, Reset view appearing after a change, the preview's limits and
  rubber band, the footer's summary and a prominent download button.
- Our additions: **Copy image** and the **DegenView mark** toggle.
- Decided: the first Studio opens Styled; a drag of the ruler to the left raises the value (documented
  by Vela for the rulers; the preview is dragged like content, the other way); the arrow steps on
  the preview; half-resolution pictures during a drag; the backdrop does not close the window; the
  look is remembered (and synced) but never the framing.
- Shortcuts are unchanged: the Studio has none, and the existing ones wait while it is open.
- **Not checked**: everything above was driven headlessly (pointer and keyboard events over a real
  grid and the window in pictures of both themes at the default and the smallest window size);
  nothing has been used in a real window, the Save panel and the clipboard were stood in for, and
  the logo from the icon cache and a real layout of several charts were not seen in the window.


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
(a 500-candle chart with indicators, two script panes, a drawing and the account, release build on
an Apple M2 Max; the first run of a process pays for fonts and is the slower UI-thread figure):

| Camera menu image | Freeze + tessellate (UI thread) | Rasterize (worker) | PNG | File | Raw |
| --- | --- | --- | --- | --- | --- |
| 760 × 520 (1×) | 9 ms | 11 ms | 9 ms | 92 KiB | 1 MiB |
| 1520 × 1040 (2×) | 4 ms | 40 ms | 28 ms | 226 KiB | 6 MiB |

Checked by hand on a release build against live Binance data: the camera and menu render, ⇧⌘S puts
a 2724 × 1446 PNG on the clipboard (read back as `PNGf`), a first ⌥S asks before uploading, and
after agreeing the image reaches a local mock host as an `image/png` POST and its link is on the
clipboard; the uploaded PNG holds no crosshair, hover strip or navigation pill although the pointer
was over the chart. Not checked by hand: the Save panel, a real remote host, X's composer, both
themes and window edges, Windows and Linux. To repeat: the camera menu in both themes and near a
window edge, each action on a real chart with
several indicators, a script pane and drawings, with replay, in a second window, and against a
host.

The Studio's composer is covered by `ui/snapshot_studio` tests (shape equality at two scales, pixel probes
of every backdrop and the rounded corners, Include and Styled, dimensions, framing, layouts, no
mutation), its window by `ui/snapshot_studio/window` (every control by pointer and keyboard
events, disabled states and their tooltips, the smallest window) and by `kit::ruler`, its pipeline
by `previews` (one job in flight, latest wins, restart) and its workspace side by
`workspace/chart/snapshot/studio/tests.rs` (a real selection over a real grid, the areas, Portion
and back, the picture arriving and replaced in one texture, half size during a drag, the export
to a PNG of the footer's size, the live charts untouched, the keys waiting).
`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_studio -- --ignored` writes
every backdrop in the Chart and Styled looks and every size on a dark and a light canvas, plus layouts and
portions, and (`gallery_of_the_studio_window`, with `DEGENVIEW_GALLERY_FILTER=<part of a name>` to
write some) the whole window in both themes at the default and smallest sizes in every state.

`cargo test --release studio_cost -- --ignored --nocapture` prints the Studio's stage timings (a
500-candle chart with indicators, a script pane and a drawing, 3:2 on Spectrum, release build on an
Apple M2 Max). The compose step runs on the UI thread, the raster and the PNG on the snapshot
worker; the freeze happens once, when the capture is taken (1.3 ms here).

| Studio picture | Compose + tessellate (UI thread) | Rasterize (worker) | PNG | File |
| --- | --- | --- | --- | --- |
| Preview, 1599 × 1066 (full) | 3.7–4.8 ms | 74 ms | not made | |
| Preview, 799 × 533 (half, while dragging) | 3.5–4.2 ms | 20 ms | not made | |
| Export at 2×, 1710 × 1140 | 4.0 ms | 88 ms | 42 ms | 279 KiB (7 MiB raw) |

The half-size picture holds a quarter of the pixels and rasterizes in about a quarter of the
time, which is what lets a drag keep up; the compose step costs the same at either size, since it
makes the same shapes. An export is the preview's work at another scale plus the PNG, about 130 ms
on the worker after the click, with the window never waiting.

The Settings page and its provider rows are drawn headless by
`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_snapshots_page -- --ignored`, which
writes the page in both themes at the Settings window's width and at its narrowest, with nothing on,
ImgBB needing a key, ImgBB active, a host of your own, and both rows unfolded. The upload client is
tested against a mock server for both providers (the self-hosted wire format is unchanged, asserted
byte for byte; ImgBB's request line, multipart body and error answers). Not checked: ImgBB itself
with a real key, the rows in a real window, and a real Keychain/Secret Service/Credential Manager
round trip of an ImgBB key.
