"""Capture one window, by id, as fast as screencapture allows, until a STOP file appears.

usage: recorder.py <window id> <out dir>
Writes f00000.jpg, f00001.jpg, ... and times.json ([[frame, seconds since start], ...]).
Only that window is captured, never the screen or a region.
"""

import json
import os
import subprocess
import sys
import time

window, out = sys.argv[1], sys.argv[2]
stop = os.path.join(out, "STOP")
start = time.time()
times = []
while not os.path.exists(stop):
    frame = len(times)
    subprocess.run(["screencapture", "-x", "-o", "-t", "jpg", "-l", window,
                    os.path.join(out, f"f{frame:05d}.jpg")], check=False)
    times.append((frame, time.time() - start))
with open(os.path.join(out, "times.json"), "w") as file:
    json.dump(times, file)
