# Tutorial videos

Makes a narrated screen-capture tutorial from the real app: a script drives a throwaway DegenView with
the real mouse and keyboard while only its window is filmed, then a second script adds a large cursor,
click ripples, burned-in subtitles, an OpenAI voice-over and title and end cards. Output is a 1080p30
MP4 and an `.srt`. macOS only. The first tutorial is [`tutorials/pine-debugger`](tutorials/pine-debugger/tutorial.py)
(about 5 minutes: breakpoints, stepping, call stack, watches, conditional breakpoints).

## Make one

Build the binary once. A release build is needed (a debug build is too slow to film), and the
`dev-unsealed-mcp` feature keeps it from asking for a Keychain key on every new build (never enable it for
a distributed build):

```bash
cargo build --release --locked --features dev-unsealed-mcp
```

Then:

```bash
export OPENAI_API_KEY=...            # or KEY_FILE=<file with the key on its first line>
tools/tutorial-video/make.sh pine-debugger
```

`make.sh <name> --dry` skips the narration and writes a silent preview (`dry.mp4`) from estimated timing,
which is the way to check the picture without spending anything. Each stage also runs on its own:

| Stage | Command | Writes (in the work directory) |
| --- | --- | --- |
| Narrate | `tts.py <name>` | `tts/<beat>.wav`, `tts/durations.json` |
| Record | `record.py <name> [--dry] [--beats N]` | `rec.mov`, `take.json` |
| Compose | `compose.py <name> [--dry] [--seconds N]` | `<name>.mp4`, `subtitles.srt` |
| Measure | `probe.py <name> <beat> x0 y0 x1 y1` | `probe-<beat>.png` |

The work directory is `$DEGENVIEW_VIDEO_WORK/<name>` (default `$TMPDIR/degenview-video/<name>`).
`DEGENVIEW_BIN=<path>` uses another binary. Composing takes about 9 minutes for 5 minutes of video.

What you need: Xcode's command line tools (`swiftc`), `ffmpeg` with the VideoToolbox encoder, Python 3 with
[Pillow](https://pillow.readthedocs.io) and [numpy](https://numpy.org), and a Retina display. The terminal
you run it from needs Screen Recording and Accessibility permission.

**Leave the keyboard and mouse alone while it records** (about as long as the video). The app's window is
frontmost and the pointer is hidden and driven by the script.

## What it does and does not touch

- The app runs on a throwaway `HOME` seeded by `record.py` (the tutorial's scripts, its open tab and its
  settings), with no Keychain and no notifications. Your own profile is never read or written, and your own
  DegenView can stay open.
- Only the app's window is captured, by window id, never the screen or a region. Nothing else on your screen
  is filmed.
- Every click is refused, and the take stops, unless the app's window is the topmost one under the pointer.
  The only other mouse action is one drag of the window's corner to size it, under the same check.
- The narration text is the only thing sent to OpenAI. The key is read from the environment or a file and is
  never printed or stored.
- The system pointer is hidden for the length of the take through a private CoreGraphics call
  (`SetsCursorInBackground`) and shown again when the script ends, even when it fails.

## Writing a tutorial

A tutorial is a folder `tutorials/<name>/` with a `tutorial.py` and the scripts it uses. Copy the Pine
debugger one. It defines:

- `BEATS`: one beat per spoken sentence, `(id, text, acts, scene)`. `scene` is `title`, `screen` or `end`
  (the cards have no acts). A beat lasts as long as its sentence or its acts, whichever is longer, and
  the acts start 0.3 s into the sentence. Acts: `("go", x, y)`, `("click", x, y)`, `("wait", s)`,
  `("type", text)`, `("key", name, *modifiers)`.
- Coordinates in points inside the app window, which `record.py` sizes to `WINDOW` (1600x900).
- `SCRIPTS`, `TAB` and `SETTINGS`: what the throwaway store holds when the app starts. Seed everything the
  story needs; a tutorial that clicks through setup shows setup.
- `SYNC_REGION`: the first beat's first click must change this region of the window (a menu opening). The
  video is lined up with the action log by finding that change.
- `VOICE`, `VOICE_INSTRUCTIONS`, `KEYCAPS` (keys shown on screen), `TITLE`, `SUBTITLE`, `END_CARD`.

To find coordinates, record a take, then look at where things were: `probe.py pine-debugger b15 1200 500
1600 900` writes the region as it was at the end of beat `b15` with a labelled grid. Positions inside a
panel move with its state (a watch adds a row, a collapsed section removes one), so measure in the state
the beat is in.

Write narration the way it is said: spell out symbols (`bullish equals equals five`), and keep numbers from
live data out of it, since the preview chart shows today's market.

## Gotchas

- **Click only on controls, not on section headers.** The debugger panel's headers collapse on a click,
  and the rows under them move.
- **The capture draws a purple screen-share pill over the window's traffic lights** (macOS 26).
  `compose.py` paints the three buttons back over it.
- **Python's `-I` hides Pillow and numpy** installed for the user; run the scripts plainly.
- **The OpenAI wav has no length in its header**; `tts.py` re-muxes each clip so its length can be read.
- **A stopped `screencapture` can leave the movie in** `~/Library/Group Containers/group.com.apple.screencapture/ScreenRecordings`;
  `record.py` moves it to the work directory.
- **Colour:** the capture is BT.709 limited range. `compose.py` decodes and encodes with that matrix
  explicitly; without it the picture turns green.
- ffmpeg here has no `libass` or `drawtext`, so subtitles are drawn by `compose.py` itself. YouTube takes
  the `.srt` as well, for captions people can turn off.

## Retake when

The UI a tutorial shows changes: coordinates in `tutorial.py` are measured on one build. The Pine debugger
video was recorded from `main` at 2aebb21e (23 Nov 2026).

## Files

| File | Role |
| --- | --- |
| `make.sh` | Runs narrate, record and compose |
| `tts.py` | One OpenAI speech clip per beat, cached by text |
| `record.py` | Seeds the store, runs the app, drives it, films its window |
| `compose.py` | Cursor, ripples, subtitles, cards, narration mix, encode |
| `probe.py` | Grid-annotated crop of a recorded frame, to measure coordinates |
| `drive.py` | Mouse and keyboard through CoreGraphics, and hiding the pointer |
| `common.py` | Tutorial loader, work directory, Swift helper builds |
| `winlist.swift`, `topat.swift` | Find the app's window; find which window is under a point |
