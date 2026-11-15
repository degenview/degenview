"""Turn a recording into a looping GIF, a black margin around the window, under a size budget.

usage: encode.py <take dir> <out.gif> [max MB, default 8]
Resamples the timestamped frames to a steady rate, then halves the colours and the rate until the
file fits. Needs ffmpeg and Pillow.
"""

import json
import os
import shutil
import subprocess
import sys

from PIL import Image

take, out = sys.argv[1], sys.argv[2]
budget = float(sys.argv[3]) if len(sys.argv) > 3 else 8.0
WIDTH, MARGIN = 1224, 28
times = json.load(open(os.path.join(take, "times.json")))
duration = dict(json.load(open(os.path.join(take, "events.json"))))["end"]


def render(fps, colors):
    frames = os.path.join(take, "gif-frames")
    shutil.rmtree(frames, ignore_errors=True)
    os.makedirs(frames)
    cached = (None, None)
    for k in range(int(duration * fps)):
        newest = max([i for i, t in times if t <= k / fps] or [0])
        if cached[0] != newest:
            shot = Image.open(os.path.join(take, f"f{newest:05d}.jpg")).convert("RGB")
            shot = shot.resize((WIDTH, round(WIDTH * shot.height / shot.width)), Image.LANCZOS)
            canvas = Image.new("RGB", (WIDTH + 2 * MARGIN, shot.height + 2 * MARGIN), (0, 0, 0))
            canvas.paste(shot, (MARGIN, MARGIN))
            cached = (newest, canvas)
        cached[1].save(os.path.join(frames, f"s{k:05d}.png"))
    graph = (f"split[a][b];[a]palettegen=max_colors={colors}:stats_mode=diff[p];"
             "[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i",
                    os.path.join(frames, "s%05d.png"), "-vf", graph, "-loop", "0", out], check=True)
    shutil.rmtree(frames)
    return os.path.getsize(out) / 1e6


for fps, colors in ((10, 256), (10, 128), (8, 128), (6, 96)):
    size = render(fps, colors)
    print(f"{fps} fps, {colors} colours: {size:.1f} MB")
    if size <= budget:
        break
else:
    raise SystemExit(f"still over {budget} MB, shorten the story in drive.py")
