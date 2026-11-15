"""Seed a throwaway DegenView data directory: one BTCUSDT 1D chart, an MCP client, two Pine scripts.

usage: seed.py <HOME>      (the app must have run once so its tables exist, and must be closed)
The MCP token comes from DEGENVIEW_RECORD_TOKEN, the port from DEGENVIEW_RECORD_PORT.
"""

import hashlib
import json
import os
import shutil
import sqlite3
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
data = os.path.join(sys.argv[1], "Library/Application Support/es.nico.degenview")
token = os.environ["DEGENVIEW_RECORD_TOKEN"]
port = int(os.environ.get("DEGENVIEW_RECORD_PORT", "8791"))

PERMISSIONS = [
    "charts.read", "charts.create", "charts.indicators.write", "charts.pine_instances.write",
    "charts.candles.read", "charts.view.write", "charts.snapshot.read",
    "drawings.read", "drawings.write", "markets.read", "pine.scripts.list", "pine.scripts.read",
]
chart = {
    "id": 1, "symbol": "BTCUSDT", "source": "binance", "y_zoom": 1.0,
    "indicators": {"show_volume": False, "show_rsi": False, "show_ema": False, "ema_period": 20,
                   "show_bollinger": False, "show_trend_flips": False},
}
columns = [{"id": 2, "chart_ids": [1]}]
tab = {"id": 1, "timeframe": "Day", "watchlist_visible": False, "focused": 1, "active_view": 1,
       "columns": columns, "charts": [chart]}
view = {"id": 1, "name": "BTC analysis", "timeframe": "Day", "created_at_ms": 1700000000000,
        "autosave": True, "columns": columns, "charts": [chart]}
client = {"id": "recorder", "name": "recorder", "verifier": hashlib.sha256(token.encode()).hexdigest(),
          "permissions": PERMISSIONS, "createdAtMs": 1700000000000}

db = sqlite3.connect(os.path.join(data, "degenview.sqlite"))
for table, row in (("tab", tab), ("saved_view", view)):
    db.execute(f"insert or replace into {table}(id, position, payload) values(?, 0, ?)", ("1", json.dumps(row)))
db.execute("insert or replace into mcp_client(id, position, payload) values(?, 0, ?)", ("recorder", json.dumps(client)))
for key, value in (
    ("mcp.settings", {"enabled": True, "port": port, "require_confirmation": False}),
    ("mcp.permissions", PERMISSIONS),
):
    db.execute("insert or replace into setting(key, value) values(?, ?)", (key, json.dumps(value)))
db.commit()
db.close()

# The MCP cannot add the bundled indicators, but it can apply any script in the library folder.
scripts = os.path.join(data, "Scripts")
os.makedirs(scripts, exist_ok=True)
for name in ("ichimoku-cloud", "macd"):
    shutil.copy(os.path.join(ROOT, "src/model/pine/bundled", f"{name}.pine"), scripts)
