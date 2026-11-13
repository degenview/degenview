//! The Predictions tab: Polymarket and Kalshi events with their choices. Choices of one event can
//! be ticked and charted together as the lines of one chart.

mod events;
pub mod rows;
mod trending;

pub use events::{
    Events, Tick, event_title, group_tick, is_ticked, ticked_count, toggle_choice, toggle_group,
};
pub use trending::{PROVIDER as TRENDING_PROVIDER, Trending};

use crate::model::DataSource;
use crate::service::SearchResult;

/// An event with several choices, as the header row above them.
#[derive(Clone, Debug, PartialEq)]
pub struct EventHead {
    pub title: String,
    pub image_url: Option<String>,
    pub choices: usize,
    /// The likeliest choice, which stands for the event in the detail panel.
    pub lead: SearchResult,
    pub open: bool,
    pub tick: Tick,
}

impl EventHead {
    pub fn source(&self) -> DataSource {
        self.lead.source
    }

    /// What the header says under the title: how many choices, and the likeliest.
    pub fn summary(&self) -> String {
        let count = format!(
            "{} choice{}",
            self.choices,
            if self.choices == 1 { "" } else { "s" }
        );
        match self.lead.price {
            Some(chance) => format!(
                "{count} · {} {}",
                self.lead.symbol,
                crate::format::headline(chance, true)
            ),
            None => count,
        }
    }
}

/// The list items of `events`: a lone choice stands as a market of its own; an event with
/// several is a header followed, while it is open, by its choices.
pub(super) fn items(events: &Events, ticked: Option<&SearchResult>) -> Vec<super::Item> {
    let mut items = Vec::new();
    for group in &events.groups {
        if group.results.len() == 1 {
            items.push(super::Item::Market(group.lead().clone()));
            continue;
        }
        let open = events.is_open(&group.title);
        items.push(super::Item::Event(EventHead {
            title: group.title.clone(),
            image_url: group.image_url().map(str::to_owned),
            choices: group.results.len(),
            lead: group.lead().clone(),
            open,
            tick: group_tick(ticked, group),
        }));
        if open {
            items.extend(group.results.iter().cloned().map(super::Item::Choice));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::EventInfo;

    fn choice(event: &str, label: &str, price: f64) -> SearchResult {
        SearchResult {
            price: Some(price),
            event: Some(EventInfo {
                title: event.into(),
                ..EventInfo::default()
            }),
            ..SearchResult::new(DataSource::Polymarket, label, format!("{event}/{label}"))
        }
    }

    fn held(results: &[SearchResult]) -> Events {
        let mut events = Events::default();
        events.set(results.iter());
        events
    }

    #[test]
    fn a_lone_choice_is_a_market_and_an_event_is_a_header() {
        let events = held(&[
            choice("Fed", "Cut", 0.2),
            choice("Fed", "Hold", 0.7),
            choice("Rain", "Yes", 0.4),
        ]);
        let items = items(&events, None);
        assert!(matches!(&items[0], super::super::Item::Event(head) if head.title == "Fed"));
        assert!(matches!(&items[1], super::super::Item::Market(result) if result.symbol == "Yes"));
        assert_eq!(items.len(), 2, "a folded event shows no choices");
    }

    #[test]
    fn an_open_event_lists_its_choices_likeliest_first() {
        let mut events = held(&[choice("Fed", "Cut", 0.2), choice("Fed", "Hold", 0.7)]);
        // One event alone starts open.
        let labels: Vec<String> = items(&events, None)
            .into_iter()
            .filter_map(|item| match item {
                super::super::Item::Choice(result) => Some(result.symbol),
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["Hold", "Cut"]);
        events.fold("Fed");
        assert_eq!(items(&events, None).len(), 1);
    }

    #[test]
    fn the_header_names_the_likeliest_choice() {
        let events = held(&[choice("Fed", "Cut", 0.2), choice("Fed", "Hold", 0.7)]);
        let super::super::Item::Event(head) = &items(&events, None)[0] else {
            panic!("a header");
        };
        assert_eq!(head.summary(), "2 choices · Hold 70.0%");
    }
}
