//! Prediction events: search hits grouped under their event, and the choices ticked to chart
//! together as the lines of one chart.

use crate::model::PmSeries;
use crate::service::{EventInfo, SearchResult};
use std::collections::HashSet;

/// The choices of one event, likeliest first.
#[derive(Clone, Debug, PartialEq)]
pub struct EventGroup {
    pub title: String,
    pub results: Vec<SearchResult>,
}

impl EventGroup {
    /// The choice that stands for the event: the likeliest one.
    pub fn lead(&self) -> &SearchResult {
        &self.results[0]
    }

    pub fn image_url(&self) -> Option<&str> {
        self.lead()
            .event
            .as_ref()
            .and_then(|event| event.image_url.as_deref())
    }
}

/// Prediction-market results grouped by event, and which events show their choices.
#[derive(Default)]
pub struct Events {
    pub groups: Vec<EventGroup>,
    /// Titles of the events whose choices are showing.
    expanded: HashSet<String>,
}

impl Events {
    /// Regroups `results`. A new set of markets starts folded, unless it is a single event; the
    /// same markets with fresh prices keep what the user opened.
    pub fn set<'a>(&mut self, results: impl Iterator<Item = &'a SearchResult>) {
        let groups = group_by_event(results);
        if !self.has_same_markets(&groups) {
            self.expanded = match groups.as_slice() {
                [only] => HashSet::from([only.title.clone()]),
                _ => HashSet::new(),
            };
        }
        self.groups = groups;
    }

    fn has_same_markets(&self, groups: &[EventGroup]) -> bool {
        let markets = |groups: &[EventGroup]| -> Vec<(String, String)> {
            groups
                .iter()
                .flat_map(|group| {
                    group
                        .results
                        .iter()
                        .map(|result| (group.title.clone(), result.full_symbol.clone()))
                })
                .collect()
        };
        markets(&self.groups) == markets(groups)
    }

    pub fn is_open(&self, title: &str) -> bool {
        self.expanded.contains(title)
    }

    /// Shows or hides the choices of the event.
    pub fn fold(&mut self, title: &str) {
        if !self.expanded.remove(title) {
            self.expanded.insert(title.to_owned());
        }
    }

    pub fn group(&self, title: &str) -> Option<&EventGroup> {
        self.groups.iter().find(|group| group.title == title)
    }
}

/// Buckets results by event without reordering events, then stably sorts each event's choices by
/// descending YES probability, choices without one last. A result with no event title stands
/// alone under its own label.
pub fn group_by_event<'a>(results: impl Iterator<Item = &'a SearchResult>) -> Vec<EventGroup> {
    let mut groups: Vec<EventGroup> = Vec::new();
    for result in results {
        let title = event_title(result);
        match groups.iter_mut().find(|group| group.title == title) {
            Some(group) => group.results.push(result.clone()),
            None => groups.push(EventGroup {
                title: title.to_owned(),
                results: vec![result.clone()],
            }),
        }
    }
    for group in &mut groups {
        group.results.sort_by(|a, b| match (a.price, b.price) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }
    groups
}

/// The event a choice belongs to: its event's title, else its own label.
pub fn event_title(result: &SearchResult) -> &str {
    result
        .event
        .as_ref()
        .map(|event| event.title.as_str())
        .filter(|title| !title.is_empty())
        .unwrap_or(&result.symbol)
}

// MARK: Ticking choices

/// How many of an event's choices are ticked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tick {
    None,
    Some,
    All,
}

/// Whether the pick has `result` among the choices ticked to chart together.
pub fn is_ticked(picked: Option<&SearchResult>, result: &SearchResult) -> bool {
    picked
        .and_then(|picked| picked.event.as_ref())
        .is_some_and(|event| {
            event
                .series
                .iter()
                .any(|choice| choice.token_id == result.full_symbol)
        })
}

/// How many choices the pick charts together.
pub fn ticked_count(picked: Option<&SearchResult>) -> usize {
    picked
        .and_then(|picked| picked.event.as_ref())
        .map_or(0, |event| event.series.len())
}

pub fn group_tick(picked: Option<&SearchResult>, group: &EventGroup) -> Tick {
    let ticked = group
        .results
        .iter()
        .filter(|result| is_ticked(picked, result))
        .count();
    match ticked {
        0 => Tick::None,
        count if count == group.results.len() => Tick::All,
        _ => Tick::Some,
    }
}

/// The pick for the `group` choices `keep` accepts, in the order the group lists them: the first
/// is the market the chart opens on, and all of them are its lines. `None` when none are kept.
fn pick_choices(group: &EventGroup, keep: impl Fn(&SearchResult) -> bool) -> Option<SearchResult> {
    let kept: Vec<&SearchResult> = group.results.iter().filter(|result| keep(result)).collect();
    let mut pick = (*kept.first()?).clone();
    let event = pick.event.get_or_insert_with(|| EventInfo {
        title: group.title.clone(),
        ..EventInfo::default()
    });
    event.series = kept
        .iter()
        .map(|result| PmSeries {
            token_id: result.full_symbol.clone(),
            label: result.symbol.clone(),
        })
        .collect();
    Some(pick)
}

/// The pick after ticking or unticking one choice. Ticking in another event drops the ticks of
/// the event before it: only one event is charted at a time.
pub fn toggle_choice(
    picked: Option<&SearchResult>,
    group: &EventGroup,
    token_id: &str,
) -> Option<SearchResult> {
    pick_choices(group, |result| {
        (result.full_symbol == token_id) != is_ticked(picked, result)
    })
}

/// The pick after the event's checkbox: every choice ticked, or none when all already were.
pub fn toggle_group(picked: Option<&SearchResult>, group: &EventGroup) -> Option<SearchResult> {
    match group_tick(picked, group) {
        Tick::All => None,
        Tick::None | Tick::Some => pick_choices(group, |_| true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn result(symbol: &str, source: DataSource) -> SearchResult {
        SearchResult::new(source, symbol, symbol)
    }

    fn choice(event: &str, label: &str, price: Option<f64>) -> SearchResult {
        SearchResult {
            full_symbol: format!("{event}/{label}"),
            price,
            event: Some(EventInfo {
                title: event.into(),
                image_url: None,
                series: Vec::new(),
            }),
            ..result(label, DataSource::Polymarket)
        }
    }

    fn titles(events: &Events) -> Vec<&str> {
        events
            .groups
            .iter()
            .map(|group| group.title.as_str())
            .collect()
    }

    fn labels(group: &EventGroup) -> Vec<&str> {
        group
            .results
            .iter()
            .map(|result| result.symbol.as_str())
            .collect()
    }

    #[test]
    fn choices_group_under_their_event_in_search_order() {
        let results = [
            choice("Fed", "Cut", Some(0.2)),
            choice("Election", "Yes", Some(0.5)),
            choice("Fed", "Hold", Some(0.7)),
        ];
        let mut events = Events::default();
        events.set(results.iter());
        assert_eq!(titles(&events), ["Fed", "Election"]);
        assert_eq!(labels(&events.groups[0]), ["Hold", "Cut"]);
        assert_eq!(events.groups[0].lead().symbol, "Hold");
    }

    #[test]
    fn choices_sort_by_probability_with_unpriced_last_and_ties_kept() {
        let results = [
            choice("E", "none", None),
            choice("E", "tie a", Some(0.4)),
            choice("E", "top", Some(0.9)),
            choice("E", "tie b", Some(0.4)),
        ];
        let groups = group_by_event(results.iter());
        assert_eq!(labels(&groups[0]), ["top", "tie a", "tie b", "none"]);
    }

    #[test]
    fn a_result_without_an_event_stands_under_its_own_label() {
        let results = [result("Will it rain?", DataSource::Kalshi)];
        let groups = group_by_event(results.iter());
        assert_eq!(groups[0].title, "Will it rain?");
        assert_eq!(groups[0].results.len(), 1);
    }

    #[test]
    fn events_start_folded_unless_there_is_only_one() {
        let mut events = Events::default();
        events.set([choice("A", "x", None), choice("B", "y", None)].iter());
        assert!(!events.is_open("A") && !events.is_open("B"));
        events.set([choice("A", "x", None)].iter());
        assert!(events.is_open("A"));
    }

    #[test]
    fn refreshed_prices_keep_what_the_user_opened() {
        let mut events = Events::default();
        events.set([choice("A", "x", Some(0.1)), choice("B", "y", None)].iter());
        events.fold("B");
        events.set([choice("A", "x", Some(0.3)), choice("B", "y", None)].iter());
        assert!(events.is_open("B"));
        events.set([choice("C", "z", None), choice("B", "y", None)].iter());
        assert!(!events.is_open("B"));
        events.fold("C");
        events.fold("C");
        assert!(!events.is_open("C"), "folding twice is a no-op");
    }

    fn event(choices: &[(&str, Option<f64>)]) -> EventGroup {
        let results: Vec<SearchResult> = choices
            .iter()
            .map(|(label, price)| choice("Fed", label, *price))
            .collect();
        group_by_event(results.iter()).remove(0)
    }

    fn ticked_labels(picked: &SearchResult) -> Vec<&str> {
        picked
            .event
            .as_ref()
            .expect("event")
            .series
            .iter()
            .map(|choice| choice.label.as_str())
            .collect()
    }

    #[test]
    fn ticking_choices_builds_one_pick_led_by_the_first_in_event_order() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7)), ("Hike", Some(0.1))]);
        // The event lists Hold, Cut, Hike; ticking Hike then Cut still leads with Cut.
        let picked = toggle_choice(None, &group, "Fed/Hike");
        let picked = toggle_choice(picked.as_ref(), &group, "Fed/Cut").expect("pick");
        assert_eq!(picked.symbol, "Cut");
        assert_eq!(ticked_labels(&picked), ["Cut", "Hike"]);
        assert_eq!(group_tick(Some(&picked), &group), Tick::Some);
        assert_eq!(ticked_count(Some(&picked)), 2);
    }

    #[test]
    fn unticking_the_last_choice_clears_the_pick() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7))]);
        let picked = toggle_choice(None, &group, "Fed/Cut");
        assert!(toggle_choice(picked.as_ref(), &group, "Fed/Cut").is_none());
        assert_eq!(ticked_count(None), 0);
    }

    #[test]
    fn the_event_checkbox_ticks_everything_then_clears() {
        let group = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7)), ("Hike", None)]);
        let some = toggle_choice(None, &group, "Fed/Hold");
        let all = toggle_group(some.as_ref(), &group).expect("all");
        assert_eq!(ticked_labels(&all), ["Hold", "Cut", "Hike"]);
        assert_eq!(group_tick(Some(&all), &group), Tick::All);
        assert!(toggle_group(Some(&all), &group).is_none());
        assert_eq!(group_tick(None, &group), Tick::None);
    }

    #[test]
    fn ticking_in_another_event_drops_the_first_events_ticks() {
        let fed = event(&[("Cut", Some(0.2)), ("Hold", Some(0.7))]);
        let other = group_by_event(
            [
                choice("Vote", "Yes", Some(0.5)),
                choice("Vote", "No", Some(0.5)),
            ]
            .iter(),
        )
        .remove(0);
        let in_fed = toggle_choice(None, &fed, "Fed/Cut");
        let in_vote = toggle_choice(in_fed.as_ref(), &other, "Vote/No").expect("pick");
        assert_eq!(ticked_labels(&in_vote), ["No"]);
        assert_eq!(group_tick(Some(&in_vote), &fed), Tick::None);
    }
}
