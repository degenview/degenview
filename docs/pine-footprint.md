# Pine volume footprints

`request.footprint()` and the `footprint` / `volume_row` types, as a script sees them. The pure
computation is `src/pine/footprint.rs`; the script-facing side (arguments, intrabar candles,
handles, dispatch) is `src/pine/footprint_call.rs`; end-to-end tests are `src/pine/footprint_tests.rs`.

## What TradingView documents

Sources: the Pine Script release notes (January 2026), the Pine docs ("Other timeframes and data →
`request.footprint()`", "Type system → `footprint` and `volume_row`"), the Help Center article for the
Volume Footprint chart, and TradersPost's reference. Premium and Ultimate plans only on TradingView.

- `request.footprint(ticks_per_row, va_percent = 70, imbalance_percent = 300) → footprint`.
  `ticks_per_row` is a positive whole number of `syminfo.mintick`; a row is that tall. The result
  is the **current bar's** footprint, or `na` when it has none.
- A footprint "categorizes volume values from lower timeframes as buy (upward) or sell (downward)
  based on intrabar price action": an intrabar whose close is above its open is buy volume, below is
  sell volume, and a doji is compared with the previous intrabar's close, then keeps the previous
  class when level. TradingView's resolution is a tick, 1 second, 1 minute or 60 minutes by plan and
  by how old the bar is.
- `footprint.buy_volume / sell_volume / total_volume / delta (id)`, `footprint.poc / vah / val (id)`
  (a `volume_row`), `footprint.rows (id)` (`array<volume_row>`, lowest price first),
  `footprint.get_row_by_price (id, price)` (`na` outside every row).
- `volume_row.up_price / down_price / buy_volume / sell_volume / total_volume / delta /
  has_buy_imbalance / has_sell_imbalance (id)`.
- All of them work in function syntax (`footprint.poc(fp)`) and method syntax (`fp.poc()`).
- A buy imbalance compares a row's buy volume with the sell volume of the row **below**, a sell
  imbalance a row's sell volume with the buy volume of the row **above**; the test is
  `max >= imbalance_percent / 100 * min`.
- A script may depend on one unique `request.footprint()`; a second with other arguments is a
  runtime error.

## What this build does

- **Data.** There is no tick or trade data anywhere in the app (`model::Kline` is OHLCV). The
  footprint is built from the chart's symbol cut into finer candles, which the
  `SecurityDataProvider` supplies the way it does for `request.security_lower_tf`. The candle size
  is the smallest of 1, 5, 15, 30 or 60 minutes that is shorter than the chart's bar, divides it,
  and keeps the whole chart within `MAX_INTRABAR_CANDLES` (a long daily chart gets 30-minute
  intrabars, a short 5-minute chart gets 1-minute ones). TradingView's own rule for the older part
  of its history is the same idea; its newest bars use ticks or seconds, which this build cannot.
  A footprint here is therefore **coarser than TradingView's, not the exchange's order flow**; the
  call warns once (`PINE7001`: "only partly supported") for that reason.
- **`na`.** `request.footprint()` is `na` for a bar with no usable intrabars (no provider, none for
  that bar, none with volume), on charts of 1 minute or less (nothing finer is fetched), on
  price-based charts (Renko and the like; their bars have no fixed length), and for a bar that
  would need more than `MAX_ROWS` (1000) rows. Guard with `not na(fp)` as TradingView's examples do.
- **Handles.** A `footprint` is the bar's index and a `volume_row` that index with a row number;
  nothing is stored in the run's state. The data sits in `Session::footprints`, which keeps the
  newest 500 bars, so `fp[1]` works and a handle to an older bar reads as `na`. A handle is not a
  heap object: it cannot be sent through `request.security` (it comes out `na`).
- **Errors.** `PINE4033` a second `request.footprint()` with other arguments; `PINE4034` inside
  another request's expression; `PINE4035` a bad argument; `PINE4036` a `footprint.*` or
  `volume_row.*` function called without an id or with `na` (method syntax on `na` is `PINE4018`,
  as for every other type).
- **Symbol and tick size.** The tick is `syminfo.mintick`, which the host supplies or the session
  infers from the decimals of the last 200 bars; for a crypto pair it is not the exchange's tick
  size, so rows can differ from TradingView's for the same `ticks_per_row`.

## Inferred (TradingView does not say)

Each is a deliberate choice, named in the code, and should be revisited if TradingView's behaviour
is ever observed.

- **Row grid.** Rows sit on a global grid (row `k` is `k * height` to `(k + 1) * height`), so the
  rows of neighbouring bars line up, from the bar's lowest row to its highest with empty rows
  between. TradingView says only that rows "cover the bar's price range".
- **Spreading volume.** An intrabar's volume is spread evenly over its low-to-high, each row
  taking the share of the range it overlaps (a candle with no range goes into one row). The
  volume profile tools do the same (`model/volume_profile.rs`).
- **First doji.** The first intrabar of a bar compares with the last intrabar before it; with
  nothing before it, it is buy.
- **POC and value area.** The row of most volume; a tie goes to the row nearest the middle of the
  bar's range, then the lower one. The value area grows from it by the heavier pair of rows beside
  it until it holds `va_percent` of the volume. Both are the volume profile's
  (`volume_profile::point_of_control_and_value_area`, shared); TradingView says its algorithm is
  "similar to" that of its Volume Profile indicators.
- **Imbalance edges.** The lowest row has no buy imbalance and the highest no sell imbalance (no
  neighbour). A row that bought against a neighbour that sold nothing is an imbalance, one that
  bought nothing is not.
- **`get_row_by_price`.** A row owns the prices from its bottom up to, not including, its top; the
  highest row owns its top too.
- **Row limit.** 1000 rows per bar, so a one-tick row height on a wide bar gives `na` rather than
  tens of thousands of rows per bar.
- **Inside `request.security`.** Not supported (`PINE4034`); TradingView counts such a call as
  another unique request.

## Not verified

- Never compared against a TradingView chart: only the documented definitions were implemented,
  and the tests use hand-computed numbers on a fake provider.
- Not exercised against live exchange candles in a window; the host's intrabar fetch is the one
  `request.security_lower_tf` already uses.
- The footprint on a bar still forming is rebuilt when its chart candle changes, but the provider's
  intrabars are fetched once per run, as for `request.security_lower_tf`.

## Follow-ups that would make it closer to TradingView

- Seconds from Binance (it serves `1s` klines) so 1-minute charts and the newest bars of others get
  a footprint; real buy/sell volume from Binance's taker-buy volume (kline column 9, which the
  parser drops) or aggregated trades instead of the close-versus-open rule.
