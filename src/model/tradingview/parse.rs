//! Reads the documents the TradingView app fetched into a [`Library`]. The documents are the web
//! app's own responses; none is a published format, so every field is optional here and a document
//! that does not have the expected shape contributes nothing rather than failing the rest.

use super::drawing::RawDrawing;
use super::grid::Grid;
use super::library::{Layout, LayoutChart, Library, Script, Study, TvWatchlist, WatchRow};
use super::{TvSymbol, interval};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// One response of TradingView's, by what it holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Document {
    /// A chart page: the layout it opened, id and settings, sits in the page's start-up script.
    /// (Its address does not always name the layout.)
    LayoutPage { html: String },
    /// Drawings of a layout, as a delta: a drawing named with `null` was deleted.
    Sources { layout: String, json: String },
    /// One watchlist, or several; `complete` when it is every list the account has.
    Watchlists { json: String, complete: bool },
    /// One version of a script of the account's own.
    Script { id: String, json: String },
    /// The list of scripts the account has saved.
    SavedScripts { json: String },
}

/// A document and when TradingView received it (epoch milliseconds).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fetched {
    pub at_ms: i64,
    pub document: Document,
}

/// A script found, with whether the account's saved list names it.
#[derive(Default)]
struct ScriptBuild {
    newest: Option<((u32, u32), Script)>,
}

/// What a delta says of one drawing.
enum Change {
    Drawn(Box<RawDrawing>),
    Unreadable,
    Deleted,
}

/// A rebuilt layout shows at most this many markets; the rest keep their drawings under a chart.
const MAX_REBUILT_CHARTS: usize = 4;

#[derive(Default)]
struct LayoutBuild {
    page: Option<Page>,
    /// Drawing id to its newest state.
    drawings: BTreeMap<String, Change>,
    seen_at_ms: i64,
}

struct Page {
    name: String,
    grid: Grid,
    charts: Vec<LayoutChart>,
}

/// Builds the library from every document, applying them oldest first so that a newer response
/// replaces what an older one said.
pub fn library(mut documents: Vec<Fetched>) -> Library {
    documents.sort_by_key(|fetched| fetched.at_ms);
    let mut layouts: BTreeMap<String, LayoutBuild> = BTreeMap::new();
    let mut watchlists: BTreeMap<String, (i64, TvWatchlist)> = BTreeMap::new();
    let mut scripts: BTreeMap<String, ScriptBuild> = BTreeMap::new();
    let mut saved: BTreeMap<String, String> = BTreeMap::new();

    for Fetched { at_ms, document } in documents {
        match document {
            Document::LayoutPage { html } => {
                if let Some((layout, page)) = embedded_id(&html).zip(page(&html)) {
                    let build = layouts.entry(layout).or_default();
                    build.page = Some(page);
                    build.seen_at_ms = at_ms;
                }
            }
            Document::Sources { layout, json } => {
                let changes = sources(&json);
                if changes.is_empty() {
                    continue;
                }
                let build = layouts.entry(layout).or_default();
                build.seen_at_ms = at_ms;
                build.drawings.extend(changes);
            }
            Document::Watchlists { json, complete } => {
                // A list missing from a newer list of all of them was deleted.
                if complete && let Some(ids) = listed(&json, number_id) {
                    watchlists.retain(|id, _| ids.contains(id));
                }
                for list in watchlists_of(&json) {
                    watchlists.insert(list.id.clone(), (at_ms, list));
                }
            }
            Document::Script { id, json } => {
                if let Some((version, script)) = script(&id, &json) {
                    let build = scripts.entry(id).or_default();
                    if build.newest.as_ref().is_none_or(|(old, _)| *old <= version) {
                        build.newest = Some((version, script));
                    }
                }
            }
            Document::SavedScripts { json } => {
                // Likewise a script a newer saved list no longer names.
                if let Some(ids) = listed(&json, |script| {
                    script.get("scriptIdPart")?.as_str().map(str::to_owned)
                }) {
                    scripts.retain(|id, _| ids.contains(id));
                }
                saved.extend(saved_names(&json));
            }
        }
    }

    let mut layouts: Vec<Layout> = layouts
        .into_iter()
        .filter_map(|(id, build)| layout(id, build))
        .collect();
    layouts.sort_by(|a, b| b.seen_at_ms.cmp(&a.seen_at_ms).then(a.id.cmp(&b.id)));

    let mut watchlists: Vec<(i64, TvWatchlist)> = watchlists
        .into_values()
        .filter(|(_, list)| list.symbols().next().is_some())
        .collect();
    watchlists.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));

    let mut scripts: Vec<Script> = scripts
        .into_iter()
        .filter_map(|(id, build)| {
            let (_, mut script) = build.newest?;
            if let Some(name) = saved.get(&id) {
                script.name.clone_from(name);
            }
            Some(script)
        })
        .collect();
    scripts.sort_by_key(|script| script.name.to_lowercase());

    Library {
        layouts,
        watchlists: watchlists.into_iter().map(|(_, list)| list).collect(),
        scripts,
    }
}

fn layout(id: String, build: LayoutBuild) -> Option<Layout> {
    let mut drawings = Vec::new();
    let mut unreadable = 0;
    for change in build.drawings.into_values() {
        match change {
            Change::Drawn(drawing) => drawings.push(*drawing),
            Change::Unreadable => unreadable += 1,
            Change::Deleted => {}
        }
    }
    let (name, grid, charts) = match build.page {
        Some(page) => (Some(page.name), page.grid, page.charts),
        None => {
            let charts = charts_from_drawings(&drawings);
            (None, Grid::square(charts.len()), charts)
        }
    };
    (!charts.is_empty()).then_some(Layout {
        id,
        name,
        grid,
        charts,
        drawings,
        unreadable,
        seen_at_ms: build.seen_at_ms,
    })
}

/// A chart for each of the markets drawn on most, at the interval most of its drawings were made
/// on.
fn charts_from_drawings(drawings: &[RawDrawing]) -> Vec<LayoutChart> {
    let mut markets: Vec<(TvSymbol, usize)> = Vec::new();
    for drawing in drawings {
        match markets
            .iter_mut()
            .find(|(symbol, _)| *symbol == drawing.symbol)
        {
            Some((_, count)) => *count += 1,
            None => markets.push((drawing.symbol.clone(), 1)),
        }
    }
    markets.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    markets
        .into_iter()
        .take(MAX_REBUILT_CHARTS)
        .map(|(symbol, _)| {
            let mut intervals: BTreeMap<&str, usize> = BTreeMap::new();
            for drawing in drawings.iter().filter(|drawing| drawing.symbol == symbol) {
                if interval::timeframe(&drawing.interval).is_some() {
                    *intervals.entry(&drawing.interval).or_default() += 1;
                }
            }
            let interval = intervals
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map_or_else(|| "D".to_owned(), |(text, _)| text.to_owned());
            LayoutChart {
                symbol,
                interval,
                style: None,
                overlays: Vec::new(),
                studies: Vec::new(),
                tick: None,
                shown: true,
            }
        })
        .collect()
}

/// The layout id assigned to `initData.uid`; empty on a chart that was never saved.
fn embedded_id(html: &str) -> Option<String> {
    let after = &html[html.find("initData.uid")? + "initData.uid".len()..];
    let value = after.trim_start().strip_prefix('=')?.trim_start();
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let id = value[1..].split(quote).next()?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_owned())
}

/// The JSON assigned to `initData.content` in a chart page.
fn embedded_content(html: &str) -> Option<Value> {
    let after = &html[html.find("initData.content")? + "initData.content".len()..];
    let json = after.trim_start().strip_prefix('=')?.trim_start();
    let value: Value = serde_json::Deserializer::from_str(json)
        .into_iter::<Value>()
        .next()?
        .ok()?;
    // Some builds assign the JSON as a string.
    match value {
        Value::String(text) => serde_json::from_str(&text).ok(),
        other => Some(other),
    }
}

fn page(html: &str) -> Option<Page> {
    let content = embedded_content(html)?;
    let name = content.get("name")?.as_str()?.trim().to_owned();
    let mut charts: Vec<LayoutChart> = content
        .get("charts")?
        .as_array()?
        .iter()
        // Every stored chart keeps its place: the grid counts them by it.
        .map(chart)
        .collect();
    if charts.is_empty() {
        return None;
    }
    let code = content.get("layout").and_then(Value::as_str).unwrap_or("s");
    let grid = Grid::of(code, charts.len());
    // A chart is shown when the grid has a place for it.
    for (index, chart) in charts.iter_mut().enumerate() {
        chart.shown = grid.columns.iter().any(|column| column.contains(&index));
    }
    Some(Page { name, grid, charts })
}

fn chart(chart: &Value) -> LayoutChart {
    let sources = chart
        .get("panes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pane| pane.get("sources")?.as_array())
        .flatten();
    let mut main = None;
    let mut overlays = Vec::new();
    let mut studies = Vec::new();
    for source in sources {
        let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
        let state = source.get("state");
        if kind == "MainSeries" {
            main = Some(source);
            continue;
        }
        if !(kind == "Study" || kind.starts_with("study")) {
            continue;
        }
        let inputs = state.and_then(|state| state.get("inputs"));
        let Some((id, name)) = study_identity(source) else {
            continue;
        };
        if id.starts_with("Overlay@") || id.starts_with("Compare@") {
            let symbol = inputs
                .and_then(|inputs| inputs.get("symbol"))
                .and_then(Value::as_str)
                .and_then(TvSymbol::read);
            if let Some(symbol) = symbol {
                super::library::push_unique(&mut overlays, &symbol);
            }
        } else {
            let length = inputs
                .and_then(|inputs| inputs.get("length"))
                .and_then(Value::as_u64)
                .and_then(|length| u32::try_from(length).ok());
            studies.push(Study { id, name, length });
        }
    }
    let state = main.and_then(|main| main.get("state"));
    let text = |key: &str| {
        state
            .and_then(|state| state.get(key))
            .and_then(Value::as_str)
    };
    let number = |key: &str| {
        main.and_then(|main| main.get("formattingDeps"))
            .and_then(|deps| deps.get(key))
            .and_then(Value::as_f64)
            .filter(|value| *value > 0.0)
    };
    LayoutChart {
        symbol: text("symbol")
            .and_then(TvSymbol::read)
            .unwrap_or_else(TvSymbol::unknown),
        interval: text("interval").unwrap_or("D").to_owned(),
        style: state
            .and_then(|state| state.get("style"))
            .and_then(Value::as_i64),
        overlays,
        studies,
        tick: number("pricescale").map(|scale| number("minmov").unwrap_or(1.0) / scale),
        shown: true,
    }
}

/// A study's id without its version, and a name to show.
fn study_identity(source: &Value) -> Option<(String, String)> {
    let meta = source.get("metaInfo");
    let full = match meta {
        Some(Value::String(text)) => text.as_str(),
        Some(object) => object
            .get("fullId")
            .or_else(|| object.get("id"))
            .and_then(Value::as_str)?,
        None => return None,
    };
    // `RSI@tv-basicstudies-274` is version 274 of `RSI@tv-basicstudies`.
    let id = match full.rsplit_once('-') {
        Some((head, version)) if version.chars().all(|c| c.is_ascii_digit()) => head,
        _ => full,
    }
    .to_owned();
    let name = meta
        .and_then(|meta| meta.get("description"))
        .and_then(Value::as_str)
        .map_or_else(
            || id.split('@').next().unwrap_or(&id).to_owned(),
            str::to_owned,
        );
    Some((id, name))
}

/// The drawing changes of a sources response.
fn sources(json: &str) -> Vec<(String, Change)> {
    let Ok(body) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(sources) = body
        .get("payload")
        .and_then(|payload| payload.get("sources"))
    else {
        return Vec::new();
    };
    let Some(sources) = sources.as_object() else {
        return Vec::new();
    };
    sources
        .iter()
        .filter_map(|(id, source)| {
            let change = match source {
                Value::Null => Change::Deleted,
                // A newer state replaces the older one even when it cannot be read: the older
                // one is no longer what the drawing looks like.
                source => match RawDrawing::read(id, source) {
                    Some(drawing) => Change::Drawn(Box::new(drawing)),
                    None if is_drawing(source) => Change::Unreadable,
                    None => return None,
                },
            };
            Some((id.clone(), change))
        })
        .collect()
}

/// Whether a source is a drawing, readable or not (a study kept among them is not).
fn is_drawing(source: &Value) -> bool {
    source
        .get("state")
        .and_then(|state| state.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.starts_with("LineTool"))
}

fn watchlists_of(json: &str) -> Vec<TvWatchlist> {
    let Ok(body) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let lists = match &body {
        Value::Array(lists) => lists.iter().collect(),
        list => vec![list],
    };
    lists.into_iter().filter_map(watchlist).collect()
}

/// The ids a whole listing names. `None` unless it is a list that names something: an error, or
/// the empty answer of a signed-out account, says nothing about what was deleted.
fn listed(json: &str, id: impl Fn(&Value) -> Option<String>) -> Option<BTreeSet<String>> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(json) else {
        return None;
    };
    let ids: BTreeSet<String> = entries.iter().filter_map(id).collect();
    (!ids.is_empty()).then_some(ids)
}

fn number_id(list: &Value) -> Option<String> {
    list.get("id")?.as_i64().map(|id| id.to_string())
}

/// A list the account made. The colour-flag lists are not lists of their own.
fn watchlist(list: &Value) -> Option<TvWatchlist> {
    if list.get("type").and_then(Value::as_str) != Some("custom") {
        return None;
    }
    let id = number_id(list)?;
    let name = list.get("name")?.as_str()?.trim().to_owned();
    let mut seen = BTreeSet::new();
    let rows = list
        .get("symbols")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|text| match text.strip_prefix("###") {
            // A section separator carries an invisible character after its title.
            Some(title) => Some(WatchRow::Section(
                title
                    .trim_matches(|c: char| c.is_whitespace() || c == '\u{2064}')
                    .to_owned(),
            )),
            None => TvSymbol::read(text)
                .filter(|symbol| seen.insert(symbol.clone()))
                .map(WatchRow::Market),
        })
        .collect();
    Some(TvWatchlist { id, name, rows })
}

fn version_of(text: &str) -> (u32, u32) {
    let mut parts = text.split('.').map(|part| part.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn script(id: &str, json: &str) -> Option<((u32, u32), Script)> {
    let body: Value = serde_json::from_str(json).ok()?;
    let source = body.get("source")?.as_str()?;
    // Protected and invite-only scripts come without readable source.
    if !source.contains("//@version")
        && !source.contains("indicator(")
        && !source.contains("strategy(")
        && !source.contains("study(")
        && !source.contains("library(")
    {
        return None;
    }
    let text = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let version = text("version").unwrap_or("0").to_owned();
    let name = text("scriptName")
        .or_else(|| text("scriptTitle"))
        .unwrap_or("TradingView script")
        .to_owned();
    Some((
        version_of(&version),
        Script {
            id: id.to_owned(),
            name,
            source: source.replace("\r\n", "\n"),
            version,
        },
    ))
}

fn saved_names(json: &str) -> Vec<(String, String)> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let id = entry.get("scriptIdPart")?.as_str()?;
            let name = entry.get("scriptName")?.as_str()?.trim();
            (id.starts_with("USER;") && !name.is_empty()).then(|| (id.to_owned(), name.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tradingview::fixtures;

    #[test]
    fn a_layout_page_gives_its_charts_overlays_studies_and_grid() {
        let found = library(fixtures::documents());
        let layout = &found.layouts[0];
        assert_eq!(layout.name.as_deref(), Some("Majors"));
        assert_eq!(layout.grid.columns, [[0], [1]], "side by side");
        assert_eq!(layout.charts.len(), 3, "the hidden third chart is kept");
        assert_eq!(layout.shown_charts().count(), 2);
        let first = &layout.charts[0];
        assert_eq!(first.symbol.as_str(), "BINANCE:BTCUSDT");
        assert_eq!(first.interval, "5");
        assert_eq!(first.tick, Some(0.01));
        assert_eq!(first.overlays[0].as_str(), "OKX:ETHUSDT");
        assert_eq!(first.studies[0].id, "RSI@tv-basicstudies");
        assert_eq!(first.studies[0].length, Some(14));
    }

    #[test]
    fn a_newer_response_replaces_and_deletes_drawings() {
        let found = library(fixtures::documents());
        let ids: Vec<&str> = found.layouts[0]
            .drawings
            .iter()
            .map(|drawing| drawing.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["line", "rect"],
            "`gone` was deleted by the later delta"
        );
        assert_eq!(found.layouts[0].drawings[0].points[1].price, 120.0);
    }

    #[test]
    fn a_layout_with_only_drawings_is_rebuilt_from_them() {
        let found = library(vec![fixtures::sources(
            "Xy",
            5,
            &[fixtures::trend_line("a")],
        )]);
        let layout = &found.layouts[0];
        assert!(layout.is_rebuilt());
        assert_eq!(layout.charts[0].symbol.as_str(), "BINANCE:BTCUSDT");
        assert_eq!(layout.charts[0].interval, "60");
    }

    #[test]
    fn watchlists_keep_sections_and_skip_flag_lists() {
        let found = library(fixtures::documents());
        assert_eq!(found.watchlists.len(), 1);
        let rows = &found.watchlists[0].rows;
        assert_eq!(rows[0], WatchRow::Section("Crypto".into()));
        assert_eq!(rows.len(), 3, "the repeated market is listed once");
    }

    #[test]
    fn the_newest_version_of_a_script_wins_and_takes_its_saved_name() {
        let found = library(fixtures::documents());
        assert_eq!(found.scripts.len(), 1);
        assert_eq!(found.scripts[0].name, "My bands");
        assert_eq!(found.scripts[0].version, "12.0");
    }

    #[test]
    fn broken_documents_contribute_nothing() {
        let broken = |document| Fetched { at_ms: 1, document };
        let found = library(vec![
            broken(Document::LayoutPage {
                html: r#"initData.uid = "a"; initData.content = {nope"#.into(),
            }),
            broken(Document::Sources {
                layout: "a".into(),
                json: "[]".into(),
            }),
            broken(Document::Watchlists {
                complete: true,
                json: "null".into(),
            }),
            broken(Document::Script {
                id: "USER;1".into(),
                json: r#"{"source":"hidden"}"#.into(),
            }),
        ]);
        assert!(found.is_empty());
    }

    #[test]
    fn a_chart_that_cannot_be_a_market_keeps_its_place_in_the_grid() {
        let html = r#"initData.uid = "Cd"; initData.content = {"name": "S", "layout": "2h",
            "charts": [
              {"panes": [{"sources": [{"type": "MainSeries",
                  "state": {"symbol": "BINANCE:BTCUSDT/BINANCE:ETHUSDT"}}]}]},
              {"panes": [{"sources": [{"type": "MainSeries", "state": {"symbol": "NASDAQ:AAPL"}}]}]},
              {"panes": [{"sources": [{"type": "MainSeries", "state": {"symbol": "NASDAQ:MSFT"}}]}]}
            ]};"#;
        let found = library(vec![Fetched {
            at_ms: 1,
            document: Document::LayoutPage { html: html.into() },
        }]);
        let shown: Vec<bool> = found.layouts[0].charts.iter().map(|c| c.shown).collect();
        assert_eq!(shown, [true, true, false]);
        assert!(found.layouts[0].charts[0].symbol.is_expression());
    }

    #[test]
    fn a_newer_state_that_cannot_be_read_replaces_the_older_one_and_is_counted() {
        let mut broken = fixtures::trend_line("a");
        broken["state"]["points"] = serde_json::json!([{"price": 1.0}]);
        let found = library(vec![
            fixtures::sources(
                "Ab",
                1,
                &[fixtures::trend_line("a"), fixtures::trend_line("b")],
            ),
            fixtures::sources("Ab", 2, &[broken]),
        ]);
        let layout = &found.layouts[0];
        assert_eq!((layout.drawings.len(), layout.unreadable), (1, 1));
    }

    #[test]
    fn a_rebuilt_layout_shows_only_the_markets_drawn_on_most() {
        let drawings: Vec<serde_json::Value> = (0..9)
            .map(|index| {
                fixtures::drawing(
                    &format!("d{index}"),
                    "LineToolHorzLine",
                    &format!("X:M{}USD", index.min(7)),
                    serde_json::json!({}),
                    &[(0, 1.0)],
                )
            })
            .collect();
        let found = library(vec![fixtures::sources("Ab", 1, &drawings)]);
        let layout = &found.layouts[0];
        assert_eq!(layout.charts.len(), 4);
        assert_eq!(layout.charts[0].symbol.as_str(), "X:M7USD", "two drawings");
        assert_eq!(layout.drawings.len(), 9, "none is lost");
    }

    #[test]
    fn what_a_newer_listing_no_longer_names_was_deleted() {
        let lists = |at_ms, complete, json: serde_json::Value| Fetched {
            at_ms,
            document: Document::Watchlists {
                json: json.to_string(),
                complete,
            },
        };
        let saved = |at_ms, json: serde_json::Value| Fetched {
            at_ms,
            document: Document::SavedScripts {
                json: json.to_string(),
            },
        };
        let list = |id: i64| {
            serde_json::json!({"id": id, "type": "custom", "name": format!("L{id}"),
                               "symbols": ["BINANCE:BTCUSDT"]})
        };
        let names = |extra: Vec<Fetched>| {
            let mut documents = fixtures::documents();
            documents.extend(extra);
            let found = library(documents);
            (
                found
                    .watchlists
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<Vec<_>>(),
                found.scripts.len(),
            )
        };

        let gone = names(vec![
            lists(100, true, serde_json::json!([list(9)])),
            saved(
                100,
                serde_json::json!([{"scriptIdPart": "USER;other", "scriptName": "o"}]),
            ),
        ]);
        assert_eq!(gone, (vec!["L9".to_owned()], 0));

        // One list on its own, an error and a signed-out account's nothing delete nothing.
        let kept = names(vec![
            lists(100, false, serde_json::json!(list(9))),
            lists(101, true, serde_json::json!({"detail": "sign in"})),
            lists(102, true, serde_json::json!([])),
            saved(103, serde_json::json!([])),
        ]);
        assert_eq!(kept, (vec!["L9".to_owned(), "Coins".to_owned()], 1));
    }
}
