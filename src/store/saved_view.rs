//! Persistence for named saved layouts, under the `saved_view` document table.

use crate::model::SavedView;
use crate::store::{Document, Store};

impl Document for SavedView {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    pub fn saved_views(&self) -> Vec<SavedView> {
        self.documents("saved_view")
    }

    pub fn replace_saved_views(&self, views: &[SavedView]) -> rusqlite::Result<()> {
        self.replace_documents("saved_view", views)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Timeframe;

    fn view(id: u64) -> SavedView {
        SavedView {
            id,
            name: "BTC watch".into(),
            timeframe: Timeframe::Hour,
            created_at_ms: 1_000,
            charts: Vec::new(),
            columns: None,
            candle_count: 0,
            last_opened_at_ms: None,
            autosave: None,
            sync: Default::default(),
        }
    }

    /// A layout whose one chart holds `drawings` (as stored JSON).
    fn view_with(drawings: Vec<serde_json::Value>) -> SavedView {
        let mut value = serde_json::to_value(view(3)).unwrap();
        value["charts"] = serde_json::json!([{
            "id": 1,
            "symbol": "BTC",
            "source": crate::model::DataSource::Binance,
            "log_scale": false,
            "y_zoom": 1.0,
            "indicators": crate::model::IndicatorConfig::default(),
            "drawings": drawings,
        }]);
        serde_json::from_value(value).expect("a layout")
    }

    #[test]
    fn every_line_tool_and_a_future_one_survive_the_database_exactly() {
        use crate::model::{Anchor, Drawing, Placement, Tool};
        let anchors: Vec<Anchor> = (0..4)
            .map(|index| Anchor {
                time_ms: 1_700_000_000_000 + index * 3_600_000,
                price: 100.0 + index as f64 * 7.5,
            })
            .collect();
        let mut stored: Vec<serde_json::Value> = Tool::line_tools()
            .enumerate()
            .map(|(index, tool)| {
                let count = match tool.spec().placement {
                    Placement::Points(count) => usize::from(count),
                    _ => 1,
                };
                let mut style = tool.default_style();
                style.pitchfork_levels[4].line.enabled = true;
                style.regression.lower_dev = -3.5;
                let mut drawing =
                    Drawing::build(tool, index as u64 + 1, &anchors[..count], style).unwrap();
                drawing.set_text("kept");
                serde_json::to_value(&drawing).unwrap()
            })
            .collect();
        // A tool from a build newer than this one rides along untouched.
        stored.push(serde_json::json!({"Shape": {"id": 99, "geometry": {"hologram": {"x": 1}}}}));
        let store = Store::open_in_memory().expect("open");
        store
            .replace_saved_views(&[view_with(stored.clone())])
            .expect("write");
        let loaded = store.saved_views();
        let drawings = &loaded[0].charts[0].drawings;
        assert_eq!(drawings.len(), 18);
        assert!(matches!(drawings[17], Drawing::Unreadable(_)));
        // Written back, the row is what it was: the readable ones re-encode identically and the
        // unreadable one verbatim.
        store.replace_saved_views(&loaded).expect("rewrite");
        let again = store.saved_views();
        let back: Vec<serde_json::Value> = again[0].charts[0]
            .drawings
            .iter()
            .map(|drawing| match drawing {
                Drawing::Unreadable(raw) => raw.clone(),
                other => serde_json::to_value(other).unwrap(),
            })
            .collect();
        assert_eq!(back, stored);
    }

    #[test]
    fn every_pattern_wave_and_cycle_survives_the_database_exactly() {
        use crate::model::elliott::Degree;
        use crate::model::{Anchor, Drawing, Placement, Tool};
        let anchors: Vec<Anchor> = (0..7)
            .map(|index| Anchor {
                time_ms: 1_700_000_000_000 + index * 3_600_000,
                price: 100.0 + (index % 3) as f64 * 7.5 + index as f64,
            })
            .collect();
        let stored: Vec<serde_json::Value> = Tool::pattern_tools()
            .enumerate()
            .map(|(index, tool)| {
                let count = match tool.spec().placement {
                    Placement::Points(count) => usize::from(count),
                    _ => 1,
                };
                let mut style = tool.default_style();
                style.pattern.degree = Degree::Supercycle;
                style.pattern.precision = 4;
                let drawing =
                    Drawing::build(tool, index as u64 + 1, &anchors[..count], style).unwrap();
                serde_json::to_value(&drawing).unwrap()
            })
            .collect();
        let store = Store::open_in_memory().expect("open");
        store
            .replace_saved_views(&[view_with(stored.clone())])
            .expect("write");
        let loaded = store.saved_views();
        let drawings = &loaded[0].charts[0].drawings;
        assert_eq!(drawings.len(), 14);
        assert!(drawings.iter().all(|drawing| drawing.tool().is_some()));
        store.replace_saved_views(&loaded).expect("rewrite");
        let again = store.saved_views();
        let back: Vec<serde_json::Value> = again[0].charts[0]
            .drawings
            .iter()
            .map(|drawing| serde_json::to_value(drawing).unwrap())
            .collect();
        assert_eq!(back, stored);
    }

    #[test]
    fn saved_views_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_saved_views(&[view(7), view(9)])
            .expect("write");
        let loaded = store.saved_views();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, 7);
        assert_eq!(loaded[1].id, 9);
    }
}
