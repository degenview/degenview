//! Documents shaped like TradingView's responses, with made-up content.

use super::parse::{Document, Fetched};
use serde_json::{Value, json};

/// Three drawing points apart by an hour from this bar.
pub const T0: i64 = 1_700_000_000;

pub fn drawing(id: &str, tool: &str, symbol: &str, props: Value, points: &[(i64, f64)]) -> Value {
    let mut state = json!({"interval": "60", "symbol": symbol, "visible": true, "frozen": false});
    if let (Some(state), Some(props)) = (state.as_object_mut(), props.as_object()) {
        state.extend(props.clone());
    }
    let points: Vec<Value> = points
        .iter()
        .map(|(hour, price)| {
            json!({"time_t": T0 + hour * 3_600, "offset": 0, "price": price, "interval": "60"})
        })
        .collect();
    json!({
        "id": id,
        "symbol": symbol,
        "state": {"type": tool, "id": id, "state": state, "points": points, "zorder": -1000},
    })
}

pub fn trend_line(id: &str) -> Value {
    drawing(
        id,
        "LineToolTrendLine",
        "BINANCE:BTCUSDT",
        json!({"linecolor": "rgba(255, 235, 59, 1)", "linewidth": 3, "linestyle": 2}),
        &[(0, 100.0), (1, 120.0)],
    )
}

pub fn sources(layout: &str, at_ms: i64, drawings: &[Value]) -> Fetched {
    let map: serde_json::Map<String, Value> = drawings
        .iter()
        .map(|d| (d["id"].as_str().unwrap_or_default().to_owned(), d.clone()))
        .collect();
    Fetched {
        at_ms,
        folder: 0,
        document: Document::Sources {
            layout: layout.into(),
            json: json!({"success": true, "payload": {"sources": map}}).to_string(),
        },
    }
}

fn main_series(symbol: &str, interval: &str) -> Value {
    json!({
        "type": "MainSeries",
        "id": "_seriesId",
        "formattingDeps": {"pricescale": 100, "minmov": 1},
        "state": {"symbol": symbol, "interval": interval, "style": 1},
    })
}

/// A two-column layout with a third, hidden chart.
pub fn layout_page(layout: &str, at_ms: i64) -> Fetched {
    let content = json!({
        "name": "Majors",
        "layout": "2h",
        "charts": [
            {"panes": [{"sources": [
                main_series("BINANCE:BTCUSDT", "5"),
                {"type": "study_Overlay", "metaInfo": "Overlay@tv-basicstudies-274",
                 "state": {"inputs": {"symbol": "OKX:ETHUSDT"}}},
                {"type": "Study", "metaInfo": "RSI@tv-basicstudies-1",
                 "state": {"inputs": {"length": 14}}},
            ]}]},
            {"panes": [{"sources": [main_series("BITSTAMP:ETHUSD", "60")]}]},
            {"panes": [{"sources": [main_series("NASDAQ:AAPL", "D")]}]},
        ],
    });
    Fetched {
        at_ms,
        folder: 0,
        document: Document::LayoutPage {
            html: format!(
                "<script>window.initData.content = {content};\n\
                 window.initData.uid = \"{layout}\";\n</script>"
            ),
        },
    }
}

fn script_version(version: &str, at_ms: i64) -> Fetched {
    Fetched {
        at_ms,
        folder: 0,
        document: Document::Script {
            id: "USER;abc".into(),
            json: json!({
                "scriptName": "",
                "scriptTitle": "Bands draft",
                "version": version,
                "source": "//@version=6\nindicator(\"Bands\")\nplot(close)\n",
            })
            .to_string(),
        },
    }
}

/// One layout with drawings, a watchlist, a colour list and two versions of a script.
pub fn documents() -> Vec<Fetched> {
    let lists = json!([
        {"id": 7, "type": "custom", "name": "Coins",
         "symbols": ["###Crypto\u{2064}", "BINANCE:BTCUSDT", "BINANCE:BTCUSDT", "NOPE:XYZ"]},
        {"id": 8, "type": "colored", "name": "", "color": "red", "symbols": ["BINANCE:BTCUSDT"]},
    ]);
    let rect = drawing(
        "rect",
        "LineToolRectangle",
        "BINANCE:BTCUSDT",
        json!({"color": "#9c27b0"}),
        &[(0, 90.0), (3, 130.0)],
    );
    let gone = drawing(
        "gone",
        "LineToolHorzLine",
        "BINANCE:BTCUSDT",
        json!({}),
        &[(0, 90.0)],
    );
    let mut deleted = sources("Ab", 30, &[]);
    if let Document::Sources { json, .. } = &mut deleted.document {
        *json = r#"{"payload":{"sources":{"gone":null}}}"#.into();
    }
    vec![
        layout_page("Ab", 10),
        sources("Ab", 20, &[trend_line("line"), rect, gone]),
        deleted,
        Fetched {
            at_ms: 5,
            folder: 0,
            document: Document::Watchlists {
                complete: true,
                json: lists.to_string(),
            },
        },
        script_version("12.0", 40),
        script_version("2.0", 50),
        Fetched {
            at_ms: 60,
            folder: 0,
            document: Document::SavedScripts {
                json: json!([{"scriptIdPart": "USER;abc", "scriptName": "My bands"}]).to_string(),
            },
        },
    ]
}
