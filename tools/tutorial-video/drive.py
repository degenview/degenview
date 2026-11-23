"""Mouse and keyboard for the recorder: CoreGraphics events through ctypes (macOS), and hiding the
system pointer so only the drawn cursor shows in the capture."""
import ctypes
import math
import time

_cg = ctypes.CDLL("/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices")
_cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
_core = ctypes.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")


class _Point(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


_cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
_cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, _Point, ctypes.c_uint32]
_cg.CGEventCreateKeyboardEvent.restype = ctypes.c_void_p
_cg.CGEventCreateKeyboardEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool]
_cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
_cg.CGEventSetFlags.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
_cg.CGEventSetIntegerValueField.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int64]
_cg.CGEventKeyboardSetUnicodeString.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_uint16)]
_cf.CFRelease.argtypes = [ctypes.c_void_p]
_cf.CFStringCreateWithCString.restype = ctypes.c_void_p
_cf.CFStringCreateWithCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_uint32]
_core.CGSMainConnectionID.restype = ctypes.c_int
_core.CGSSetConnectionProperty.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p]
_core.CGDisplayHideCursor.argtypes = [ctypes.c_uint32]
_core.CGDisplayShowCursor.argtypes = [ctypes.c_uint32]

_MOVE, _DOWN, _UP, _DRAG = 5, 1, 2, 6
_CLICK_STATE_FIELD = 1
_UTF8 = 0x08000100
FLAGS = {"shift": 0x20000, "ctrl": 0x40000, "alt": 0x80000, "cmd": 0x100000}
KEYS = {"return": 36, "esc": 53, "tab": 48, "space": 49, "delete": 51, "left": 123, "right": 124, "down": 125,
        "up": 126, "f5": 96, "f6": 97, "f7": 98, "f8": 100, "f9": 101, "f10": 109, "f11": 103}


def _post(event):
    _cg.CGEventPost(0, event)
    _cf.CFRelease(event)


def _mouse(kind, x, y):
    return _cg.CGEventCreateMouseEvent(None, kind, _Point(x, y), 0)


def hide_cursor():
    """Hides the system pointer for as long as this process lives (everywhere, not only in our windows)."""
    connection = _core.CGSMainConnectionID()
    key = _cf.CFStringCreateWithCString(None, b"SetsCursorInBackground", _UTF8)
    _core.CGSSetConnectionProperty(connection, connection, key, ctypes.c_void_p.in_dll(_cf, "kCFBooleanTrue"))
    _core.CGDisplayHideCursor(0)


def show_cursor():
    _core.CGDisplayShowCursor(0)


class Driver:
    """Moves the real pointer and types; `log(kind, **fields)` receives every step for the overlay."""

    def __init__(self, log):
        self.log = log
        self.x = self.y = 0.0

    def jump(self, x, y):
        self.x, self.y = x, y
        _post(_mouse(_MOVE, x, y))

    def move(self, x, y, duration):
        """An eased move with a slight arc, so the path reads as a hand and not a ruler."""
        x0, y0 = self.x, self.y
        dx, dy = x - x0, y - y0
        distance = math.hypot(dx, dy) or 1.0
        nx, ny = -dy / distance, dx / distance
        bend = min(40.0, distance * 0.06)
        start = time.monotonic()
        while (progress := (time.monotonic() - start) / duration) < 1.0:
            eased = progress * progress * (3 - 2 * progress)
            offset = math.sin(math.pi * eased) * bend
            px, py = x0 + dx * eased + nx * offset, y0 + dy * eased + ny * offset
            self.jump(px, py)
            self.log("move", x=px, y=py)
            time.sleep(1 / 120)
        self.jump(x, y)
        self.log("move", x=x, y=y)

    def click(self):
        for kind, name in ((_DOWN, "down"), (_UP, "up")):
            event = _mouse(kind, self.x, self.y)
            _cg.CGEventSetIntegerValueField(event, _CLICK_STATE_FIELD, 1)
            self.log(name, x=self.x, y=self.y)
            _post(event)
            time.sleep(0.09)

    def drag(self, x0, y0, x1, y1, duration):
        """Press at (x0, y0), drag to (x1, y1), release: used once, to size the window."""
        self.jump(x0, y0)
        time.sleep(0.2)
        _post(_mouse(_DOWN, x0, y0))
        start = time.monotonic()
        while (progress := (time.monotonic() - start) / duration) < 1.0:
            self.x, self.y = x0 + (x1 - x0) * progress, y0 + (y1 - y0) * progress
            _post(_mouse(_DRAG, self.x, self.y))
            time.sleep(1 / 120)
        _post(_mouse(_DRAG, x1, y1))
        _post(_mouse(_UP, x1, y1))
        self.x, self.y = x1, y1

    def key(self, name, modifiers=()):
        flags = 0
        for modifier in modifiers:
            flags |= FLAGS[modifier]
        self.log("key", name=name, mods=list(modifiers))
        for down in (True, False):
            event = _cg.CGEventCreateKeyboardEvent(None, KEYS[name], down)
            _cg.CGEventSetFlags(event, flags)
            _post(event)
            time.sleep(0.04)

    def type_text(self, text, gap=0.07):
        for char in text:
            unit = (ctypes.c_uint16 * 1)(ord(char))
            for down in (True, False):
                event = _cg.CGEventCreateKeyboardEvent(None, 0, down)
                _cg.CGEventKeyboardSetUnicodeString(event, 1, unit)
                _post(event)
                time.sleep(0.01)
            time.sleep(gap)
