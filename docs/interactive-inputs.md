# Interactive Pine inputs

`input.*(…, confirm = true)` and "Reset points…": a script's inputs set by clicking its chart, as
TradingView does it. Read this before changing the setup flow, the picking mode or the markers.

## TradingView, as measured (2026-11-28)

Probed live on TradingView; "manual" marks what comes from the Pine manual alone.

- **Points** are `input.time` and `input.price`. A time and a price with the same `inline` and
  `group`, and no other time or price there, are one click, in either order (manual).
- **Adding** asks only the points marked `confirm`, in declaration order. The script shows as
  loading and does not run. A separate-pane script gets an empty pane at once.
- The **prompt** is a pill at the bottom of the plot: `Set the "<inline>" time and price for
  "<script>"`, or `Set the "<title>" time for …` / `price for …`. Its X hides only the pill.
- The **crosshair** shows what the click sets: both lines, only the vertical one (time) or only the
  horizontal one (price). Points already clicked stay drawn.
- A drag pans; the magnet snaps to open/high/low/close; bars past the newest can be picked (the
  future bar's open time); the price is not rounded to the tick.
- A separate-pane script that has not run reads its prices from the **price chart**; its empty pane
  takes no clicks.
- `active = false` inputs are still asked.
- After the clicks, when an input that is not one of those points is marked `confirm`, a **Confirm
  Inputs** sheet lists every `confirm` input (clicked values editable). With only points there is
  no sheet.
- **Esc** while picking, or Cancel in the sheet, removes the script.
- **Reset points…** asks every point, `confirm` or not, shows no sheet, and applies after the last
  click. Esc keeps the old values.
- Changing the symbol does not ask again.

## Decided, not measured

- A pair is `confirm` when either half is.
- Reset points on a separate-pane script that has run reads its own pane.
- A chart switched to another market while a script is being added starts its clicks over; a reset
  is dropped.
- A second script added meanwhile waits its turn.
- Right-click is unchanged while picking.
- Esc gives up the setup only when nothing lies on top of the chart (picker, popover, sheet, text
  field, open menu) and no tool is in hand, one thing per press.
- Taking up a drawing tool suspends picking until it is put down.
- Markers and tags are neutral, not TradingView's blue.

## How it is built

- `pine::inputs::points` pairs inputs into `PointInput`s; `model::pine::setup::InputSetup` is the
  state machine (pick, sheet, done). It holds only what was clicked or edited.
- `workspace/pine/setup` queues the setups. **A script being added is not on the chart**: the
  session owns the `PineInstance` (its id reserved with `ChartCard::reserve_instance_id`), and the
  card only gets a `SetupView` (chip, empty pane, point asked). Saving, sync, MCP, alerts and runs
  read `pine_instances()`, so none of them can see it. Finishing is one undo step; giving up leaves
  none.
- `ChartWorkspace::add_script` is the way a person adds a script (indicator picker, chart menus,
  Pine Studio, chart settings). `insert_script` adds without asking: MCP, which reports
  `confirm_inputs_at_default` instead.
- The card (`ui/chart/card/points`) draws the prompt and points and reports
  `SetupEvent::Picked { time_ms, price }`. Only a click answers; a drag still pans.
- A time past the data comes from `bucket::Calendar` through `TimeIndex`, so a stock's next bar is
  the next session's.

## Not done

- `inline` inputs are not laid out on one row in the settings or the sheet.
- Time inputs are a text field, not a date picker.
- The Pine Studio preview does not ask; it uses its Inputs tab.
- Symbol change mid-pick, a second add mid-pick, a half-`confirm` pair and a reset on a running
  separate-pane script have not been checked on TradingView.
