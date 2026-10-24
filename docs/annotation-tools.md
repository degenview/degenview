# Text, note and content tools

Twelve drawing tools in a third flyout off the strip (the Notes tile, after Shapes), in two
sections: **TEXT AND NOTES** (Text, Note, Price note, Pin, Table, Callout, Comment, Price label,
Signpost, Flag mark) and **CONTENT** (Image, Post). They are ordinary drawings: placed,
edited in place, styled, moved, resized, locked, hidden by timeframe, undone, saved, synced,
listed in the object tree and painted into snapshots by the code every other tool uses. A tool is
a row of `Tool::spec()` and `Tool::default_style()`, a `Geometry` variant and a projection; there
is no second drawing framework. Read [Drawing tools](drawing-tools.md) first.

What TradingView's help pages and Charting Library `*LineToolOverrides` pages state is marked
**doc** or **lib**; what they do not state, and was chosen here, is marked **inferred**. Nothing was
compared pixel for pixel with TradingView.

## The tools

| Tool | Placement | What is drawn | Default |
|---|---|---|---|
| Text | 1 click, then type | a text box at a chart point, optional background and border, optional wrapping | `#2962FF`, 14 px; background `rgba(41,98,255,.25)` off; border `#707070` off; wrap off, 200 px (**lib**) |
| Comment | 1 click, then type | a compact speech bubble whose tail points at the point | background and border `#2962FF`, white text, 16 px (**lib**) |
| Price label | 1 click | a plate showing the price at the point, with a pointer | `#2962FF`, white bold 14 px (**lib**) |
| Flag mark | 1 click | a pole planted at the point with a flag; no text | `#2962FF`, fixed size (**doc**, **lib**) |
| Pin | 1 click | a pin at the point; its note opens when the pin is clicked | blue card like Note, wraps at 220 px (**inferred**) |
| Note | 2 points, then type | a card with a dot at the first point and a thin leader to it | `rgba(41,98,255,.7)`, border and marker `#2962FF`, white 14 px (**lib**) |
| Price note | 2 points | as Note, with the anchor's price on the label (and optional text) | as Note (**inferred**) |
| Callout | 2 points, then type | a speech bubble: the first point is what it points at, the second where the box sits | `rgba(0,151,167,.7)`, border `#0097A7` 2 px, white 14 px, wrap 200 px (**lib**) |
| Signpost | 2 points | a marker at the anchor with a plate on the same bar, joined by a stem | plate `#2962FF`, 12 px, symbol off (**lib**) |
| Table | 1 click | a grid of text cells, 3 × 3 to start; chart-anchored or pinned to the screen | slate card, grey border, light 13 px text (**inferred**) |
| Image | choose, then 1 click | a stored picture at a size of its own, top-left corner at the click | none |
| Post | paste a link, then 1 click | a card for an X post | theme colours (**inferred**) |

Text, Comment, Note and Callout open their editor as soon as they are placed and leave nothing
behind if it is cancelled with Escape. Text of the other tools is added with Edit text or the
dialog. A Price label's text is its price, formatted at paint time with the chart's own
`PriceFormat`; it is never stored, so moving the anchor changes it.

### Choices TradingView does not document

* **Signpost position.** The Charting Library describes a "vertical position percentage and bar"
  that no page defines. Here the label's price is stored as a second anchor and `Geometry::constrain`
  holds its time to the anchor's, so the plate is always on the anchor's bar and can be dragged
  only up and down.
* **Signpost symbols.** The app's fonts cannot draw colour emoji, so the optional pin shows one of
  26 monochrome symbols (`Emoji`). Each is in the fonts egui bundles, the only fonts every platform
  has: the macOS system fonts the app puts first would draw more (hearts, arrows, triangles, ₿), but a
  Linux build would show a box for them, which Linux CI caught. A test checks every symbol against the
  bundled fonts alone. The choice is stored as a name, never a character.
* **Pin.** Its note opens on a click on the pin, centred above it (below when there is no room above),
  slid sideways to stay on the plot, and closes with Escape or a click elsewhere. A note taller than
  the plot is held on it and its text is cut at the box's end (there is no scrolling). Which pins are
  open is layer state: it is never saved or synced.
* **Table, Pin, Price note, Image and Post defaults** are not in any TradingView page.
* **Post** has no official page. It follows the community description: a post is pasted as a link
  and attaches to the time it was made. TradingView's **Idea** tool (a published idea tied to a
  symbol) is not built.

## How it is built

* `model/drawings.rs`: `Geometry::{Annotation, Leader, Signpost, Table, Image, Card}`.
  `Annotation` covers Text, Comment, Price label, Flag mark and Pin (`AnnotationKind`); `Leader`
  covers Note, Callout and Price note (`LeaderKind`). `Drawing::edits_text_in_place` (the
  `Caps::INLINE_TEXT` capability) replaced the old "text on a line" test. `Tool::max_text` is 2000
  characters for Pin, Note and Callout and 500 for the rest; a table cell holds 500.
* `model/drawing_style.rs`: the box style fields (`border`, `wrap`, `box_width`, `emoji`,
  `emoji_shown`, `emoji_fill`) are `Copy`, clamped in `sanitized()` and left out of a stored row while
  they hold their defaults, like the pitchfork levels.
* Geometry is pure and tested in `ui/annotation_geometry.rs` (bubble with a tail, pin, flag,
  signpost, note rectangles); projection is `drawing_layer/note_shapes.rs` and
  `table_shapes.rs`. Boxes are sized with the context that paints (`TextMeasure`, in `Projection`),
  so hit shapes, handles, the toolbar and a snapshot agree on every box's size.
* Text with no background still has to be clickable: `Screen.hit` holds regions that count for a
  click though nothing is painted there. Every new `Screen` has a line, an outline, a solid or a hit
  region; the every-tool loops depend on it.
* Everything stays on the layer's contract: it returns `DrawingCommand`s, one batch is one undo
  step, and previews and cancelled drafts are not commands.

### Text editing

`drawing_layer/text_edit.rs` edits a drawing's text in place with a real focused egui `TextEdit`
(so selection, the clipboard and `egui_wants_keyboard_input` work as in any field, and shortcuts
leave the keys alone). A selected drawing that takes text shows a faint "Add text" where its text goes;
a click there opens the editor. Return, or leaving the field, keeps the text; Shift+Return adds a line;
Escape drops the edit. A tool that is nothing without its text (Text, Note, Callout, Comment) is a
*draft* while it is typed: shown as it will be, and only added, with its text, when the edit is kept,
so Escape leaves no drawing, no undo step and no sync entry. A kept edit of an existing drawing is one
`Replace`.

IME composition in a real window was not exercised.

### Tables

`model/table.rs` holds the pure operations (`insert_row`, `insert_column`, `delete_row`,
`delete_column`, `set_cell`, `resize_*`, `scale_to`, tab-separated paste) with the limits
`MAX_ROWS` 30, `MAX_COLUMNS` 12 and `MAX_CELL_CHARS` 500, enforced in `is_valid` and in every
operation. A table keeps its chart anchor *and* an optional `ScreenAnchor` (fractions of the plot).
With `screen` set the table ignores its time and price, so a resize, a different display or a
second Mac keep it where it was; `Geometry::anchors()` returns none for it and every move goes
through `edit::translate`. Switching back to the chart restores the anchor it had.

Interaction: click a cell to select it, type or double-click to edit, Tab / Shift+Tab / arrows
move between cells (the editor consumes them from the event list, as it does Enter), the borders
highlight and drag to resize a row or column, the corner scales the whole table, the toolbar and
context menu add and delete rows and columns and pin or unpin the table. Copy, Cut and Paste of a
selected cell belong to the cell, not the chart (`take_table_cell_selected`). Pasted text expands
into cells within the limits and never grows the grid past them.

### Images

* **Ingest** (`service/image_ingest.rs`, on `work_queue::ASSETS`, never the UI thread): PNG or JPEG
  only, by signature. At most 10 MiB read, at most 8192 px a side and 32 Mpx, decided from the header
  before decoding. The picture is scaled to at most 2000 px, re-encoded (which drops every piece of
  metadata), as PNG when it really has transparency and JPEG otherwise, and squeezed (quality, then
  size) until it is at most 600 000 bytes, which is what fits a sync record. A picture that cannot
  fit is refused with a message and nothing is created.
* **Store** (`store/asset_store.rs`): table `drawing_asset`, key = sha256 of the stored bytes, so the
  same picture is one row however many drawings use it. The hash is verified on read; a damaged row is
  kept and shown as damaged, never replaced.
* **Drawing**: `ImageData { at, size, asset, name, pixels, opacity, keep_aspect }`. It names its
  picture and never carries it, so layouts, undo steps and synced drawing records stay small. The
  picture's top-left corner is a chart point, so it moves with the chart; its `size` is in logical
  pixels, so zooming, scrolling or stretching an axis never resizes it. It starts at its own size
  (at most 480 × 360, and at most 40 % of the plot's width and half its height where it is placed).
  The eight box handles resize it (keeping its shape by default; the corner stays a chart point and
  the new size is stored in pixels), opacity is 5 to 100 %.
* **Painting** (`service/asset_host.rs`): decoded once on a worker, uploaded as a texture once and
  dropped when nothing shows it. A picture that is loading, not in the store (it may be on its way from
  another Mac, and is asked for again every two seconds while it is visible) or damaged is painted as
  a placeholder that says which.
* **Choosing**: arming the tool opens a sheet with the file panel (macOS, `osascript`, like CSV
  import), a **Paste from clipboard** button and a drop zone. A file dropped on the window is placed in
  the middle of the focused chart: macOS reports no pointer position during a file drag. An image-only
  clipboard gives the windowing layer no paste event, so Cmd+V does not paste a picture; the button
  reads `NSPasteboard` itself. Replace, from the toolbar, the context menu or the dialog, swaps the
  picture and keeps the place, width and opacity. From the dialog it commits with OK.
* **Snapshots** are handed the decoded pictures of the visible images (frozen on the UI thread) and
  paint the same placeholder for any that are not ready.

### Post

`model/link_card.rs` is pure. It accepts `https` (and upgrades `http`), no user name or port, and only
`x.com`, `twitter.com` and their `www`/`mobile` forms (`/{handle}/status/{id}` and `/i/status/{id}`),
and writes every accepted link one canonical way. A card keeps the link, its size and, if the user fetched one, a
`LinkMeta` of title, author and text, each cut to a cap.

* **What the card shows without a fetch** is what the link proves: the service, `@handle`, the id and
  the post's time, decoded from the snowflake id (`(id >> 22) + 1288834974657` ms; ids from before
  snowflake ids have none).
* **Placement**: arming opens a sheet that checks the link as it is typed. A post is anchored where it
  is placed, and can be dragged anywhere. A link pasted on a chart (Cmd+V, no text field focused) places
  a card in the middle of the focused chart.
* **Line to the candle**: a thin line with a dot joins the card to the candle of the time the post was
  written, the way TradingView draws it. It is always there and is worked out each frame from the
  link's time and the chart's own candles, so it follows a drag, a pan and a change of timeframe
  (on 1D it lands on the day that holds the post, on 1H the hour); nothing about it is stored. It
  meets the candle's high when the card is above the candle's middle, its low when below. There is
  no line for a post with no time (numbered before snowflake ids), one older than the candles loaded
  (it appears when older history is paged in), or one newer than the last candle. It is drawn in the
  card's stroke colour (**inferred**: TradingView's exact look is not documented).
* **Preview** is fetched automatically right after a card is placed (by a click or a paste) and only
  then: never on paint, on opening a layout, or for a card that arrived from another Mac (its preview
  syncs with it). The first time, the app asks once for permission (`content.prefs`, a setting that
  stays on this Mac); after that placing a card just loads it. Three requests, all to X's own hosts, on
  `work_queue::LINKS`: `publish.x.com/oembed` for the name, handle and text (the host
  `publish.twitter.com` only redirects), `cdn.syndication.twimg.com/tweet-result` (the endpoint X's own
  embeds use; it wants a token computed from the post id) for the address of the author's picture, and
  `pbs.twimg.com` for the picture. https only, no redirects, no cookies, no credentials, 12 s, at most
  256 KB read of an answer and 1 MB of a picture. Answers are parsed as data, nothing in them runs, and
  the result is written to the card as one undoable change. The picture's address is accepted only on
  `pbs.twimg.com` under `/profile_images/`; the picture goes through the same checks and re-encoding as
  an Image (so it is a stored picture, named on the preview, and syncs as one: see Images) and is best
  effort, since the text is the preview and a card without a picture is not an error. The **Preview** chip on a card's toolbar is shown only while the card has no
  preview (to retry after a failure); once one is loaded it is gone. A deleted or private link says so and leaves the card as it was. Post
  thumbnails and media are not fetched; the author's picture is.
* **Open** (`ctx.open_url`) is always a deliberate click; so are Copy link and Replace link (which
  drops the old preview).

The shipped request path (`link_fetch::fetch`: the client, https-only, the parsers) was run, on
2026-10-23, against a real public post: the oEmbed answer's author name, `@handle` and text came back,
and the author's picture (73 × 73 px, 3.9 kB after re-encoding) was downloaded and accepted.
That is the ignored test `live_a_real_post_gives_a_preview` (`cargo test --lib live_ -- --ignored
--nocapture`). It is not part of the suite, and X does not promise that behaviour. If it stops
answering, the card shows the link and the failure reason, and everything else works.

## Persistence and sync

A drawing is one record as before. `Geometry::Table` carries its cells, `Geometry::Card` its link and
`LinkMeta` (all short strings, far below the 10 000-character field limit) and `Geometry::Image` only a
64-character picture name. See [iCloud Sync](icloud-sync.md#pictures) for the picture entity, its limits
and what is not verified.

## Snapshots

Everything above is painted by the same `paint_frame` the chart uses, so a snapshot shows what is on
screen: boxes, tables, cards, pictures (placeholders for unready ones), and the notes of the pins
that are open (`SnapshotInput.open_notes`). A parity test compares a live frame with a snapshot frame.

## MCP

`list_drawings` and `delete_drawing` see the new tools by their snake-case names (`text`, `price_note`,
`flag_mark`, `table`, `image`, `post`, …); they cannot create them. Pictures and previews are not
exposed.

## Tests

Every `every_tool_…` loop in `ui/drawing_layer/tests.rs` runs over all 45 grouped tools, content tools
included (through `Drawing::sample` and `sample_content`): placement in one gesture and one undo step,
select, move, delete, every handle, save and reopen, the toolbar. Per-area suites:
`drawing_layer/tests/{note,table,image,card}_tests.rs`, `service/image_ingest.rs`,
`sync/asset_tests.rs` (on `MockCloud`), `service/link_fetch.rs` (parsers on fixtures),
`ui/content_card.rs`, `workspace/context/tests.rs` (paste, consent, fetch results). Galleries that
write PNGs for a person to look at:

```
DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_ -- --ignored
```

## Not verified, and known gaps

* **Real CloudKit and a second Mac.** Picture records are exercised on `MockCloud` only. Whether a
  picture arrives on another Mac, and how a record near the 900 KB limit behaves, has not been seen.
* **Pictures are never deleted.** Layouts sync, tab sessions do not, so no Mac can know another still
  draws a picture, and a tombstone for an immutable record would also trip the mass-delete hold. A
  picture stays after its drawing is gone: at most 600 kB per distinct image.
* **Live X** was reached once (see above), not continuously. Embedded timelines, counts and
  post media are not implemented, by design. The embed endpoint for the author's picture is
  undocumented (it is what X's own embeds call); if it changes, cards load without a picture.
* **TradingView pixel parity**, the exact look of Pin, Table, Price note and the Post card.
* **Real window.** On 2026-10-23 a release build was launched against a throwaway home whose saved
  session held one BTC/USDT chart with a drawing of each of the tools (the stored picture included).
  They all restored and painted correctly with the real fonts and GPU textures. Synthetic mouse clicks
  did not register in that window, so no placement, editing, dragging or sheet was driven in a real
  window: those are covered by the controller tests only.
* **IME**, VoiceOver and a real-window drag of a table border were not exercised.
* **Windows and Linux**: the file panel and the clipboard picture are macOS-only; drag and drop work
  where egui reports them.
* **No Cmd+V of an image**, and a dropped file lands in the middle of the chart (see Images).
