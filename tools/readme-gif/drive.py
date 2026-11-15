"""Play the README hero story on a running, seeded DegenView while recorder.py films its window.

usage: drive.py <app pid> <window id> <out dir>

Story (about 20 s): a clean chart, then Volume, EMA 20, Ichimoku Cloud and MACD appear, a resistance
line draws itself with a level and a zone, then ETHUSDT is typed into the picker and overlaid with
Compare. Everything but Compare goes through the MCP. Compare has no MCP tool, so it is typed in the
picker (Cmd+K, the symbol, Option+Return). The drawings' anchors come from the live candles, so every
take fits the market of the day. Writes events.json (name, seconds) next to the frames.
"""

import datetime
import json
import os
import subprocess
import sys
import time

import mcp_client as mcp

pid, window, out = sys.argv[1:4]
here = os.path.dirname(os.path.abspath(__file__))
WORKSPACE, CHART = "1", "1"
events = []


def mark(name):
    events.append((name, time.time() - start))
    print(f"{events[-1][1]:5.1f}s  {name}", flush=True)


def system_events(action):
    subprocess.run(["osascript", "-e", f'tell application "System Events" to {action}'], check=True)


def activate():
    system_events(f"set frontmost of (first process whose unix id is {pid}) to true")


def to_ms(iso):
    return int(datetime.datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp() * 1000)


def to_iso(ms):
    return datetime.datetime.fromtimestamp(ms / 1000, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def lerp(a, b, t):
    return a + (b - a) * t


def pivots(bars, column, is_high, width=4):
    """Swing points as (bar index, price): a bar beating the `width` bars on each side."""
    found = []
    for i in range(width, len(bars) - width):
        price = bars[i][column]
        around = [bars[j][column] for j in range(i - width, i + width + 1) if j != i]
        if (price > max(around)) if is_high else (price < min(around)):
            found.append((i, price))
    return found


def plan_drawings(response):
    """Pick a resistance line, a support level and a demand zone from the last 110 daily bars."""
    column = {name: i for i, name in enumerate(response["columns"])}
    bars = response["bars"][-110:]
    time_of = lambda i: bars[i][column["time"]]
    last = len(bars) - 1
    highs = pivots(bars, column["high"], True)
    lows = pivots(bars, column["low"], False)
    if not highs or not lows:
        raise SystemExit("no swing points in the loaded candles, retake later")
    floor_i, floor = min(lows, key=lambda p: p[1])
    # The most recent of the tops within 2% of the highest, so a double top counts as one resistance.
    ceiling = max(price for _, price in highs)
    tops = [p for p in highs if p[1] >= ceiling * 0.98 and p[0] <= last - 8]
    if not tops:
        raise SystemExit("the high is too recent for a resistance line, retake later")
    top_i, top = max(tops, key=lambda p: p[0])
    # The upper tangent from that top: the gentlest slope, so no later high pokes through the line.
    slope = max((bars[j][column["high"]] - top) / (j - top_i) for j in range(top_i + 1, last + 1))
    resistance = ((time_of(top_i), top), (time_of(last), top + slope * (last - top_i)))
    # The lowest swing low between the floor and the top: the support that price has since lost.
    between = [p for p in lows if floor_i < p[0] < top_i] or [(floor_i, floor)]
    level_i, level = min(between, key=lambda p: p[1])
    return {
        "resistance": resistance,
        "level": ((time_of(level_i), level), (time_of(last), level)),
        "zone": ((time_of(floor_i), floor * 0.99), (time_of(last), floor * 1.025)),
    }


def find_id(value):
    """The first drawing id anywhere in an add result."""
    if isinstance(value, dict):
        for key in ("drawing_id", "id"):
            if key in value:
                return value[key]
        value = list(value.values())
    if isinstance(value, list):
        for item in value:
            found = find_id(item)
            if found is not None:
                return found
    return None


def draw(tool, a, b, steps=0, pause=0.11, **style):
    """Add a drawing. With `steps`, its end anchor travels from a to b like a line drawn by hand."""
    (t0, p0), (t1, p1) = (to_ms(a[0]), a[1]), (to_ms(b[0]), b[1])
    first = 0.1 if steps else 1.0
    spec = {"tool": tool, "a": {"time": to_iso(t0), "price": p0},
            "b": {"time": to_iso(lerp(t0, t1, first)), "price": lerp(p0, p1, first)}, **style}
    added = mcp.call("degenview_add_drawings", {"workspace_id": WORKSPACE, "chart_id": CHART, "drawings": [spec]})
    drawing_id = find_id(added)
    for step in range(2, steps + 1):
        time.sleep(pause)
        f = step / steps
        mcp.call("degenview_update_drawing", {
            "workspace_id": WORKSPACE, "chart_id": CHART, "drawing_id": str(drawing_id),
            "b": {"time": to_iso(lerp(t0, t1, f)), "price": lerp(p0, p1, f)}})


candles = mcp.call("degenview_get_candles", {"workspace_id": WORKSPACE, "chart_id": CHART, "limit": 140})
plan = plan_drawings(candles)
library = {s["name"]: s["script_id"] for s in mcp.call("degenview_list_pine_scripts")["scripts"]}
add_script = lambda name: mcp.call("degenview_add_pine_script_to_chart", {
    "workspace_id": WORKSPACE, "chart_id": CHART, "script_id": library[name]})

recorder = subprocess.Popen([sys.executable, os.path.join(here, "recorder.py"), window, out])
start = time.time()
activate(); time.sleep(1.3); mark("clean")
mcp.call("degenview_add_indicator", {"workspace_id": WORKSPACE, "chart_id": CHART, "indicator": "volume"}); mark("volume"); time.sleep(0.7)
mcp.call("degenview_add_indicator", {"workspace_id": WORKSPACE, "chart_id": CHART, "indicator": "ema", "inputs": {"period": 20}}); mark("ema"); time.sleep(0.7)
add_script("ichimoku-cloud"); mark("ichimoku"); time.sleep(1.1)
add_script("macd"); mark("macd"); time.sleep(1.1)
draw("trend_line", *plan["resistance"], steps=10, color="#FF5252", width=2.5, extend_right=True); mark("resistance"); time.sleep(0.5)
draw("trend_line", *plan["level"], steps=10, color="#FFB74D", width=2, dash="dashed"); mark("level"); time.sleep(0.5)
draw("rectangle", *plan["zone"], color="#26A69A", fill_color="#26A69A33", width=1.5); mark("zone"); time.sleep(0.9)
activate(); system_events('keystroke "k" using command down'); mark("picker"); time.sleep(1.0)
for letter in "ETHUSDT":
    system_events(f'keystroke "{letter}"'); time.sleep(0.13)
mark("typed"); time.sleep(2.0)
system_events("key code 36 using option down"); mark("compare"); time.sleep(3.2)
mark("end")
open(os.path.join(out, "STOP"), "w").close()
recorder.wait()
with open(os.path.join(out, "events.json"), "w") as file:
    json.dump(events, file)
