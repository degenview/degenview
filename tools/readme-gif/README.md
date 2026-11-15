# README hero GIF

Records the animated hero picture from the real app: a script drives a throwaway DegenView through its
own MCP server while a second script films the app's window. Retake it when the UI it shows changes.
macOS only.

## The story

About 20 seconds, then it loops.

1. A clean BTCUSDT 1D chart.
2. Volume, EMA 20, Ichimoku Cloud and MACD appear one after another.
3. A resistance line draws itself, followed by a support level and a demand zone.
4. `ETHUSDT` is typed into the picker and overlaid with Compare, and the axis flips to percent.

The drawings' anchors are computed from the live candles (swing highs and lows of the last 110 bars),
so each take fits the market of the day. Edit the story in `drive.py`.

## Run it

Build the binary once. MCP credentials seeded into a store are only read by a debug build or by a release
build with this feature (never enable it for a distributed build):

```bash
CARGO_TARGET_DIR=target-gif cargo build --release --locked --features dev-unsealed-mcp
```

Then record:

```bash
tools/readme-gif/record.sh /tmp/hero.gif
```

`DEGENVIEW_BIN=<path>` uses another binary and `KEEP=1` keeps the work directory, which holds every
frame and a contact sheet (`take/sheet.png`) for checking each beat.

What you need: Xcode's command line tools (`swiftc`), Python 3 with [Pillow](https://pillow.readthedocs.io),
and `ffmpeg`. The terminal you run it from needs Screen Recording and Accessibility permission.

While it runs (about a minute) the app window is frontmost, so leave the keyboard and mouse alone.

## What it does and does not touch

- The app runs with a throwaway `HOME`, no Keychain and no notifications. Your own profile is not read
  or written, and your own DegenView can stay open (the recorder's MCP port is 8791, change it with
  `DEGENVIEW_RECORD_PORT`).
- Only the app's own window is captured, by window id, never the screen or a region.
- The MCP token is generated for the run and exists only in the process environment and the
  throwaway store.
- The only mouse action is one drag of the window's corner to resize it, refused unless the app's window
  is the topmost one under the pointer. Everything else is the MCP or keystrokes to the app.

## Files

| File | Role |
| --- | --- |
| `record.sh` | Runs everything and cleans up |
| `seed.py` | Seeds the throwaway store: one chart, an MCP client, the Ichimoku and MACD scripts |
| `drive.py` | The story, timed |
| `mcp_client.py` | A minimal MCP client |
| `recorder.py` | Captures the window, frame by frame, with timestamps |
| `encode.py` | Frames to a looping GIF under 8 MB (lowers rate and colours until it fits) |
| `sheet.py` | Contact sheet of the key frames |
| `winlist.swift`, `mouse.swift` | Find the window, resize it, park the pointer |

## Notes

- The MCP cannot add the bundled Pine indicators, so `seed.py` copies two of them into the throwaway
  script library and `drive.py` applies them from there. Their chips on the chart show the file names
  (`macd`, `ichimoku-cloud`) in lowercase for that reason.
- The picture is live data. If the app's update check shows an "Update" mark in the tab strip, retake it.
