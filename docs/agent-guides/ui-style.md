# Flat style

Default look of every window, sheet, panel and card. Reference: the Settings window
(`src/ui/settings_window.rs`) beside the main window. New UI follows it. Existing UI is not audited:
do not restyle screens you were not asked to touch, and do not copy their older habits.

## Rules

- **Flat surfaces.** Faint fill (`theme::card_fill`), 1px hairline (`theme::card_border`, a clearly
  visible lighter gray, not faint), radius `kit::CARD_RADIUS` (10). Build cards with `kit::card_frame`.
  No gradients, lit top edge or drop shadows; shadows only for things floating above the app (menus,
  popovers, dialogs). `theme::card_highlight` is not for cards.
- **Icons stand alone.** Bare, in `theme::text_secondary()` (`text_primary()` when the row is
  selected): no tile, disc, circle, rounded square or colour behind. Cards use `kit::paint_card_badge`
  (18px, no tile); sidebars 16px icons. Exception: a brand logo (broker, data source) keeps its own
  mark, with a hairline.
- **Picture headers** (`ui::art`, `settings_art`, `alert_art`) may colour marks inside the picture;
  Settings pictures set `Art::flat` so no coloured disc sits behind them.
- **Colour means state or data.** Selected row `theme::selected_tint`, status `kit::status_badge`,
  a series or price its own colour. Chrome, icons, controls neutral. Never blue; see
  [Core guidance](core.md).
- **Native window controls.** Secondary windows (Settings, Alerts) keep the platform title bar and
  close / minimise / zoom. Only tab windows draw their own strip (`ui::window_chrome`). Never draw an
  in-window title bar, gear-and-title header or close button in a window that has one.
- **One scale of text.** Page title `kit::size::TITLE2 + 3`, section heading `HEADLINE` bold, body
  `BODY`, hints `CAPTION` in `text_secondary`. Sizes only from `kit::size`.

## Pages have no bar and no Save button

No footer bar, Save or Apply under a Settings page. Inputs save as entered; other actions sit in the
content they act on.

- `settings_api_keys::Autosave` with `field`'s response: `watch(&response, secret)`, then `due(ctx)`
  and save. Plain fields save when typing pauses (700 ms) or the field is left; secrets (key, token)
  only on leaving, so half a key is never stored. Choices (switch, segment, picked provider) save at
  once with `touch()`.
- A value that cannot be saved yet (half a key pair, invalid URL) is not saved; say why inline under
  the field with `status_message`, never behind a button.
- Test, Remove and similar buttons go in the card of their target, under its fields; "Add ..." under
  the list it adds to. Results/failures: a `status_message` line beside or under them.
- A secondary view (the Pine theme editor) leaves via its own back button in the header; reset
  actions live in a menu beside the name, not a bar.

## Sidebar and page layout

Use the `settings_window.rs` helpers, not hand-built pages:

| Need | Use |
| --- | --- |
| A page with header and scrolling body (no footer) | `page_shell` (header is `page_header`: picture, title, one-line description) |
| A titled group inside a page | `section` |
| A card of rows | `card`, separated by `divider`, one row as `row` |
| A card for one setting (icon, title, hint, control right) | `settings_card::card` / `card_reserving` |
| A status pill | `status_badge`; a status line `status_message` |

Sidebar: `SIDEBAR_WIDTH` wide on `theme::background()`, `theme::separator()` its only edge. Rows 30
points: bare icon, title, `selected_tint` when open, faint tint on hover. Pages must work at the
window's minimum size; Pine Studio page tests read `WINDOW_SIZE` and `SIDEBAR_WIDTH`, so change them
together.

## Checking it

Before shipping, look at the page in both themes at default and narrowest window size: a gallery test
or window screenshot (recipe in the gallery tests in `ui/`). Compare card edges and icon alignment
with a neighbouring Settings page, not memory.
