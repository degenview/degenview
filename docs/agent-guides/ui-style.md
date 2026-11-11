# Flat style

The default look of every window, sheet, panel and card in DegenView. The reference is the Settings
window (`src/ui/settings_window.rs`) next to the main window. New UI follows this; existing UI has not
been audited for it, so do not restyle a screen you were not asked to touch, but do not copy its older
habits either.

## Rules

- **Flat surfaces.** A surface is a faint fill (`theme::card_fill`) and a 1px hairline
  (`theme::card_border`, a clearly visible lighter-gray line, not a faint one), radius `kit::CARD_RADIUS` (10). Build cards with `kit::card_frame`. No
  gradients, no lit top edge, no drop shadows. Shadows exist only for things that float above the app
  (menus, popovers, dialogs). `theme::card_highlight` is not for cards.
- **Icons stand alone.** A row's icon is drawn bare, in `theme::text_secondary()` (`text_primary()` when
  its row is selected): no tile, disc, circle, rounded square or colour behind it. Cards use
  `kit::paint_card_badge` (18px, no tile). Sidebars use 16px icons. A brand logo (a broker, a data
  source) is the one exception and keeps its own mark, with a hairline.
- **Picture headers** (`ui::art`, `settings_art`, `alert_art`) may colour the marks inside the picture,
  but Settings pictures set `Art::flat` so no coloured disc sits behind them.
- **Colour means state or data.** A selected row is `theme::selected_tint`, a status is a
  `kit::status_badge`, a series or price is its own colour. Chrome, icons and controls are neutral.
  Never blue; see [Core guidance](core.md).
- **Native window controls.** A secondary window (Settings, Alerts) keeps the platform's title bar and
  its close / minimise / zoom controls. Only tab windows draw their own strip (`ui::window_chrome`).
  Do not draw an in-window title bar, a gear-and-title header or a close button inside a window that
  already has one.
- **One scale of text.** Page title `kit::size::TITLE2 + 3`, section heading `HEADLINE` bold, body
  `BODY`, hints `CAPTION` in `text_secondary`. Take sizes from `kit::size`, do not invent them.

## Sidebar and page layout

Use the helpers in `settings_window.rs` instead of building a page by hand:

| Need | Use |
| --- | --- |
| A page with header, scrolling body and an optional footer | `page_shell` (header is `page_header`: picture, title, one-line description) |
| A titled group inside a page | `section` |
| A card of rows | `card`, separated by `divider`, one row as `row` |
| A card for one setting (icon, title, hint, control right) | `settings_card::card` / `card_reserving` |
| A status pill | `status_badge`; a status line `status_message` |
| Actions pinned under a page | `footer_row` |

The sidebar is `SIDEBAR_WIDTH` wide on `theme::background()`, with `theme::separator()` as its only
edge. Rows are 30 points: a bare icon, a title, `selected_tint` when open, a faint tint on hover. Pages
must work at the window's minimum size; the Pine Studio page tests read `WINDOW_SIZE` and
`SIDEBAR_WIDTH`, so change them only together.

## Checking it

Look at the page, in both themes, at the default and the narrowest window size, before shipping:
a gallery test or a screenshot of the window (see the gallery tests in `ui/` for the recipe). Compare
the card edges and icon alignment against a neighbouring Settings page, not against memory.
