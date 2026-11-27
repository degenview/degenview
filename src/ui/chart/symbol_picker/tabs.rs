//! The row of scope tabs and the pull-down at its right.

use super::*;

impl SymbolPicker {
    /// The tabs and the filter at their right. Returns what was chosen this frame.
    pub(super) fn tab_row(&self, ui: &mut Ui, area: Rect) -> (Option<Scope>, Option<Filter>) {
        let tabs = Scope::tabs(self.mode);
        let menu = self.filter_menu();
        let full_filter = menu.as_ref().map(|menu| filter_width(ui, &menu.label));
        let widths: Vec<f32> = tabs
            .iter()
            .map(|tab| {
                kit::galley(
                    ui,
                    tab.title(),
                    15.0,
                    if *tab == self.scope { SEMIBOLD } else { MEDIUM },
                    theme::text_secondary(),
                    f32::INFINITY,
                )
                .size()
                .x
            })
            .collect();
        let fit = fit_tabs(&widths, full_filter, area.width() - 2.0 * PADDING);

        // A language whose tabs do not fit even in the tightest row is cut where the pull-down
        // starts, never drawn under it.
        let tabs_right = full_filter.map_or(area.right(), |full| {
            let filter = if fit.compact { FILTER_COMPACT } else { full };
            area.right() - PADDING - filter - FILTER_GAP
        });
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            area.min,
            pos2(tabs_right, area.bottom()),
        ));
        let mut scope = None;
        let mut x = area.left() + PADDING;
        for tab in tabs {
            let active = tab == self.scope;
            let label = kit::galley(
                ui,
                tab.title(),
                15.0,
                if active { SEMIBOLD } else { MEDIUM },
                if active {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                },
                f32::INFINITY,
            );
            let width = label.size().x + fit.padding;
            let rect = Rect::from_min_size(pos2(x, area.top()), vec2(width, area.height()));
            let response = ui
                .interact(
                    rect,
                    Id::new(("symbol_picker_tab", tab as u8)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let response = crate::ui::accessibility::label(response, tab.title());
            let color = if active || response.hovered() {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            painter.galley(
                pos2(
                    rect.center().x - label.size().x / 2.0,
                    rect.center().y - label.size().y / 2.0,
                ),
                label,
                color,
            );
            if active {
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(rect.left() + 8.0, rect.bottom() - 2.0),
                        pos2(rect.right() - 8.0, rect.bottom()),
                    ),
                    1.0,
                    theme::text_primary(),
                );
            }
            if response.clicked() {
                scope = Some(tab);
            }
            x += width;
        }
        let filter = menu.and_then(|menu| self.filter_button(ui, area, &menu, fit.compact));
        (scope, filter)
    }

    /// What the search box asks for: a watchlist's picker names the list it adds to.
    pub(super) fn placeholder(&self) -> String {
        if self.mode == PickMode::Watchlist && self.scope.searches() {
            tr!("Search to add to {list}…", list = self.list_name)
        } else {
            self.scope.placeholder().to_owned()
        }
    }

    /// What the pull-down at the right of the tabs offers in this tab: the exchanges, or on the
    /// Dashboards tab the portfolios. `None` when there is nothing to choose between.
    pub(super) fn filter_menu(&self) -> Option<FilterMenu> {
        if self.scope == Scope::Dashboards {
            if self.portfolios.is_empty() {
                return None;
            }
            let current = self.card_scope_in_use();
            let mut entries = vec![(Filter::Portfolio(None), tr!("All Portfolios").to_owned())];
            entries.extend(self.portfolios.iter().map(|portfolio| {
                (
                    Filter::Portfolio(Some(portfolio.id)),
                    portfolio.name.clone(),
                )
            }));
            let label = entries
                .iter()
                .find(|(filter, _)| *filter == Filter::Portfolio(current))
                .map_or_else(
                    || tr!("All Portfolios").to_owned(),
                    |(_, name)| name.clone(),
                );
            return Some(FilterMenu {
                label,
                active: current.is_some(),
                tip: tr!("Filter by portfolio"),
                entries,
                current: Filter::Portfolio(current),
            });
        }
        let venues = self.scope.venues();
        if venues.is_empty() {
            return None;
        }
        // A venue typed as a prefix wins over the pull-down, so the pull-down says what is searched.
        let venue = self.lookup().venue;
        let mut entries = vec![(Filter::Venue(None), tr!("All venues").to_owned())];
        entries.extend(
            venues
                .iter()
                .map(|venue| (Filter::Venue(Some(*venue)), venue.display_name().to_owned())),
        );
        Some(FilterMenu {
            label: venue
                .map_or(tr!("All venues"), |venue| venue.display_name())
                .to_owned(),
            active: venue.is_some(),
            tip: tr!("Filter by exchange"),
            entries,
            current: Filter::Venue(venue),
        })
    }

    /// The pull-down at the right of the tabs; just its icon when the tabs need the room.
    pub(super) fn filter_button(
        &self,
        ui: &mut Ui,
        area: Rect,
        menu: &FilterMenu,
        compact: bool,
    ) -> Option<Filter> {
        let galley = kit::galley(
            ui,
            &menu.label,
            14.0,
            MEDIUM,
            theme::text_secondary(),
            180.0,
        );
        let width = if compact {
            FILTER_COMPACT
        } else {
            filter_width_of(galley.size().x)
        };
        let rect = Rect::from_min_size(
            pos2(area.right() - PADDING - width, area.center().y - 16.0),
            vec2(width, 32.0),
        );
        let response = ui
            .interact(rect, Id::new("symbol_picker_venue"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, menu.tip);
        let response = if compact {
            response.on_hover_text(&menu.label)
        } else {
            response
        };
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
        if response.hovered() || open {
            ui.painter().rect_filled(rect, 6.0, theme::ink(10));
        }
        let color = if menu.active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let mut x = rect.left() + 8.0;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0)),
            Icon::Filter,
            color,
        );
        x += 22.0;
        if !compact {
            ui.painter().galley(
                pos2(x, rect.center().y - galley.size().y / 2.0),
                galley.clone(),
                color,
            );
            x += galley.size().x + 6.0;
        } else {
            x += 2.0;
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 5.0, rect.center().y), Vec2::splat(10.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );

        // The chosen row keeps a check at its edge, which the width `fitted_menu_rows` gives a row
        // does not include: an empty hint adds the little room that keeps its label whole.
        let rows: Vec<(&str, Option<&str>)> = menu
            .entries
            .iter()
            .map(|(_, label)| (label.as_str(), Some("")))
            .collect();
        let mut chosen = None;
        kit::fitted_menu_rows(ui, &response, egui::RectAlign::BOTTOM_END, &rows, |ui| {
            kit::menu_scope(ui, 0.0);
            for (filter, label) in &menu.entries {
                let mut row = kit::MenuRow::new(label);
                row.reserve_icon = false;
                row.current = *filter == menu.current;
                if kit::menu_row(ui, row).clicked() {
                    chosen = Some(*filter);
                    ui.close();
                }
            }
        });
        chosen
    }
}

/// What the pull-down at the right of the tabs filters by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Filter {
    /// The markets of one exchange (`None`: every exchange of the tab).
    Venue(Option<DataSource>),
    /// The cards of one portfolio (`None`: every portfolio).
    Portfolio(Option<u64>),
}

/// The pull-down's choices and how it reads now.
pub(super) struct FilterMenu {
    pub(super) label: String,
    /// Something narrower than "all" is chosen.
    pub(super) active: bool,
    pub(super) tip: &'static str,
    pub(super) entries: Vec<(Filter, String)>,
    pub(super) current: Filter,
}

/// The room the pull-down takes when it is reduced to its icon and chevron.
pub(super) const FILTER_COMPACT: f32 = 48.0;
/// The gap between the tabs and the pull-down.
pub(super) const FILTER_GAP: f32 = 8.0;
/// Space a tab adds around its label, from roomy to tight.
pub(super) const TAB_PADDINGS: [f32; 3] = [28.0, 20.0, 14.0];

/// How wide the pull-down is with `label` written out: the icon, the label and the chevron, with
/// as much room after the chevron as before the icon.
pub(super) fn filter_width_of(label: f32) -> f32 {
    8.0 + 16.0 + 6.0 + label + 6.0 + 10.0 + 8.0
}

pub(super) fn filter_width(ui: &Ui, label: &str) -> f32 {
    filter_width_of(
        kit::galley(ui, label, 14.0, MEDIUM, theme::text_secondary(), 180.0)
            .size()
            .x,
    )
}

/// How the tab row spends its room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TabFit {
    /// Space each tab adds around its label.
    pub(super) padding: f32,
    /// The pull-down shows only its icon.
    pub(super) compact: bool,
}

/// Fits tabs with labels `labels` wide and a pull-down `filter` wide (`None`: there is none)
/// into `room`: first roomy tabs, then tighter tabs, then the pull-down reduced to its icon.
pub(super) fn fit_tabs(labels: &[f32], filter: Option<f32>, room: f32) -> TabFit {
    let attempts = [
        (TAB_PADDINGS[0], false),
        (TAB_PADDINGS[1], false),
        (TAB_PADDINGS[1], true),
        (TAB_PADDINGS[2], true),
    ];
    for (padding, compact) in attempts {
        let tabs: f32 = labels.iter().map(|label| label + padding).sum();
        let filter = filter.map_or(0.0, |full| {
            FILTER_GAP + if compact { FILTER_COMPACT } else { full }
        });
        if tabs + filter <= room {
            return TabFit { padding, compact };
        }
    }
    TabFit {
        padding: TAB_PADDINGS[2],
        compact: true,
    }
}
