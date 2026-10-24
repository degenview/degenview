//! The example Pine scripts that ship inside the app (`demos/*.pine`), ported from
//! `DemoScriptLibrary.swift`. They are copied into the user's script library, never edited in
//! place, so they behave like any other script once they are there.

use super::store::{ScriptStore, ScriptStoreError, declared_type};

pub struct Demo {
    pub name: &'static str,
    pub source: &'static str,
}

/// Set once the first launch has offered the demos, so ones the user deletes stay deleted.
pub const SEEDED_KEY: &str = "scripts.demosSeeded";

macro_rules! demo {
    ($name:literal) => {
        Demo {
            name: $name,
            source: include_str!(concat!("demos/", $name, ".pine")),
        }
    };
}

/// Every bundled demo, named after its file, in alphabetical order.
pub const BUNDLED: [Demo; 9] = [
    demo!("Bollinger Squeeze"),
    demo!("EMA Cross Strategy"),
    demo!("EMA Cross"),
    demo!("Higher Timeframe Trend"),
    demo!("MACD Histogram"),
    demo!("Pivot Levels"),
    demo!("RSI Zones"),
    demo!("Stats Table"),
    demo!("Volume Spike Alert"),
];

/// Adds each demo the library doesn't already hold under that name and returns how many it
/// added. Existing scripts are never touched.
pub fn add_missing(store: &mut ScriptStore) -> Result<usize, ScriptStoreError> {
    let mut added = 0;
    // The sidebar lists newest first, so adding in reverse leaves the demos alphabetical.
    for demo in BUNDLED.iter().rev() {
        match store.create(
            demo.name,
            declared_type(demo.source),
            Some(demo.source),
            false,
        ) {
            Ok(_) => added += 1,
            Err(ScriptStoreError::NameConflict) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_manager::model::ScriptType;

    #[test]
    fn demos_add_once_and_types_follow_declarations() {
        let root = std::env::temp_dir().join(format!("degenview-demo-{}", std::process::id()));
        let mut store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        assert_eq!(add_missing(&mut store).unwrap(), 9);
        assert_eq!(add_missing(&mut store).unwrap(), 0);
        let all = store.all_scripts().unwrap();
        let strategy = all.iter().find(|s| s.name == "EMA Cross Strategy").unwrap();
        assert_eq!(strategy.script_type, ScriptType::Strategy);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use crate::model::Kline;
    use crate::pine::program::SymbolInfo;

    fn bars(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|i| {
                let base = 100.0 + (i as f64 * 0.3).sin() * 10.0 + i as f64 * 0.05;
                Kline {
                    open_time: 1_700_000_000_000 + i as i64 * 86_400_000,
                    open: base,
                    high: base + 2.0,
                    low: base - 2.0,
                    close: base + 0.5,
                    volume: 1000.0,
                    close_time: 1_700_000_000_000 + (i as i64 + 1) * 86_400_000 - 1,
                }
            })
            .collect()
    }

    #[test]
    fn every_bundled_demo_runs_to_the_end() {
        let symbol = SymbolInfo {
            ticker: "BTCUSDT".into(),
            ticker_id: "BINANCE:BTCUSDT".into(),
            currency: "USD".into(),
            kind: "crypto".into(),
            base_currency: None,
            ..SymbolInfo::default()
        };
        let mut problems = Vec::new();
        for demo in &BUNDLED {
            if demo.name == "Higher Timeframe Trend" {
                continue; // needs request.security (network)
            }
            let (output, failure) = crate::host::pine_host::run_script_checked(
                demo.source,
                &bars(200),
                &symbol,
                &std::collections::HashMap::new(),
            );
            let drawn = output.plots.len()
                + output.lines.len()
                + output.labels.len()
                + output.hlines.len()
                + output.markers.len()
                + output.tables.len()
                + output.backgrounds.len()
                + output.strategy.iter().count();
            if drawn == 0 || failure.is_some() {
                problems.push(format!("{}: {:?}", demo.name, failure.map(|d| d.message)));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
