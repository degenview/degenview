"""Lay key frames of a take on one sheet, to check every beat before encoding.

usage: sheet.py <take dir>      writes <take dir>/sheet.png
"""

import json
import os
import sys

from PIL import Image

take = sys.argv[1]
times = json.load(open(os.path.join(take, "times.json")))
events = dict(json.load(open(os.path.join(take, "events.json"))))
beats = [("clean", 0.3), ("ema", 0.6), ("ichimoku", 1.2), ("macd", 1.4), ("resistance", 0.0),
         ("level", 0.3), ("zone", 1.2), ("typed", 0.1), ("compare", 0.4), ("end", -0.3)]
shots = []
for name, offset in beats:
    frame = min(times, key=lambda t: abs(t[1] - (events[name] + offset)))[0]
    shot = Image.open(os.path.join(take, f"f{frame:05d}.jpg"))
    shots.append(shot.resize((700, round(700 * shot.height / shot.width))))
w, h = shots[0].size
sheet = Image.new("RGB", (2 * w, 5 * h))
for i, shot in enumerate(shots):
    sheet.paste(shot, ((i % 2) * w, (i // 2) * h))
sheet.save(os.path.join(take, "sheet.png"))
print(os.path.join(take, "sheet.png"))
