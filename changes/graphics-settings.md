new: Settings has a Graphics page to tune display sync, frame queue and the GPU backend when the mouse feels sticky inside the window.
fixed-rc: Mouse input no longer lags inside chart windows after the first few seconds on Windows (present mode, frame latency, and skipping presents on the hidden root — needs the matching degenview-eframe change).
fixed-rc: Settings feels less sticky while moving the mouse — paint cache and per-page host loads, without a second layout pass on card hover.
