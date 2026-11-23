#!/usr/bin/env python3
"""Narration through the OpenAI speech API: one clip per beat, lengths in <work>/tts/durations.json.

    tts.py <tutorial> [--key-file PATH] [--voice NAME]

The key comes from $OPENAI_API_KEY or --key-file (its first line); it is never printed. The only thing
sent to OpenAI is the beats' spoken text. Clips are cached by text, voice and style, so a re-run only
speaks what changed.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
import wave
from pathlib import Path

from common import load_tutorial, work_dir

MODEL = "gpt-4o-mini-tts"
ENDPOINT = "https://api.openai.com/v1/audio/speech"


def read_key(key_file):
    if key_file:
        return Path(key_file).expanduser().read_text().strip()
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        sys.exit("set OPENAI_API_KEY or pass --key-file")
    return key


def speak(key, text, voice, instructions, destination):
    body = json.dumps({"model": MODEL, "voice": voice, "input": text, "instructions": instructions,
                       "response_format": "wav"}).encode()
    request = urllib.request.Request(ENDPOINT, data=body, headers={
        "Authorization": f"Bearer {key}", "Content-Type": "application/json", "User-Agent": "tutorial-narration/1.0"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                raw = response.read()
            break
        except urllib.error.HTTPError as error:
            if error.code in (429, 500, 502, 503) and attempt < 3:
                time.sleep(3 * (attempt + 1))
                continue
            sys.exit(f"speech request failed: HTTP {error.code} {error.read().decode()[:200]}")
    streamed = destination.with_suffix(".stream.wav")
    streamed.write_bytes(raw)
    # the API streams a wav whose header has no length: rewrite it so the length can be read
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(streamed), "-ar", "24000", "-ac", "1",
                    "-c:a", "pcm_s16le", str(destination)], check=True)
    streamed.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("tutorial")
    parser.add_argument("--key-file")
    parser.add_argument("--voice")
    args = parser.parse_args()
    tutorial = load_tutorial(args.tutorial)
    voice = args.voice or tutorial.VOICE
    key = read_key(args.key_file)
    folder = work_dir(tutorial.NAME) / "tts"
    folder.mkdir(exist_ok=True)

    durations = {}
    for bid, text, _, _ in tutorial.BEATS:
        digest = hashlib.sha1(f"{MODEL}|{voice}|{tutorial.VOICE_INSTRUCTIONS}|{text}".encode()).hexdigest()[:10]
        clip = folder / f"{bid}-{digest}.wav"
        if not clip.exists():
            speak(key, text, voice, tutorial.VOICE_INSTRUCTIONS, clip)
        with wave.open(str(clip)) as wav:
            durations[bid] = wav.getnframes() / wav.getframerate()
        link = folder / f"{bid}.wav"
        link.unlink(missing_ok=True)
        link.symlink_to(clip.name)
        print(f"{bid:4s} {durations[bid]:5.2f}s  {text[:64]}")
    (folder / "durations.json").write_text(json.dumps(durations))
    print(f"{sum(durations.values()):.0f} s of speech in {folder}")


main()
