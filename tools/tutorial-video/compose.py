#!/usr/bin/env python3
"""Composes the video: title card, the screen recording with a large cursor, click ripples and burned-in
subtitles, an end card, and the narration mixed in at each beat.

    compose.py <tutorial> [--dry] [--seconds N]

--dry composes silently from estimated beat lengths (to check the picture); --seconds N stops after N
seconds of output. Needs <work>/rec.mov and take.json (record.py) and <work>/tts (tts.py). Writes
<work>/<tutorial>.mp4 and <work>/subtitles.srt. Requires Pillow, numpy and ffmpeg.
"""
import argparse
import bisect
import io
import json
import re
import subprocess
import sys
import wave

import numpy as np
from PIL import Image, ImageDraw, ImageFont

from common import load_tutorial, narration_length, work_dir

FPS, W, H = 30, 1920, 1080
SUBTITLE_STRIP = 90            # pixels kept free under the window for subtitles
BG = (14, 17, 20)
INK, DIM, LINE = (236, 239, 242), (140, 148, 158), (44, 50, 58)
HALO, RIPPLE = (255, 214, 10), (255, 160, 20)
FONT = "/System/Library/Fonts/SFNS.ttf"
SENTENCE_GAP = 0.35            # seconds between spoken sentences on the cards


def font(size, weight="Regular"):
    face = ImageFont.truetype(FONT, size)
    try:
        face.set_variation_by_name(weight)
    except OSError:
        pass
    return face


class Capture:
    """The recording: where the window sits in it, and when the first click shows up."""

    def __init__(self, path, window, sync_region, first_click):
        self.path = path
        out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                              "stream=width,height,duration", "-of", "json", str(path)],
                             capture_output=True, text=True).stdout
        stream = json.loads(out)["streams"][0]
        self.width, self.height, self.duration = int(stream["width"]), int(stream["height"]), float(stream["duration"])
        self.cw, self.ch = window[0] * 2, window[1] * 2         # the window, in device pixels (Retina)
        self.crop = self._find_crop()
        self.offset = self._find_offset(window, sync_region, first_click)

    def _frame(self, t):
        raw = subprocess.run(["ffmpeg", "-v", "error", "-ss", str(t), "-i", str(self.path), "-frames:v", "1",
                              "-f", "image2pipe", "-vcodec", "png", "-"], capture_output=True).stdout
        return np.array(Image.open(io.BytesIO(raw)).convert("RGB"))

    def _find_crop(self):
        """The window's box inside the frame: the capture adds a black margin for the shadow."""
        frame = self._frame(2.0).max(axis=2)
        cols = np.where(frame[self.height // 2] > 14)[0]
        rows = np.where(frame[:, self.width // 2] > 14)[0]
        x0, x1, y0, y1 = cols.min(), cols.max() + 1, rows.min(), rows.max() + 1
        if abs((x1 - x0) - self.cw) > 40 or abs((y1 - y0) - self.ch) > 40:
            sys.exit(f"unexpected capture box {x0},{y0}..{x1},{y1} for a {self.cw}x{self.ch} window (needs a Retina display)")
        return x0 + ((x1 - x0) - self.cw) // 2, y0 + ((y1 - y0) - self.ch) // 2

    def _find_offset(self, window, region, first_click):
        """Video time minus action-log time: when the first click's change appears on screen, against when
        it was clicked."""
        scale = 400 / window[0]
        w, h = 400, round(window[1] * scale)
        raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(self.path), "-vf",
                              f"crop={self.cw}:{self.ch}:{self.crop[0]}:{self.crop[1]},scale={w}:{h},fps={FPS},format=gray",
                              "-f", "rawvideo", "-"], capture_output=True).stdout
        count = len(raw) // (w * h)
        frames = np.frombuffer(raw, np.uint8)[: count * w * h].reshape(count, h, w).astype(np.int16)
        x0, y0, x1, y1 = (round(v * scale) for v in region)
        patch = frames[:, y0:y1, x0:x1]
        reference = FPS                                          # one second in: before anything moves
        change = np.abs(patch - patch[reference]).mean(axis=(1, 2))
        near = [k for k in np.where((np.arange(count) > reference) & (change > 2.0))[0]
                if abs(k / FPS - (first_click + 1.0)) < 6]
        if not near:
            sys.exit("could not find the first click's effect in the recording (check SYNC_REGION)")
        return near[0] / FPS - first_click


def chunks(text, limit=62):
    """Subtitle lines: sentences, then clauses, then words, none longer than `limit` characters."""
    out = []
    for sentence in (s.strip() for s in re.split(r"(?<=[.!?:])\s+", text) if s.strip()):
        current = ""
        for clause in re.split(r"(?<=,)\s+", sentence):
            if len(clause) > limit:
                if current:
                    out.append(current)
                    current = ""
                words, line = clause.split(), ""
                for word in words:
                    if len(line) + len(word) + 1 <= limit:
                        line = f"{line} {word}".strip()
                    else:
                        out.append(line)
                        line = word
                clause = line
            if len(current) + len(clause) + 1 <= limit:
                current = f"{current} {clause}".strip()
            else:
                out.append(current)
                current = clause
        if current:
            out.append(current)
    return out


def srt_time(x):
    hours, rest = divmod(int(x), 3600)
    minutes, seconds = divmod(rest, 60)
    return f"{hours:02d}:{minutes:02d}:{seconds:02d},{int((x % 1) * 1000):03d}"


class Overlay:
    """The drawn cursor, click ripples, key caps and subtitles."""

    def __init__(self, log, origin, scale, left, keycaps):
        self.moves = [(e["t"], e["x"], e["y"]) for e in log if e["kind"] == "move"]
        self.move_times = [m[0] for m in self.moves]
        self.downs = [e["t"] for e in log if e["kind"] == "down"]
        self.keys = [(e["t"], e["name"]) for e in log if e["kind"] == "key"]
        self.origin, self.scale, self.left, self.keycaps = origin, scale, left, keycaps
        self.sprites = {False: self._cursor(False), True: self._cursor(True)}
        self.subtitle_font = font(38, "Medium")

    @staticmethod
    def _cursor(pressed):
        ss = 4
        size = 150 * ss
        c = size // 2
        image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        d = ImageDraw.Draw(image)
        r = (30 if pressed else 38) * ss
        d.ellipse([c - r, c - r, c + r, c + r], fill=HALO + (62,), outline=HALO + (210,), width=3 * ss)
        k = (2.25 if pressed else 2.6) * ss
        points = [(0, 0), (0, 17), (4.4, 13.2), (7.6, 20.5), (10.6, 19.2), (7.4, 12.2), (13, 12)]
        arrow = [(c + x * k, c + y * k) for x, y in points]
        d.polygon([(x + 2.5 * ss, y + 3 * ss) for x, y in arrow], fill=(0, 0, 0, 90))
        d.polygon(arrow, fill=(255, 255, 255, 255), outline=(10, 10, 10, 255), width=int(2.2 * ss))
        return image.resize((150, 150), Image.LANCZOS)

    def cursor_at(self, t):
        """Window-relative point under the cursor at log time t."""
        if not self.moves or t < self.moves[0][0]:
            return (self.origin[0] + 800.0, self.origin[1] + 380.0)
        i = bisect.bisect_right(self.move_times, t) - 1
        if i >= len(self.moves) - 1:
            return self.moves[-1][1:]
        a, b = self.moves[i], self.moves[i + 1]
        u = min(1.0, max(0.0, (t - a[0]) / (b[0] - a[0]))) if b[0] > a[0] else 0.0
        return (a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u)

    def _px(self, point):
        return self.left + (point[0] - self.origin[0]) * self.scale, (point[1] - self.origin[1]) * self.scale

    def draw(self, image, t):
        for down in self.downs:
            for delay in (0.0, 0.14):
                age = t - down - delay
                if 0 <= age < 0.6:
                    u = age / 0.6
                    cx, cy = self._px(self.cursor_at(down))
                    radius = 16 + 62 * (1 - (1 - u) ** 2)
                    layer = Image.new("RGBA", (200, 200), (0, 0, 0, 0))
                    ImageDraw.Draw(layer).ellipse([100 - radius, 100 - radius, 100 + radius, 100 + radius],
                                                  outline=RIPPLE + (int(235 * (1 - u)),), width=max(2, int(7 * (1 - u))))
                    image.paste(layer, (int(cx - 100), int(cy - 100)), layer)
        cx, cy = self._px(self.cursor_at(t))
        pressed = any(d <= t <= d + 0.14 for d in self.downs)
        sprite = self.sprites[pressed]
        image.paste(sprite, (int(cx - 75), int(cy - 75)), sprite)
        for key_time, name in self.keys:
            age = t - key_time
            if 0 <= age < 1.1 and name in self.keycaps:
                self._pill(image, self.keycaps[name], (W // 2, 925), 44, 1 - max(0.0, (age - 0.7) / 0.4))

    @staticmethod
    def _pill(image, text, center, size, alpha):
        face = font(size, "Bold")
        layer = Image.new("RGBA", (int(face.getlength(text) + 60), size + 40), (0, 0, 0, 0))
        d = ImageDraw.Draw(layer)
        d.rounded_rectangle([0, 0, layer.width - 1, layer.height - 1], radius=16, fill=(18, 22, 26, int(215 * alpha)),
                            outline=(90, 98, 108, int(200 * alpha)), width=2)
        d.text((30, 14), text, font=face, fill=INK + (int(255 * alpha),))
        image.paste(layer, (int(center[0] - layer.width / 2), int(center[1] - layer.height / 2)), layer)

    def subtitle(self, image, lines, starts, t):
        i = bisect.bisect_right(starts, t) - 1
        if i < 0 or t > lines[i][1] + 0.25:
            return
        text = lines[i][2]
        width = self.subtitle_font.getlength(text)
        x, y = (W - width) / 2, 1010
        d = ImageDraw.Draw(image)
        d.rounded_rectangle([x - 26, y - 8, x + width + 26, y + 56], radius=14, fill=(24, 29, 34), outline=(52, 59, 68), width=2)
        d.text((x, y), text, font=self.subtitle_font, fill=(245, 247, 250))


def patch_traffic_lights(image, left, scale):
    """macOS draws its screen-share pill over the window's traffic lights in a capture: paint them back."""
    d = ImageDraw.Draw(image)
    background = image.getpixel((int(left + 78 * scale), int(22 * scale)))
    d.rectangle([left + 4 * scale, 7 * scale, left + 76 * scale, 34 * scale], fill=background)
    for x, colour in ((16, (255, 95, 87)), (39, (254, 188, 46)), (62, (40, 200, 64))):
        cx, cy, r = left + x * scale, 20.5 * scale, 6.2 * scale
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=colour)


def title_card(tutorial):
    image = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(image)
    d.text((W / 2, 440), tutorial.TITLE, font=font(104, "Bold"), fill=INK, anchor="mm")
    d.text((W / 2, 556), tutorial.SUBTITLE, font=font(44), fill=DIM, anchor="mm")
    d.line([(W / 2 - 60, 640), (W / 2 + 60, 640)], fill=LINE, width=4)
    d.text((W / 2, 716), tutorial.BRAND, font=font(40, "Semibold"), fill=INK, anchor="mm")
    return image


def end_card(card):
    image = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(image)
    d.text((W / 2, 150), card["title"], font=font(70, "Bold"), fill=INK, anchor="mm")
    columns = (400, 1100, 1480)
    y = 250
    for x, heading in zip(columns, card["columns"]):
        d.text((x, y), heading, font=font(34, "Semibold"), fill=DIM, anchor="lm")
    y += 36
    d.line([(columns[0], y), (W - 400, y)], fill=LINE, width=2)
    for row in card["rows"]:
        y += 74
        for x, cell in zip(columns, row):
            d.text((x, y), cell, font=font(42), fill=INK, anchor="lm")
        d.line([(columns[0], y + 37), (W - 400, y + 37)], fill=(26, 31, 36), width=2)
    return image


def fade(image, alpha):
    return image.copy() if alpha >= 1 else Image.blend(Image.new("RGB", (W, H), BG), image, max(0.0, alpha))


def mix_narration(folder, starts, total, destination):
    rate = 24000
    mix = np.zeros(int(total * rate) + rate, np.float32)
    for bid, start in starts.items():
        with wave.open(str(folder / f"{bid}.wav")) as clip:
            pcm = np.frombuffer(clip.readframes(clip.getnframes()), np.int16).astype(np.float32) / 32768
        i = int(start * rate)
        mix[i:i + len(pcm)] += pcm[: max(0, len(mix) - i)]
    peak = np.abs(mix).max()
    if peak > 0.95:
        mix *= 0.95 / peak
    with wave.open(str(destination), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes((mix * 32767).astype(np.int16).tobytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("tutorial")
    parser.add_argument("--dry", action="store_true")
    parser.add_argument("--seconds", type=float)
    args = parser.parse_args()
    tutorial = load_tutorial(args.tutorial)
    work = work_dir(tutorial.NAME)
    take = json.loads((work / "take.json").read_text())
    log, origin, window = take["log"], tuple(take["origin"]), tuple(take["window"])
    beat_events = {e["id"]: e for e in log if e["kind"] == "beat"}

    scale = min((W - 160) / window[0], (H - SUBTITLE_STRIP) / window[1])
    frame_w, frame_h = round(window[0] * scale), round(window[1] * scale)
    left = (W - frame_w) // 2
    capture = Capture(work / "rec.mov", window, tutorial.SYNC_REGION, next(e["t"] for e in log if e["kind"] == "down"))
    print(f"recording {capture.width}x{capture.height}, {capture.duration:.0f} s; sync offset {capture.offset:.2f} s")

    by_scene = {scene: [b for b in tutorial.BEATS if b[3] == scene and (scene != "screen" or b[0] in beat_events)]
                for scene in ("title", "screen", "end")}
    length = lambda beat: narration_length(work, beat, args.dry)       # noqa: E731
    starts, t = {}, 0.7
    for beat in by_scene["title"]:
        starts[beat[0]] = t
        t += length(beat) + SENTENCE_GAP
    title_end = t + 0.6
    first = beat_events[by_scene["screen"][0][0]]
    video_start = max(0.0, first["t"] + capture.offset - 0.6)
    for beat in by_scene["screen"]:
        starts[beat[0]] = title_end + beat_events[beat[0]]["t"] + capture.offset - video_start
    last_beat = by_scene["screen"][-1]
    last = beat_events[last_beat[0]]
    screen_len = last["t"] + max(last["dur"], length(last_beat) + 0.45) + 1.2 + capture.offset - video_start
    screen_len = min(screen_len, capture.duration - video_start - 0.05)
    screen_end = title_end + screen_len
    t = screen_end + 0.7
    for beat in by_scene["end"]:
        starts[beat[0]] = t
        t += length(beat) + SENTENCE_GAP
    total = min(t + 1.6, args.seconds) if args.seconds else t + 1.6
    print(f"title {title_end:.0f} s, screen {screen_len:.0f} s, total {total:.0f} s")

    lines = []
    for beat in tutorial.BEATS:
        if beat[0] not in starts:
            continue
        parts = chunks(beat[1])
        start, spoken = starts[beat[0]], length(beat)
        for part in parts:
            end = start + spoken * len(part) / sum(len(p) for p in parts)
            lines.append((start, end, part))
            start = end
    starts_sorted = [line[0] for line in lines]
    if not args.dry:
        (work / "subtitles.srt").write_text("".join(
            f"{i}\n{srt_time(a)} --> {srt_time(b)}\n{text}\n\n" for i, (a, b, text) in enumerate(lines, 1)))
        mix_narration(work / "tts", {k: v for k, v in starts.items()}, total, work / "narration.wav")

    destination = work / ("dry.mp4" if args.dry else f"{tutorial.NAME}.mp4")
    encode = ["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-"]
    if not args.dry:
        encode += ["-i", str(work / "narration.wav")]
    encode += ["-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p", "-c:v", "h264_videotoolbox", "-b:v", "12M",
               "-profile:v", "high", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
               "-color_range", "tv", "-movflags", "+faststart"]
    if not args.dry:
        encode += ["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-shortest"]
    encoder = subprocess.Popen(encode + [str(destination)], stdin=subprocess.PIPE)
    decoder = subprocess.Popen(
        ["ffmpeg", "-v", "error", "-ss", f"{video_start:.3f}", "-t", f"{screen_len:.3f}", "-i", str(capture.path), "-vf",
         f"crop={capture.cw}:{capture.ch}:{capture.crop[0]}:{capture.crop[1]},scale={frame_w}:{frame_h}:flags=lanczos:"
         f"in_color_matrix=bt709:in_range=tv:out_range=pc,fps={FPS}", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        stdout=subprocess.PIPE)

    overlay = Overlay(log, origin, scale, left, tutorial.KEYCAPS)
    title, ending = title_card(tutorial), end_card(tutorial.END_CARD)
    background, last_frame = Image.new("RGB", (W, H), BG), b""
    for n in range(int(total * FPS)):
        t = n / FPS
        if t < title_end:
            image = fade(title, min(1.0, t / 0.5) * min(1.0, (title_end - t) / 0.4))
        elif t < screen_end:
            raw = decoder.stdout.read(frame_w * frame_h * 3)
            last_frame = raw if len(raw) == frame_w * frame_h * 3 else last_frame
            image = background.copy()
            image.paste(Image.frombuffer("RGB", (frame_w, frame_h), last_frame, "raw", "RGB", 0, 1), (left, 0))
            patch_traffic_lights(image, left, scale)
            overlay.draw(image, (t - title_end) + video_start - capture.offset)
            edge = min(1.0, (t - title_end) / 0.4, (screen_end - t) / 0.4)
            if edge < 1.0:
                image = fade(image, edge)
        else:
            image = fade(ending, min(1.0, (t - screen_end) / 0.5) * min(1.0, (total - t) / 0.6))
        overlay.subtitle(image, lines, starts_sorted, t)
        encoder.stdin.write(image.tobytes())
    encoder.stdin.close()
    encoder.wait()
    decoder.kill()
    print("wrote", destination)


main()
