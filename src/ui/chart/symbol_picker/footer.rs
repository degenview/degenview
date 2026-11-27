//! The key hints along the foot of the picker, and what Return does in words.

use super::*;

impl SymbolPicker {
    /// The keys that work here, and the button that charts ticked choices together. A narrow
    /// picker drops the least needed hints rather than run past its edge.
    pub(super) fn footer(&self, ui: &mut Ui, area: Rect, env: &Env) -> Option<(Pick, PickKind)> {
        let mut hit = None;
        let mut hints = vec![
            Hint::new(&["↑", "↓"], tr!("Navigate")),
            Hint::new(&["↵"], self.return_hint(env)),
        ];
        let market = self.highlighted.as_ref().and_then(Item::market);
        if self.mode == PickMode::Chart
            && env.replacing.is_some()
            && !matches!(self.highlighted, Some(Item::Event(_)))
        {
            hints.push(Hint::new(&["⇧", "↵"], tr!("Add to this tab")));
        }
        if self.mode == PickMode::Chart && env.can_overlay && market.is_some_and(Self::comparable) {
            hints.push(Hint::new(&["⌥", "↵"], tr!("Compare")));
        }
        hints.push(Hint::new(&["Tab"], tr!("Switch tab")));
        hints.push(Hint::new(&["Esc"], tr!("Close")));

        // What sits at the right edge: the button for ticked choices, else what was just added.
        let together = self
            .ticked
            .as_ref()
            .filter(|ticked| predictions::ticked_count(Some(ticked)) >= 1);
        let mut right = area.right() - PADDING;
        if let Some(ticked) = together {
            let label = self.ticked_label(predictions::ticked_count(Some(ticked)));
            let width = kit::galley(
                ui,
                &label,
                13.0,
                MEDIUM,
                Color32::PLACEHOLDER,
                f32::INFINITY,
            )
            .size()
            .x + 32.0;
            let width = width.max(76.0);
            let button = Rect::from_min_size(
                pos2(right - width, area.center().y - 14.0),
                vec2(width, 28.0),
            );
            let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(button));
            if widgets::sheet_button(&mut cell, &label, true, true).clicked() {
                hit = Some((Pick::Market(ticked.clone()), self.default_kind()));
            }
            right = button.left() - 16.0;
        } else if let Some(added) = &self.added {
            let galley = kit::galley(
                ui,
                &tr!("Added {market}", market = added),
                13.0,
                REGULAR,
                theme::text_secondary(),
                (area.width() * 0.4).max(80.0),
            );
            let left = right - galley.size().x;
            ui.painter().galley(
                pos2(left, area.center().y - galley.size().y / 2.0),
                galley,
                theme::text_secondary(),
            );
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(left - 14.0, area.center().y), Vec2::splat(14.0)),
                Icon::Check,
                theme::UP,
            );
            right = left - 30.0;
        }

        let row = Rect::from_min_max(
            pos2(area.left() + PADDING, area.top()),
            pos2(right, area.bottom()),
        );
        spaced_hint_row(
            ui,
            row,
            hints,
            13.0,
            WIDE_HINT_GAP,
            &[
                tr!("Compare"),
                tr!("Add to this tab"),
                tr!("Switch tab"),
                tr!("Navigate"),
            ],
        );
        hit
    }

    /// What the footer's button for `count` ticked choices says.
    pub(super) fn ticked_label(&self, count: usize) -> String {
        match self.mode {
            PickMode::Watchlist => tr_n!(count, "Add {count} choice", "Add {count} choices"),
            _ => tr_n!(count, "Chart {count} choice", "Chart {count} choices"),
        }
    }

    /// What Return does on the highlighted row, in the footer's words.
    pub(super) fn return_hint(&self, env: &Env) -> String {
        let list = &self.list_name;
        let to_watchlist = |market: Option<&SearchResult>| match market {
            Some(market)
                if self.held(env, &InstrumentId::new(market.source, &market.full_symbol)) =>
            {
                tr!("Already in {list}", list = list)
            }
            Some(market) => tr!(
                "Add {market} to {list}",
                market = market_label(market),
                list = list
            ),
            None => tr!("Add to {list}", list = list),
        };
        match &self.highlighted {
            Some(Item::Event(head)) => {
                return if head.open {
                    tr!("Hide choices")
                } else {
                    tr!("Show choices")
                }
                .to_owned();
            }
            Some(Item::Tile(Tile {
                pick: TilePick::Card(card),
                ..
            })) => {
                return match self.mode {
                    PickMode::Watchlist if self.held(env, &card.instrument_id()) => {
                        tr!("Already in {list}", list = list)
                    }
                    PickMode::Watchlist => to_watchlist(None),
                    _ => tr!("Add card").to_owned(),
                };
            }
            _ => {}
        }
        match (self.mode, self.replacing(env)) {
            (PickMode::Compare, _) if env.replacing.is_some() => {
                tr!(
                    "Compare with {market}",
                    market = env.replacing.unwrap_or_default()
                )
            }
            (PickMode::Compare, _) => tr!("Compare").to_owned(),
            (PickMode::Chart, Some(current)) => tr!("Replace {market}", market = current),
            (PickMode::Chart | PickMode::Add, _) => tr!("Add chart").to_owned(),
            (PickMode::Watchlist, _) => {
                to_watchlist(self.highlighted.as_ref().and_then(Item::market))
            }
        }
    }
}

/// The longest market name the footer's Return hint spells out; an event's title is longer.
pub(super) const HINT_NAME_CHARS: usize = 28;

/// A market as the footer names it before it is picked: its venue and its name, as the row has them.
pub(super) fn market_label(result: &SearchResult) -> String {
    // A lone "Yes" says nothing: a prediction market is named by its event.
    let name = match &result.event {
        Some(event) if !event.title.is_empty() => event.title.clone(),
        _ => pick_name(result),
    };
    let name = if name.chars().count() > HINT_NAME_CHARS {
        let cut: String = name.chars().take(HINT_NAME_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        name
    };
    format!("{} · {name}", lookup::short_name(result.source))
}
