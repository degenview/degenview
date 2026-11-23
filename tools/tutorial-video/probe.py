#!/usr/bin/env python3
"""Measures where things are in a recording, for writing a tutorial's click coordinates.

    probe.py <tutorial> <beat> x0 y0 x1 y1 [seconds-after-beat-end]

Saves <work>/probe-<beat>.png: the window region (points) as it looked at the end of that beat, with a
labelled grid every 20 points (bright every 100). Read the pixel positions of buttons and rows off it.
Needs a take from record.py.
"""
import json
import subprocess
import sys

from PIL import Image, ImageDraw

from common import load_tutorial, work_dir

tutorial_name, beat = sys.argv[1], sys.argv[2]
x0, y0, x1, y1 = (int(v) for v in sys.argv[3:7])
after = float(sys.argv[7]) if len(sys.argv) > 7 else -0.3

work = work_dir(load_tutorial(tutorial_name).NAME)
take = json.loads((work / "take.json").read_text())
width, height = take["window"]
event = next(e for e in take["log"] if e["kind"] == "beat" and e["id"] == beat)
frame = work / "probe-frame.png"
# the capture is the window plus a shadow margin, so find the window like compose.py does: by brightness
subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{event['t'] + event['dur'] + after:.2f}", "-i", str(work / "rec.mov"),
                "-frames:v", "1", str(frame)], check=True)
image = Image.open(frame).convert("RGB")
luminance = image.convert("L").point(lambda v: 255 if v > 14 else 0)
box = luminance.getbbox()
left = box[0] + ((box[2] - box[0]) - 2 * width) // 2
top = box[1] + ((box[3] - box[1]) - 2 * height) // 2
window = image.crop((left, top, left + 2 * width, top + 2 * height)).resize((width, height))
crop = window.crop((x0, y0, x1, y1)).resize(((x1 - x0) * 2, (y1 - y0) * 2))
draw = ImageDraw.Draw(crop)
for x in range((x0 // 20 + 1) * 20, x1, 20):
    draw.line([((x - x0) * 2, 0), ((x - x0) * 2, crop.height)], fill=(255, 255, 0) if x % 100 == 0 else (90, 90, 0))
    if x % 100 == 0:
        draw.text(((x - x0) * 2 + 3, 3), str(x), fill=(255, 255, 0))
for y in range((y0 // 20 + 1) * 20, y1, 20):
    draw.line([(0, (y - y0) * 2), (crop.width, (y - y0) * 2)], fill=(0, 255, 255) if y % 100 == 0 else (0, 90, 90))
    draw.text((3, (y - y0) * 2 + 3), str(y), fill=(0, 255, 255))
out = work / f"probe-{beat}.png"
crop.save(out)
print(out)
