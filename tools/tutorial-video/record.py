#!/usr/bin/env python3
"""Records the screen scenes of a tutorial: starts a clean throwaway DegenView, drives it through the
beats with the real mouse and keyboard, and films only the app's window.

    record.py <tutorial> [--dry] [--beats N] [--bin PATH]

--dry times the beats from a words-per-second estimate (no narration clips needed); --beats N plays only
the first N screen beats. Writes <work>/rec.mov and <work>/take.json (the action log and beat timeline,
all times relative to the start of the first beat).

The app's window is frontmost for the take and the pointer is hidden, so leave the mouse and keyboard
alone. Every click is refused unless the app's own window is the topmost one under the pointer.
"""
import argparse
import json
import math
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

import drive
from common import DATA_DIR, REPO, app_window, load_tutorial, narration_length, swift_tool, work_dir

LEAD, TAIL, GAP = 1.5, 2.0, 0.45     # seconds before the first beat, after the last, after each sentence
STAGING = Path.home() / "Library/Group Containers/group.com.apple.screencapture/ScreenRecordings"


def move_duration(a, b):
    return min(1.0, 0.4 + math.hypot(a[0] - b[0], a[1] - b[1]) / 1800)


def acts_duration(acts, start):
    """How long a beat's acts take from `start`, and where the cursor ends."""
    pos, total = start, 0.0
    for act in acts:
        if act[0] in ("go", "click"):
            target = (act[1], act[2])
            total += move_duration(pos, target) + (0.35 if act[0] == "click" else 0.05)
            pos = target
        elif act[0] == "wait":
            total += act[1]
        elif act[0] == "type":
            total += 0.09 * len(act[1]) + 0.2
        elif act[0] == "key":
            total += 0.3
    return total, pos


def seed_home(tutorial, home):
    """A fresh HOME: the tutorial's scripts, its open tab and its settings."""
    data = home / DATA_DIR
    (data / "Scripts").mkdir(parents=True)
    for name in tutorial.SCRIPTS:
        shutil.copy(tutorial.DIR / name, data / "Scripts" / name)
    store = sqlite3.connect(data / "degenview.sqlite")
    store.execute("CREATE TABLE IF NOT EXISTS tab (id TEXT PRIMARY KEY, position INTEGER NOT NULL, payload TEXT NOT NULL)")
    store.execute("CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    store.execute("INSERT INTO tab VALUES (?, 0, ?)", (str(tutorial.TAB["id"]), json.dumps(tutorial.TAB)))
    store.executemany("INSERT INTO setting VALUES (?, ?)", tutorial.SETTINGS.items())
    store.commit()
    store.close()


def plan_timeline(beats, dry, work):
    pos, t = (800.0, 380.0), LEAD
    timeline = []
    for beat in beats:
        audio = narration_length(work, beat, dry)
        length, pos = acts_duration(beat[2], pos)
        duration = max(audio + GAP, length + 0.9)
        timeline.append({"id": beat[0], "start": t, "dur": duration, "audio": audio, "acts": beat[2]})
        t += duration
    return timeline, t + TAIL


def finish_capture(capture, destination, started):
    """screencapture may leave the movie in its staging folder; move it to `destination`."""
    capture.send_signal(signal.SIGINT)
    try:
        capture.wait(timeout=30)
    except subprocess.TimeoutExpired:
        capture.kill()
    for _ in range(40):
        if destination.exists():
            return
        fresh = [p for p in STAGING.glob("*.mov") if p.stat().st_mtime >= started]
        if fresh:
            shutil.move(max(fresh, key=lambda p: p.stat().st_mtime), destination)
            return
        time.sleep(0.5)
    sys.exit("the recording was not saved")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("tutorial")
    parser.add_argument("--dry", action="store_true")
    parser.add_argument("--beats", type=int)
    parser.add_argument("--bin", default=os.environ.get("DEGENVIEW_BIN", str(REPO / "target/release/degenview")))
    args = parser.parse_args()
    tutorial = load_tutorial(args.tutorial)
    work = work_dir(tutorial.NAME)
    winlist, topat = swift_tool("winlist", work), swift_tool("topat", work)

    beats = [b for b in tutorial.BEATS if b[3] == "screen"][: args.beats]
    timeline, total = plan_timeline(beats, args.dry, work)
    print(f"{len(timeline)} beats, {total:.0f} s")

    home = work / "home"
    shutil.rmtree(home, ignore_errors=True)
    seed_home(tutorial, home)
    environment = dict(os.environ, HOME=str(home), DEGENVIEW_NO_KEYCHAIN="1", DEGENVIEW_NO_NOTIFICATIONS="1",
                       DEGENVIEW_CACHE_DIR=str(work / "cache"))
    app = subprocess.Popen([args.bin], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    destination = work / "rec.mov"
    destination.unlink(missing_ok=True)

    def topmost(x, y):
        out = subprocess.run([str(topat), str(x), str(y)], capture_output=True, text=True).stdout.split()
        return bool(out) and int(out[0]) == app.pid

    def front():
        subprocess.run(["osascript", "-e", f'tell application "System Events" to set frontmost of '
                        f'(first process whose unix id is {app.pid}) to true'], check=True)

    try:
        window = None
        for _ in range(60):
            time.sleep(0.5)
            if window := app_window(winlist, app.pid):
                break
        if not window:
            sys.exit("the app's window did not appear")
        front()
        time.sleep(2)
        wid, ox, oy, width, height = window
        want_w, want_h = tutorial.WINDOW
        helper = drive.Driver(lambda *a, **k: None)
        for _ in range(4):      # the app opens at its own size: drag the bottom-right corner (not recorded)
            if (width, height) == (want_w, want_h):
                break
            corner = (ox + width - 2, oy + height - 2)
            if not topmost(*corner):
                sys.exit("the app's window is not on top at its corner")
            helper.drag(*corner, ox + want_w - 2, oy + want_h - 2, 0.6)
            time.sleep(2)
            wid, ox, oy, width, height = app_window(winlist, app.pid)
        if (width, height) != (want_w, want_h):
            sys.exit(f"could not size the window to {want_w}x{want_h}, it is {width}x{height}")
        front()
        time.sleep(4)           # candles load

        started = time.time()
        capture = subprocess.Popen(["screencapture", "-v", "-x", "-V", str(int(total + 20)), "-l", str(wid),
                                    str(destination)])
        log, t0 = [], [0.0]

        def record(kind, **fields):
            log.append({"t": time.monotonic() - t0[0], "kind": kind, **fields})

        driver = drive.Driver(record)
        time.sleep(0.6)
        drive.hide_cursor()
        time.sleep(0.4)
        t0[0] = time.monotonic()
        driver.jump(ox + 800, oy + 380)
        record("start")

        def sleep_until(t):
            while time.monotonic() - t0[0] < t:
                time.sleep(0.005)

        problem, cursor = None, (800.0, 380.0)
        for beat in timeline:
            sleep_until(beat["start"])
            record("beat", id=beat["id"], dur=beat["dur"], audio=beat["audio"])
            sleep_until(beat["start"] + 0.3)
            for act in beat["acts"]:
                if act[0] in ("go", "click"):
                    target = (act[1], act[2])
                    if not topmost(ox + target[0], oy + target[1]):
                        problem = f"the app's window is not on top at {target} in beat {beat['id']}"
                        break
                    driver.move(ox + target[0], oy + target[1], move_duration(cursor, target))
                    cursor = target
                    if act[0] == "click":
                        time.sleep(0.12)
                        driver.click()
                        time.sleep(0.12)
                elif act[0] == "wait":
                    time.sleep(act[1])
                elif act[0] == "type":
                    driver.type_text(act[1])
                elif act[0] == "key":
                    driver.key(act[1], act[2:])
            if problem:
                break
        if not problem:
            sleep_until(total - 1.0)
        drive.show_cursor()
        finish_capture(capture, destination, started)
        (work / "take.json").write_text(json.dumps({
            "total": total, "origin": [ox, oy], "window": [width, height], "log": log,
            "timeline": [{k: v for k, v in b.items() if k != "acts"} for b in timeline]}))
        if problem:
            sys.exit("stopped: " + problem)
        print("recorded", destination)
    finally:
        drive.show_cursor()
        app.terminate()


main()
