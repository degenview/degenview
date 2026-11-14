//! The script list: search field, grouped collapsible sections, rows with a favorite star and
//! inline rename, the context menu, the New Script button and the footer link. Ported from the
//! sidebar of `PineStudioView` and its `ScriptRow`.

use crate::model::pine::script::{LocalScript, ScriptId, ScriptType};
use crate::pine_studio::{Cmd, PineStudio};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Color32, CursorIcon, FontId, Id, Margin, Rect, Sense, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

const ROW_HEIGHT: f32 = 34.0;
const HEADER_HEIGHT: f32 = 28.0;
/// The gutter shared by the search field, the row pills and the New Script button.
const GUTTER: f32 = 8.0;
/// The tinted tile that carries a row's type glyph.
const TILE: f32 = 22.0;
const BUTTON_HEIGHT: f32 = 30.0;
pub(in crate::pine_studio) const COMMUNITY_URL: &str = "https://www.tradingview.com/scripts/";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Group {
    Favorites,
    Indicators,
    Strategies,
    Libraries,
}

impl Group {
    pub const ALL: [Self; 4] = [
        Self::Favorites,
        Self::Indicators,
        Self::Strategies,
        Self::Libraries,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Favorites => "Favorites",
            Self::Indicators => "Indicators",
            Self::Strategies => "Strategies",
            Self::Libraries => "Libraries",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Favorites => "favorites",
            Self::Indicators => "indicators",
            Self::Strategies => "strategies",
            Self::Libraries => "libraries",
        }
    }
}

impl ScriptType {
    /// The glyph that says what a script is at a glance: a waveform for an indicator, two
    /// opposed arrows for a strategy (it trades), a stack for a library (it is built upon). Every
    /// sidebar icon is the same light gray; the shape alone tells the types apart.
    fn glyph(self) -> Icon {
        match self {
            Self::Indicator => Icon::Waveform,
            Self::Strategy => Icon::Exchange,
            Self::Library => Icon::Layers,
        }
    }
}

/// A list row is a script in a group; a favorite appears in two groups, so selection is by row,
/// not by script.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RowKey {
    pub group: Group,
    pub id: ScriptId,
}

/// A script row's inline rename in progress.
pub struct Rename {
    pub row: RowKey,
    pub text: String,
    pub error: Option<String>,
    pub needs_focus: bool,
}

/// Scripts matching `query` (name or source, case-insensitive), newest first, by group.
/// Empty groups are omitted.
pub fn grouped(scripts: &[LocalScript], query: &str) -> Vec<(Group, Vec<LocalScript>)> {
    let needle = query.trim().to_lowercase();
    let matching: Vec<&LocalScript> = scripts
        .iter()
        .filter(|s| {
            needle.is_empty()
                || s.name.to_lowercase().contains(&needle)
                || s.source.to_lowercase().contains(&needle)
        })
        .collect();
    let mut sorted = matching;
    sorted.sort_by_key(|s| std::cmp::Reverse(s.modified_at_ms));
    Group::ALL
        .iter()
        .filter_map(|&group| {
            let rows: Vec<LocalScript> = sorted
                .iter()
                .filter(|s| match group {
                    Group::Favorites => s.is_favorite,
                    Group::Indicators => s.script_type == ScriptType::Indicator,
                    Group::Strategies => s.script_type == ScriptType::Strategy,
                    Group::Libraries => s.script_type == ScriptType::Library,
                })
                .map(|s| (*s).clone())
                .collect();
            (!rows.is_empty()).then_some((group, rows))
        })
        .collect()
}

impl PineStudio {
    pub(in crate::pine_studio) fn sidebar_ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, theme::chrome());
        // The footer and the New Script button are pinned; the list takes the rest.
        let footer_height = crate::pine_studio::drawer::bottom_bar::HEIGHT;
        let button_height = BUTTON_HEIGHT + 2.0 * GUTTER;
        let list_rect = Rect::from_min_max(
            rect.min,
            pos2(rect.max.x, rect.max.y - footer_height - button_height),
        );

        let mut list = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        list.set_clip_rect(list_rect.intersect(ui.clip_rect()));
        self.search_field(&mut list);
        egui::ScrollArea::vertical()
            .id_salt("script-list-scroll")
            .auto_shrink([false, false])
            .show(&mut list, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.rows(ui);
            });

        let button_rect = Rect::from_min_max(
            pos2(rect.min.x, list_rect.max.y),
            pos2(rect.max.x, list_rect.max.y + button_height),
        );
        // A hairline sets the pinned button apart from the list that scrolls under it.
        ui.painter().hline(
            rect.x_range(),
            button_rect.min.y,
            egui::Stroke::new(1.0, theme::separator()),
        );
        self.new_script_button(ui, button_rect);
        self.footer(ui, rect, footer_height);
    }

    fn search_field(&mut self, ui: &mut Ui) {
        egui::Frame::new()
            .inner_margin(Margin::same(GUTTER as i8))
            .show(ui, |ui| {
                let width = ui.available_width();
                kit::search_field(
                    ui,
                    Id::new("script-search"),
                    &mut self.search,
                    "Search name or source",
                    width,
                );
            });
    }

    fn rows(&mut self, ui: &mut Ui) {
        let groups = grouped(&self.scripts, &self.search);
        if groups.is_empty() {
            self.empty_list(ui);
            return;
        }
        for (index, (group, scripts)) in groups.into_iter().enumerate() {
            let collapsed = self.collapsed.contains(&group);
            // Sections breathe: the gap above a header separates it from the rows it closes.
            if index > 0 {
                ui.add_space(6.0);
            }
            self.section_header(ui, group, scripts.len(), collapsed);
            if collapsed {
                continue;
            }
            for script in scripts {
                self.row(ui, group, &script);
            }
        }
        ui.add_space(GUTTER);
    }

    /// Nothing to list: either the folder is empty or the search matched nothing.
    fn empty_list(&mut self, ui: &mut Ui) {
        if self.search.trim().is_empty() {
            super::empty_library::sidebar(ui, &mut self.cmds);
            return;
        }
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
        let width = rect.width() - 2.0 * (GUTTER + 12.0);
        let painter = ui.painter();
        let title = kit::galley(
            ui,
            "No matches",
            12.0,
            kit::SEMIBOLD,
            theme::text_primary(),
            width,
        );
        let detail = {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                "No script's name or source contains that text.",
                0.0,
                egui::TextFormat {
                    font_id: FontId::proportional(11.0),
                    color: theme::text_secondary(),
                    ..Default::default()
                },
            );
            job.wrap.max_width = width;
            job.halign = Align::Center;
            painter.layout_job(job)
        };
        let mut y = rect.min.y + 34.0;
        let tile = Rect::from_center_size(pos2(rect.center().x, y + 14.0), Vec2::splat(28.0));
        painter.rect_filled(tile, 8.0, theme::quaternary(0.7));
        icons::paint(
            painter,
            Rect::from_center_size(tile.center(), Vec2::splat(14.0)),
            Icon::Magnifier,
            theme::text_secondary(),
        );
        y += 28.0 + 10.0;
        painter.galley(
            pos2(rect.center().x - title.size().x / 2.0, y),
            title.clone(),
            theme::text_primary(),
        );
        y += title.size().y + 3.0;
        painter.galley(pos2(rect.center().x, y), detail, theme::text_secondary());
    }

    fn section_header(&mut self, ui: &mut Ui, group: Group, count: usize, collapsed: bool) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::click());
        let response = crate::ui::accessibility::label(response, group.title());
        let hovered = response.hovered();
        let pill = rect.shrink2(vec2(GUTTER, 1.0));
        if hovered {
            ui.painter().rect_filled(pill, 6.0, theme::ink(8));
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };

        // The chevron is always there: a section says it can fold before the pointer finds out.
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(pill.min.x + 12.0, pill.center().y), Vec2::splat(10.0)),
            if collapsed {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            },
            color,
        );
        let title = kit::galley(
            ui,
            &group.title().to_uppercase(),
            10.0,
            kit::BOLD,
            color,
            pill.width() - 60.0,
        );
        let title_x = pill.min.x + 24.0;
        let title_width = title.size().x;
        ui.painter().galley(
            pos2(title_x, pill.center().y - title.size().y / 2.0),
            title,
            color,
        );
        if group == Group::Favorites {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(
                    pos2(title_x + title_width + 9.0, pill.center().y),
                    Vec2::splat(9.0),
                ),
                Icon::StarFill,
                theme::text_secondary(),
            );
        }

        let digits = kit::digits(
            ui,
            &count.to_string(),
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
            f32::INFINITY,
        );
        kit::paint_count(
            ui,
            pill.max.x - 8.0 - (digits.size().x + 12.0),
            pill.center().y,
            count,
            theme::text_secondary(),
            theme::quaternary(0.7),
        );
        if response.clicked() && !self.collapsed.remove(&group) {
            self.collapsed.insert(group);
        }
    }

    fn row(&mut self, ui: &mut Ui, group: Group, script: &LocalScript) {
        let key = RowKey {
            group,
            id: script.id.clone(),
        };
        let selected = self.selected_row.as_ref() == Some(&key);
        let renaming = self.renaming.as_ref().is_some_and(|r| r.row == key);
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
        let response = crate::ui::accessibility::label(response, &script.name);
        let pill = rect.shrink2(vec2(GUTTER, 1.0));
        let id = Id::new(("script-row", group.key(), &script.id));
        let hovered = response.hovered() && !renaming;

        if selected {
            ui.painter()
                .rect_filled(pill, 7.0, theme::surface_selected());
            ui.painter().rect_stroke(
                pill,
                7.0,
                egui::Stroke::new(1.0, theme::border()),
                egui::StrokeKind::Inside,
            );
            // A short accent bar on the leading edge marks the open row.
            ui.painter().rect_filled(
                Rect::from_center_size(
                    pos2(pill.min.x + 3.5, pill.center().y),
                    vec2(3.0, ROW_HEIGHT - 16.0),
                ),
                1.5,
                theme::SYSTEM_ACCENT,
            );
        } else if hovered {
            ui.painter().rect_filled(pill, 7.0, theme::ink(12));
        }

        // The type tile.
        let type_icon = script.script_type.glyph();
        let tile = Rect::from_center_size(
            pos2(pill.min.x + 11.0 + TILE / 2.0, pill.center().y),
            Vec2::splat(TILE),
        );
        ui.painter().rect_filled(
            tile,
            6.0,
            theme::quaternary(if selected || hovered { 1.0 } else { 0.7 }),
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(tile.center(), Vec2::splat(12.0)),
            type_icon,
            theme::text_secondary(),
        );

        // The favorite star sits at the trailing edge and takes its own clicks. An unfavorited
        // star only shows while the row is under the pointer or selected.
        let star_rect =
            Rect::from_center_size(pill.right_center() - vec2(14.0, 0.0), Vec2::splat(22.0));
        let star = ui.interact(star_rect, id.with("star"), Sense::click());
        if script.is_favorite || selected || hovered || star.hovered() {
            let star_color = if star.hovered() {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            icons::paint(
                ui.painter(),
                Rect::from_center_size(star_rect.center(), Vec2::splat(12.0)),
                if script.is_favorite {
                    Icon::StarFill
                } else {
                    Icon::Star
                },
                star_color,
            );
        }
        if star.clicked() {
            self.cmds.push(Cmd::ToggleFavorite(script.id.clone()));
            return;
        }

        let text_left = tile.max.x + 9.0;
        if renaming {
            self.rename_field(
                ui,
                id,
                Rect::from_min_max(
                    pos2(text_left, pill.min.y + 2.0),
                    pos2(pill.max.x - 30.0, pill.max.y - 2.0),
                ),
            );
        } else {
            let weight = if selected {
                kit::SEMIBOLD
            } else {
                kit::REGULAR
            };
            let text_color = theme::text_primary();
            let name = kit::galley(
                ui,
                &script.name,
                13.0,
                weight,
                text_color,
                pill.max.x - 30.0 - text_left,
            );
            ui.painter().galley(
                pos2(text_left, rect.center().y - name.size().y / 2.0),
                name,
                text_color,
            );
        }

        if response.clicked() && !renaming && !star.hovered() {
            self.row_clicked(key.clone());
        }
        egui::Popup::context_menu(&response)
            .frame(kit::menu_frame())
            .show(|ui| {
                ui.set_width(190.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                self.row_menu(ui, &key, script);
            });
    }

    /// A click selects; a slow second click on the selected row (0.4–1.5 s later) renames, like
    /// Finder.
    fn row_clicked(&mut self, key: RowKey) {
        let now = Instant::now();
        let slow_second = self.selected_row.as_ref() == Some(&key)
            && self.last_click.as_ref().is_some_and(|(row, at)| {
                row == &key && {
                    let gap = now.duration_since(*at);
                    gap > Duration::from_millis(400) && gap < Duration::from_millis(1500)
                }
            });
        self.last_click = Some((key.clone(), now));
        if slow_second {
            self.cmds.push(Cmd::BeginRename(key));
        } else {
            self.cmds.push(Cmd::Select(key));
        }
    }

    fn rename_field(&mut self, ui: &mut Ui, id: Id, rect: Rect) {
        let Some(rename) = self.renaming.as_mut() else {
            return;
        };
        let color = if rename.error.is_some() {
            Color32::from_rgb(0xFF, 0x45, 0x3A)
        } else {
            theme::text_primary()
        };
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let response = field.add(
            egui::TextEdit::singleline(&mut rename.text)
                .id(id.with("rename"))
                .font(FontId::proportional(13.0))
                .text_color(color)
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(rect.width()),
        );
        if rename.needs_focus {
            response.request_focus();
            rename.needs_focus = false;
            // Select the whole name, like a Finder rename.
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
                let end = rename.text.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0usize),
                        egui::text::CCursor::new(end),
                    )));
                state.store(ui.ctx(), response.id);
            }
        }
        if rename.error.is_some() {
            rename.error = rename.error.take().filter(|_| !response.changed());
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            self.cmds.push(Cmd::CancelRename);
        } else if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.cmds.push(Cmd::CommitRename);
        } else if response.lost_focus() {
            // Clicking away commits too, like a native text field.
            self.cmds.push(Cmd::CommitRename);
        }
    }

    fn row_menu(&mut self, ui: &mut Ui, key: &RowKey, script: &LocalScript) {
        let (favorite_icon, favorite) = if script.is_favorite {
            (Icon::Star, "Remove Favorite")
        } else {
            (Icon::StarFill, "Favorite")
        };
        let mut item = |ui: &mut Ui, icon: Icon, label: &str, destructive: bool, cmd: Cmd| {
            if kit::menu_item(ui, icon, label, true, destructive) {
                self.cmds.push(cmd);
                ui.close();
            }
        };
        item(
            ui,
            favorite_icon,
            favorite,
            false,
            Cmd::ToggleFavorite(script.id.clone()),
        );
        item(
            ui,
            Icon::Pencil,
            "Rename",
            false,
            Cmd::BeginRename(key.clone()),
        );
        item(
            ui,
            Icon::CopyDoc,
            "Duplicate",
            false,
            Cmd::Duplicate(script.id.clone()),
        );
        kit::menu_separator(ui);
        item(
            ui,
            Icon::Books,
            &format!("Show in {}", crate::platform::file_manager()),
            false,
            Cmd::RevealFile(script.id.clone()),
        );
        item(
            ui,
            Icon::ShareUp,
            "Export…",
            false,
            Cmd::Export(script.id.clone()),
        );
        // Until the chart settings gain their Scripts tab (phase 2), this is the only way a
        // script reaches a chart or an alert.
        if script.script_type != ScriptType::Library {
            kit::menu_separator(ui);
            item(
                ui,
                Icon::ChartLine,
                "Add to Chart",
                false,
                Cmd::AddToChart(script.id.clone()),
            );
            item(
                ui,
                Icon::Bell,
                "Create Alert",
                false,
                Cmd::CreateAlert(script.id.clone()),
            );
        }
        kit::menu_separator(ui);
        item(
            ui,
            Icon::Trash,
            "Delete",
            true,
            Cmd::AskDelete(script.id.clone()),
        );
    }

    fn new_script_button(&mut self, ui: &mut Ui, area: Rect) {
        // Same gutter as the search field and the row pills above it.
        let rect = area.shrink(GUTTER);
        let response = ui
            .interact(rect, Id::new("new-script"), Sense::click())
            .on_hover_text(shortcut::NEW_SCRIPT.hint("Create a new script"));
        let response = crate::ui::accessibility::label(response, "New Script");
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        // A neutral card, never the accent: see "Buttons are never blue" in the core guide.
        let hovered = response.hovered();
        let fill = if response.is_pointer_button_down_on() {
            theme::ink(30)
        } else {
            theme::card_fill(hovered)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, fill);
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, theme::card_border(hovered)),
            egui::StrokeKind::Inside,
        );
        painter.hline(
            rect.x_range().shrink(8.0),
            rect.min.y + 1.0,
            egui::Stroke::new(1.0, theme::card_highlight()),
        );
        let color = theme::text_primary();
        let label = kit::galley(
            ui,
            "New Script",
            12.5,
            kit::SEMIBOLD,
            color,
            rect.width() - 40.0,
        );
        // The plus and the label read as one group, centred in the button.
        let gap = 6.0;
        let icon = 11.0;
        let start = rect.center().x - (icon + gap + label.size().x) / 2.0;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(start + icon / 2.0, rect.center().y), Vec2::splat(icon)),
            Icon::Plus,
            color,
        );
        ui.painter().galley(
            pos2(start + icon + gap, rect.center().y - label.size().y / 2.0),
            label,
            color,
        );
        if response.clicked() {
            self.cmds.push(Cmd::NewScript);
        }
    }

    fn footer(&mut self, ui: &mut Ui, rect: Rect, height: f32) {
        let footer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - height), rect.max);
        crate::pine_studio::drawer::bottom_bar::paint(ui, footer);
        community_link(ui, pos2(footer.min.x + 12.0, footer.center().y - 8.0));
        let count = self.scripts.len();
        let text = format!("{count} {}", if count == 1 { "script" } else { "scripts" });
        let galley = kit::galley(
            ui,
            &text,
            10.0,
            kit::REGULAR,
            theme::text_tertiary(),
            f32::INFINITY,
        );
        ui.painter().galley(
            pos2(
                footer.max.x - 12.0 - galley.size().x,
                footer.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::text_tertiary(),
        );
    }
}

/// "Community scripts ↗" at `origin`; opens the TradingView scripts page.
pub fn community_link(ui: &mut Ui, origin: egui::Pos2) -> egui::Response {
    let font = FontId::proportional(10.0);
    let galley = ui.painter().layout_no_wrap(
        "Community scripts".to_owned(),
        font.clone(),
        theme::text_secondary(),
    );
    let width = galley.size().x + 14.0;
    let rect = Rect::from_min_size(origin, vec2(width, 16.0));
    let response = ui
        .interact(
            rect,
            Id::new(("community-link", origin.x as i32, origin.y as i32)),
            Sense::click(),
        )
        .on_hover_text("Browse community scripts on TradingView");
    let color = if response.hovered() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    // A Label: the glyph leads, as in Swift.
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.min.x + 5.0, rect.center().y), Vec2::splat(10.0)),
        Icon::ArrowUpRightSquare,
        color,
    );
    ui.painter().galley(
        pos2(rect.min.x + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(COMMUNITY_URL));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(
        id: &str,
        name: &str,
        script_type: ScriptType,
        favorite: bool,
        modified: i64,
        source: &str,
    ) -> LocalScript {
        LocalScript {
            id: id.into(),
            name: name.into(),
            script_type,
            source: source.into(),
            latest_revision_id: None,
            created_at_ms: 0,
            modified_at_ms: modified,
            is_favorite: favorite,
            compile_record: None,
        }
    }

    #[test]
    fn groups_sort_newest_first_and_favorites_appear_twice() {
        let scripts = vec![
            script("a", "Alpha", ScriptType::Indicator, false, 1, ""),
            script("b", "Beta", ScriptType::Indicator, true, 3, ""),
            script("c", "Gamma", ScriptType::Strategy, false, 2, ""),
        ];
        let groups = grouped(&scripts, "");
        let titles: Vec<&str> = groups.iter().map(|(g, _)| g.title()).collect();
        assert_eq!(
            titles,
            ["Favorites", "Indicators", "Strategies"],
            "empty Libraries hidden"
        );
        let indicators: Vec<&str> = groups[1].1.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(indicators, ["Beta", "Alpha"]);
        assert_eq!(groups[0].1[0].name, "Beta");
    }

    #[test]
    fn search_matches_name_or_source_case_insensitively() {
        let scripts = vec![
            script("a", "Alpha", ScriptType::Indicator, false, 1, "plot(close)"),
            script("b", "Beta", ScriptType::Indicator, false, 2, "plot(OPEN)"),
        ];
        let names = |q: &str| -> Vec<String> {
            grouped(&scripts, q)
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .map(|s| s.name)
                .collect()
        };
        assert_eq!(names("ALP"), ["Alpha"]);
        assert_eq!(names("open"), ["Beta"]);
        assert_eq!(names("  "), ["Beta", "Alpha"]);
        assert!(names("zzz").is_empty());
    }
}
