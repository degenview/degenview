# UI style

Default look and controls of every window, sheet, panel and card. Reference: the Settings window
(`src/ui/settings/window.rs`) beside the main window. New UI follows it. Existing UI is not audited:
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
- **Picture headers** (`ui::kit::art`, `settings_art`, `alert_art`) may colour marks inside the picture;
  Settings pictures set `Art::flat` so no coloured disc sits behind them.
- **Colour means state or data.** Selected row `theme::selected_tint`, status `kit::status_badge`,
  a series or price its own colour. Chrome, icons, controls neutral. Never blue; see
  Buttons and Links below.
- **Native window controls.** Secondary windows (Settings, Alerts) keep the platform title bar and
  close / minimise / zoom. Only tab windows draw their own strip (`ui::window::chrome`). Never draw an
  in-window title bar, gear-and-title header or close button in a window that has one.
- **Finished.** Design ships complete: every state (empty, loading, error, hover, disabled, long text) is designed, spacing and alignment are exact, and you have looked at it in a picture before pushing.
- **Busy states use the candle indicator.** `kit::loading::LoadingIndicator` (a scrolling mini
  candle chart), not `egui::Spinner`, for new work-in-progress UI. `.size(n)` is the side of a
  square: below 20 it is 3 candles, from 20 it has wicks, from 28 it takes `theme::UP`/`DOWN`;
  smaller it is `theme::text_secondary()`. `.color(c)` forces one colour at any size (a button's
  text colour). `.paint_at(ui, rect)` mirrors `Spinner::paint_at`. It repaints itself while shown.
  A pane with nothing to show yet uses `kit::loading::message(ui, area, text)` (a 40 indicator over
  its text, centred); a progress toast puts a 48 one above its text, as the snapshot notice does.
  Large sites already use it (update check, symbol pickers, portfolio panes, Add chart trending,
  the assistant); small inline `egui::Spinner`s (Test buttons, busy buttons, field adornments) are
  not migrated yet: swap one when you touch it.
- **One scale of text.** Page title `kit::size::TITLE2 + 3`, section heading `HEADLINE` bold, body
  `BODY`, hints `CAPTION` in `text_secondary`. Sizes only from `kit::size`.

## Panels, menus, buttons, links and dialogs

- **Docked panels.** Only egui's separator (`theme::border()`). Never `Frame::stroke` or painted edge lines. `show_separator_line(false)` only when deliberately replacing the separator.
- **Menus.** `kit::menu_row`, `menu_item`, `menu_heading`, `menu_separator`, `menu_submenu` inside `kit::menu_popup` or `kit::context_popup`, after `kit::menu_scope`. Never bare egui buttons, radios, checkboxes or menu buttons. `ui.close()` for actions that dismiss; toggles keep it open.
- **Menu sizing.** Fit to the longest row, never wider: `kit::fitted_menu` / `fitted_menu_rows` for button menus, `fitted_context_menu` for right-clicks, `menu_fit` / `menu_fit_rows` inside popups. Helpers measure dynamic menus only while open. Test fixed-width menus against `kit::menu_width_for_rows` (see `menu_widths_fit_their_rows` in `watchlist/sidebar.rs`). Never guess widths with `set_min_width` or `set_width`. Check visually that the longest label ends one row inset from the edge.
- **Buttons are neutral, never blue.** No blue accent in fills, borders, labels or outlines. Use `kit::sheet_button` (`prominent`), `kit::push_button`, footer Primary, or `theme::card_fill` / `card_border` / `card_highlight` with `text_primary`. Large sheet actions: `kit::large_prominent_button`; destructive: `kit::destructive_sheet_button`. Accent colours are for data and state, not controls. Compare new buttons with neighbouring sheets; existing accent buttons are not all audited.
- **Links are gray.** Underlined `theme::text_secondary()` for hyperlinks and link-like text, never blue. Add no decorative accent use; do not change selection/focus colours unless asked.
- **Dialogs.** `kit::confirm`, `kit::alert` or `kit::notice` through `ui::kit::dialog::show`; never custom modal frames or stacked full-width buttons. Layout: tinted title badge, message, close button, bottom-right Cancel/action. `Dialog::destructive` for destructive actions (red, Cancel focused), `cautious` when Enter must not approve, `body` for extra content, `leading` for a third left-side button. Tint only the badge, never blue. Large forms go in `ui::kit::sheet`. Verify in `gallery_of_the_dialogs`.

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

Use the `settings/window.rs` helpers, not hand-built pages:

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
