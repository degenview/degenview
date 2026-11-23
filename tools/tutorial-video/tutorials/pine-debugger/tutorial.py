"""Tutorial: debugging a Pine script in the Pine Studio (breakpoints, stepping, watches).

The recorder drives a throwaway DegenView through BEATS while the compositor adds the voice, the
cursor and the subtitles. One beat is one spoken sentence plus the mouse actions made while it is said.

Coordinates are points inside the app window (1600x900), measured on a release build; retake them with
`probe.py` when the debugger panel changes. Acts:
    ("go", x, y)       move there            ("click", x, y)   move there and click
    ("wait", seconds)  ("type", text)         ("key", name, *modifiers)
"""
import json

NAME = "pine-debugger"
TITLE = "Debugging Pine scripts"
SUBTITLE = "Breakpoints, stepping and live variables in Pine Studio"
BRAND = "DegenView"

WINDOW = (1600, 900)                 # points; the recorder resizes the app's window to this
SCRIPTS = ["Candle Momentum.pine"]   # copied into the throwaway script library
TAB = {"id": 2, "kind": "pine"}      # the app opens on this tab
SETTINGS = {
    "scripts.demosSeeded": "true",
    "scripts.swiftInputsImported": "true",
    # panel closed, no script selected; the debugger panel opens 394 pt tall
    "pineStudio.layout": json.dumps({
        "chartVisible": True, "chartPosition": "left", "chartWidth": 0.0, "chartHeight": 0.0,
        "drawerOpen": True, "drawerTab": "inputs", "drawerHeight": 0.0, "selectedScriptId": None,
        "debugOpen": False, "debugHeight": 394.17578, "debugSideWidth": 0.0, "debugSections": 7,
        "aiOpen": False, "aiHeight": 0.0,
    }),
}

# The first click of the first beat must change this region (points: x0, y0, x1, y1): the video is
# aligned to the action log by finding that change. Here the Scripts menu opening.
SYNC_REGION = (187, 85, 538, 225)

VOICE = "ash"
VOICE_INSTRUCTIONS = ("Warm, clear, friendly software tutorial narrator. Natural, unhurried pace, slightly "
                      "upbeat, crisp pronunciation of technical terms, brief natural pauses at commas.")

KEYCAPS = {"return": "Enter  ⏎"}     # keys shown on screen while pressed

END_CARD = {
    "title": "Debugger shortcuts",
    "columns": ("", "macOS", "Windows / Linux"),
    "rows": [("Start / Continue", "⌃⌘Y", "F5"), ("Pause", "⌃⌘P", "F6"), ("Step Over", "F6", "F10"),
             ("Step Into", "F7", "F11"), ("Step Out", "F8", "⇧F11"), ("Step Bar", "⌃⌘B", "F7"),
             ("Stop", "⌘.", "⇧F5"), ("Restart", "⇧⌘R", "Ctrl+⇧F5"), ("Toggle breakpoint", "F9", "F9")],
}

# the debugger's controls row, once the panel is open
Y = 524
CONT, PAUSE, OVER, INTO, OUT, BAR, RESTART, STOP = 822, 854, 890, 922, 954, 986, 1018, 1050

# (id, spoken text, acts, scene) with scene "title", "screen" or "end"
BEATS = [
    ("i1", "Welcome! In this tutorial you'll learn how to debug a Pine script in DegenView's Pine Studio.", [], "title"),
    ("i2", "We'll set breakpoints, step through the code line by line, and inspect variables while the script is running.", [], "title"),

    ("b1", "Here's the Pine Studio. Let's open a small script called Candle Momentum.",
     [("go", 800, 400), ("click", 800, 503), ("wait", 0.5), ("click", 285, 162)], "screen"),
    ("b2", "It averages the body of the last few candles inside a function called avgBody,",
     [("go", 1010, 215), ("go", 1010, 292)], "screen"),
    ("b3", "counts bullish candles in a variable that persists from bar to bar,",
     [("go", 1010, 326)], "screen"),
    ("b4", "and plots both results.",
     [("go", 1010, 430)], "screen"),
    ("b5", "When a script like this gives a strange result, all you see is the final plot. Print statements help, but you have to edit the code and run it again every time.",
     [("go", 420, 495), ("wait", 1.0), ("go", 600, 520)], "screen"),
    ("b6", "A debugger is better. It pauses the script in the middle of a run, so you can look at every value, and then move forward one step at a time.",
     [("go", 1150, 250)], "screen"),

    ("b7", "To pause the script, set a breakpoint. Click the line number where you want it to stop. I'll pick line fourteen, where avgBody is called.",
     [("click", 850, 342)], "screen"),
    ("b8", "A red dot marks the line. The script will stop right before it runs that line.",
     [("go", 838, 352)], "screen"),
    ("b9", "Now open the debugger panel, with the bug button on the right edge of the editor.",
     [("click", 1580, 171)], "screen"),
    ("b10", "The top row holds the controls: continue, pause, step over, step into, step out, step bar, restart, and stop.",
     [("go", CONT, Y), ("wait", 0.3), ("go", PAUSE, Y), ("wait", 0.3), ("go", OVER, Y), ("wait", 0.3),
      ("go", INTO, Y), ("wait", 0.3), ("go", OUT, Y), ("wait", 0.3), ("go", BAR, Y), ("wait", 0.3),
      ("go", RESTART, Y), ("wait", 0.3), ("go", STOP, Y)], "screen"),

    ("b11", "Press continue. The script runs on the chart's bars, and stops at the breakpoint.",
     [("click", CONT, Y)], "screen"),
    ("b12", "The editor highlights the paused line, and shows the values of the names it uses, right next to the code.",
     [("go", 1250, 345)], "screen"),
    ("b13", "The bar context tells you which bar you're on, whether it's historical or live, and its open, high, low, close, and volume.",
     [("go", 830, 554), ("go", 1450, 554)], "screen"),
    ("b14", "Variables are grouped into globals, persistent variables declared with var, and built-in values.",
     [("go", 900, 635), ("wait", 0.4), ("go", 900, 679), ("wait", 0.4), ("go", 900, 703)], "screen"),

    ("b15", "Now let's step into the function call. Step into follows the call, and pauses on the first line inside avgBody.",
     [("click", INTO, Y)], "screen"),
    ("b16", "The call stack shows where we are: avgBody on top, and the main script that called it underneath.",
     [("go", 1300, 670), ("wait", 0.3), ("go", 1300, 694)], "screen"),
    ("b17", "Keep stepping into, until we reach the loop.",
     [("click", INTO, Y), ("wait", 0.8), ("click", INTO, Y)], "screen"),
    ("b18", "Step over runs the current line and moves to the next one, without entering any function. Each pass through the loop stops once, so you can watch total grow, and i count up.",
     [("click", OVER, Y), ("wait", 0.9), ("go", 900, 660), ("click", OVER, Y), ("wait", 0.9),
      ("go", 900, 660), ("click", OVER, Y), ("wait", 0.9), ("click", OVER, Y)], "screen"),
    ("b19", "Click a frame in the call stack to see the variables of whoever called the function. The editor marks the caller's line too.",
     [("click", 1300, 694), ("wait", 1.5), ("click", 1300, 670)], "screen"),
    ("b20", "Step out finishes the current function, and stops on the next statement in the caller.",
     [("click", OUT, Y)], "screen"),
    ("b21", "Back in the main script, body now holds the result of avgBody.",
     [("go", 900, 657)], "screen"),

    ("b22", "Watches let you follow any expression you like. Type close minus open, and press Enter.",
     [("click", 1355, 613), ("type", "close - open"), ("key", "return")], "screen"),
    ("b23", "Step bar jumps to the first statement of the next bar, so you can see how values change from one bar to the next. The watch now shows the next candle.",
     [("click", BAR, Y)], "screen"),
    ("b24", "That's also handy for persistent variables like bullish, which carry their value from bar to bar.",
     [("click", BAR, Y), ("wait", 0.9), ("click", BAR, Y)], "screen"),

    ("b25", "A script runs over hundreds of bars, so you rarely want to stop on every one. Edit the breakpoint, to give it a condition.",
     [("go", 1300, 742), ("click", 1524, 742)], "screen"),
    ("b26", "I'll type: bullish equals equals five, so it only stops once five bullish candles have been counted.",
     [("click", 1383, 810), ("type", "bullish == 5"), ("key", "return")], "screen"),
    ("b27", "Press continue, and the script runs straight to the first bar where that is true.",
     [("click", CONT, Y), ("wait", 1.2), ("go", 900, 679)], "screen"),
    ("b28", "You can also stop only on the nth hit, or turn a breakpoint into a logpoint, which writes a message and keeps going.",
     [("go", 1306, 881)], "screen"),
    ("b29", "If the script hits a runtime error, the debugger stops on the failing line with everything intact, so you can see exactly why.",
     [("go", 1100, 300)], "screen"),
    ("b30", "When you're done, press stop. The debugger runs on its own copy of the chart's bars, so your charts and alerts are never affected.",
     [("click", STOP, Y)], "screen"),

    ("o1", "Every button has a keyboard shortcut, listed here. Press F9 to toggle a breakpoint, and use the others to continue and step.", [], "end"),
    ("o2", "That's debugging in Pine Studio. Thanks for watching!", [], "end"),
]
