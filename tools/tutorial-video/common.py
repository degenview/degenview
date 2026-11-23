"""Shared pieces of the tutorial-video tools: the tutorial definition, the work directory, the Swift
helpers and the narration lengths."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DATA_DIR = "Library/Application Support/es.nico.degenview"


def load_tutorial(name):
    path = HERE / "tutorials" / name / "tutorial.py"
    if not path.exists():
        sys.exit(f"no tutorial at {path}")
    spec = importlib.util.spec_from_file_location(f"tutorial_{name}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.DIR = path.parent
    return module


def work_dir(name):
    root = Path(os.environ.get("DEGENVIEW_VIDEO_WORK") or Path(tempfile.gettempdir()) / "degenview-video")
    path = root / name
    path.mkdir(parents=True, exist_ok=True)
    return path


def swift_tool(name, work):
    """Compiles tools/tutorial-video/<name>.swift once and returns the binary."""
    binary = work / "bin" / name
    source = HERE / f"{name}.swift"
    if not binary.exists() or binary.stat().st_mtime < source.stat().st_mtime:
        binary.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["swiftc", "-O", str(source), "-o", str(binary)], check=True)
    return binary


def app_window(winlist, pid):
    """(id, x, y, width, height) of the app's largest visible window, in points, or None."""
    best = None
    for line in subprocess.run([str(winlist), "degenview"], capture_output=True, text=True).stdout.splitlines():
        f = line.split()
        if int(f[1]) == pid and f[7] == "0" and f[8] == "1" and int(f[5]) > 400:
            win = (int(f[0]), int(f[3]), int(f[4]), int(f[5]), int(f[6]))
            if best is None or win[3] * win[4] > best[3] * best[4]:
                best = win
    return best


def narration_length(work, beat, dry):
    """Seconds of speech for a beat: read from the generated clips, or estimated for a dry run."""
    bid, text = beat[0], beat[1]
    if dry:
        return len(text.split()) / 2.7 + 0.3
    return json.loads((work / "tts" / "durations.json").read_text())[bid]
