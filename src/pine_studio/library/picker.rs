//! The script picker: the popup under the bar's script button. A search field, the scripts in
//! collapsible groups with a favorite star, a right-click menu on each row, and a footer with the
//! New Script button and the community link. Choosing a row opens the script and closes the
//! popup; the menu's Rename opens a sheet, never an inline box.

use super::empty_library;
use super::script_actions::script_groups;
use crate::model::pine::script::{LocalScript, ScriptType};
use crate::pine_studio::commands::Cmd;
use crate::ui::icons::{self, Icon};
use crate::ui::shortcut;
use crate::ui::{kit, theme};
use eframe::egui::{self, CursorIcon, Id, Rect, Response, Sense, Ui, Vec2, pos2, vec2};
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The popup's content width; its frame adds the margin around it. A fixed width, as the market
/// picker's popup (`preview::market_picker`) has: a search popup holds rows of any length, so it is
/// not fitted to them the way a menu is.
const WIDTH: f32 = 340.0;
/// The most the list grows to before it scrolls.
const LIST_MAX_HEIGHT: f32 = 380.0;
const ROW_HEIGHT: f32 = 30.0;
const HEADER_HEIGHT: f32 = 26.0;
const ROW_RADIUS: f32 = 6.0;
const ICON: f32 = 14.0;
const STAR: f32 = 12.0;
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

/// The glyph that says what a script is at a glance: a waveform for an indicator, two opposed
/// arrows for a strategy (it trades), a stack for a library (it is built upon). Every icon is the
/// same gray; the shape alone tells the types apart.
fn type_glyph(script_type: ScriptType) -> Icon {
    match script_type {
        ScriptType::Indicator => Icon::Waveform,
        ScriptType::Strategy => Icon::Exchange,
        ScriptType::Library => Icon::Layers,
    }
}

/// The scripts of each group as indexes into the list they were made from.
pub type Groups = Vec<(Group, Vec<usize>)>;

/// Scripts matching `query` (name or source, case-insensitive), newest first, by group. Empty
/// groups are omitted.
pub fn grouped(scripts: &[LocalScript], query: &str) -> Groups {
    let needle = query.trim().to_lowercase();
    let mut matching: Vec<usize> = (0..scripts.len())
        .filter(|&index| {
            let script = &scripts[index];
            needle.is_empty()
                || script.name.to_lowercase().contains(&needle)
                || script.source.to_lowercase().contains(&needle)
        })
        .collect();
    matching.sort_by_key(|&index| std::cmp::Reverse(scripts[index].modified_at_ms));
    Group::ALL
        .iter()
        .filter_map(|&group| {
            let rows: Vec<usize> = matching
                .iter()
                .copied()
                .filter(|&index| {
                    let script = &scripts[index];
                    match group {
                        Group::Favorites => script.is_favorite,
                        Group::Indicators => script.script_type == ScriptType::Indicator,
                        Group::Strategies => script.script_type == ScriptType::Strategy,
                        Group::Libraries => script.script_type == ScriptType::Library,
                    }
                })
                .collect();
            (!rows.is_empty()).then_some((group, rows))
        })
        .collect()
}

/// What `grouped` reads of a library, without its sources: when this is the same the groups are,
/// since a script's source changes only when it is saved, which moves its modified time.
fn signature(scripts: &[LocalScript]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for script in scripts {
        (
            &script.id,
            &script.name,
            script.script_type,
            script.is_favorite,
            script.modified_at_ms,
        )
            .hash(&mut hasher);
    }
    hasher.finish()
}

/// Whether the popup is up, what is typed in its search field and which groups are folded.
#[derive(Default)]
pub(in crate::pine_studio) struct Picker {
    open: bool,
    search: String,
    needs_focus: bool,
    collapsed: HashSet<Group>,
    /// The groups for a search and a library (`signature`), kept while neither changes: matching
    /// reads every script's source, which is not for each frame.
    groups: Option<((String, u64), Arc<Groups>)>,
}

impl Picker {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the popup with an empty search, ready to type.
    pub fn open(&mut self) {
        self.open = true;
        self.search.clear();
        self.needs_focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The groups of `scripts` for the search, worked out again only when either has changed.
    fn groups(&mut self, scripts: &[LocalScript]) -> Arc<Groups> {
        let key = (self.search.clone(), signature(scripts));
        if let Some((cached, groups)) = &self.groups
            && *cached == key
        {
            return Arc::clone(groups);
        }
        let groups = Arc::new(grouped(scripts, &self.search));
        self.groups = Some((key, Arc::clone(&groups)));
        groups
    }

    fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }
}

/// What one frame of the popup collects: the commands, and whether the popup should go.
struct Pass<'a> {
    cmds: &'a mut Vec<Cmd>,
    close: bool,
    /// A row's right-click menu is up: it hangs outside the popup, so a press there is not a press
    /// away from it.
    menu_open: bool,
}

impl Pass<'_> {
    /// A command that takes the person elsewhere (a script, a sheet): the popup closes.
    fn run(&mut self, cmd: Cmd) {
        self.cmds.push(cmd);
        self.close = true;
    }

    /// A command that changes the list and leaves the popup where it is.
    fn keep_open(&mut self, cmd: Cmd) {
        self.cmds.push(cmd);
    }
}

/// The popup under the bar's script button (`button`): the button toggles it, a press anywhere
/// else outside it (and outside a row menu) or Escape dismisses it. `open_id` is the script in the
/// editor.
pub(in crate::pine_studio) fn show(
    ui: &mut Ui,
    picker: &mut Picker,
    scripts: &[LocalScript],
    open_id: Option<&str>,
    button: &Response,
    cmds: &mut Vec<Cmd>,
) {
    if button.clicked() {
        picker.toggle();
    }
    if !picker.open {
        return;
    }
    // Not egui's single popup slot: a row's menu opens through it and must not close this.
    let popup = egui::Popup::from_response(button)
        .id(Id::new("script-picker"))
        .open(true)
        .anchor(egui::PopupAnchor::ParentRect(button.rect))
        .align(egui::RectAlign::BOTTOM_START)
        .gap(6.0)
        .frame(kit::menu_frame());
    let mut pass = Pass {
        cmds,
        close: false,
        menu_open: false,
    };
    let shown = popup.show(|ui| {
        ui.set_width(WIDTH);
        body(ui, picker, scripts, open_id, &mut pass);
    });
    let Some(shown) = shown else {
        picker.close();
        return;
    };
    let dismissed = ui.input(|i| {
        i.key_pressed(egui::Key::Escape)
            || (i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|pos| {
                    !shown.response.rect.contains(pos) && !button.rect.contains(pos)
                }))
    });
    if pass.close || (dismissed && !pass.menu_open) {
        picker.close();
    }
    // The popup has had Esc; the editor under it must not also act on it (close its find bar).
    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
}

fn body(
    ui: &mut Ui,
    picker: &mut Picker,
    scripts: &[LocalScript],
    open_id: Option<&str>,
    pass: &mut Pass,
) {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let groups = picker.groups(scripts);
    // With no scripts there is nothing to search.
    if !scripts.is_empty() {
        search_field(ui, picker, scripts, &groups, pass);
        ui.add_space(6.0);
    }
    egui::ScrollArea::vertical()
        .id_salt("script-picker-scroll")
        .max_height(LIST_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if scripts.is_empty() {
                empty_library::short(ui, pass.cmds);
            } else if groups.is_empty() {
                no_matches(ui);
            } else {
                list(ui, picker, scripts, &groups, open_id, pass);
            }
        });
    ui.add_space(6.0);
    kit::divider(ui);
    footer(ui, scripts.len(), pass);
}

fn search_field(
    ui: &mut Ui,
    picker: &mut Picker,
    scripts: &[LocalScript],
    groups: &Groups,
    pass: &mut Pass,
) {
    let width = ui.available_width();
    let response = kit::search_field(
        ui,
        Id::new("script-picker-search"),
        &mut picker.search,
        "Search name or source",
        width,
    );
    if picker.needs_focus {
        response.request_focus();
        picker.needs_focus = !response.has_focus();
    }
    // Return opens the first script listed.
    if response.lost_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && let Some(first) = groups.first().and_then(|(_, rows)| rows.first())
    {
        pass.run(Cmd::Select(scripts[*first].id.clone()));
    }
}

fn list(
    ui: &mut Ui,
    picker: &mut Picker,
    scripts: &[LocalScript],
    groups: &Groups,
    open_id: Option<&str>,
    pass: &mut Pass,
) {
    let show_in = crate::platform::show_in_file_manager();
    for (index, (group, rows)) in groups.iter().enumerate() {
        // Sections breathe: the gap above a header separates it from the rows it closes.
        if index > 0 {
            ui.add_space(6.0);
        }
        let (group, collapsed) = (*group, picker.collapsed.contains(group));
        if section_header(ui, group, rows.len(), collapsed) && !picker.collapsed.remove(&group) {
            picker.collapsed.insert(group);
        }
        if collapsed {
            continue;
        }
        for &index in rows {
            let script = &scripts[index];
            let selected = open_id == Some(script.id.as_str());
            row(ui, group, script, selected, show_in, pass);
        }
    }
}

/// Nothing matches the search.
fn no_matches(ui: &mut Ui) {
    ui.add_space(22.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        icons::paint(ui.painter(), rect, Icon::Magnifier, theme::text_secondary());
        ui.add_space(8.0);
        ui.label(kit::text(
            "No matches",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(3.0);
        ui.label(kit::text(
            "No script's name or source contains that text.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
    ui.add_space(22.0);
}

/// A group's heading; true when it was clicked (to fold or unfold it).
fn section_header(ui: &mut Ui, group: Group, count: usize, collapsed: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, group.title());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, ROW_RADIUS, theme::ink(8));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let color = if hovered {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    // The chevron is always there: a group says it can fold before the pointer finds out.
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), Vec2::splat(10.0)),
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
        kit::size::CAPTION,
        kit::BOLD,
        color,
        rect.width() - 60.0,
    );
    let title_x = rect.min.x + 24.0;
    let title_width = title.size().x;
    ui.painter().galley(
        pos2(title_x, rect.center().y - title.size().y / 2.0),
        title,
        color,
    );
    if group == Group::Favorites {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(title_x + title_width + 9.0, rect.center().y),
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
        rect.max.x - 8.0 - (digits.size().x + 12.0),
        rect.center().y,
        count,
        theme::text_secondary(),
        theme::quaternary(0.7),
    );
    response.clicked()
}

fn row(
    ui: &mut Ui,
    group: Group,
    script: &LocalScript,
    selected: bool,
    show_in: &str,
    pass: &mut Pass,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, &script.name);
    let id = Id::new(("script-row", group.key(), &script.id));
    let hovered = response.hovered();
    if selected {
        ui.painter()
            .rect_filled(rect, ROW_RADIUS, theme::selected_tint(hovered));
    } else if hovered {
        ui.painter().rect_filled(rect, ROW_RADIUS, theme::ink(12));
    }
    let ink = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let icon = Rect::from_center_size(pos2(rect.min.x + 18.0, rect.center().y), Vec2::splat(ICON));
    icons::paint(ui.painter(), icon, type_glyph(script.script_type), ink);

    // The favorite star sits at the trailing edge and takes its own clicks. An unfavorited star
    // only shows while the row is under the pointer or selected.
    let star_rect =
        Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), Vec2::splat(22.0));
    let star = ui.interact(star_rect, id.with("star"), Sense::click());
    if script.is_favorite || selected || hovered || star.hovered() {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(star_rect.center(), Vec2::splat(STAR)),
            if script.is_favorite {
                Icon::StarFill
            } else {
                Icon::Star
            },
            if star.hovered() {
                theme::text_primary()
            } else {
                ink
            },
        );
    }
    let text_left = icon.max.x + 10.0;
    let name = kit::galley(
        ui,
        &script.name,
        kit::size::BODY,
        if selected {
            kit::SEMIBOLD
        } else {
            kit::REGULAR
        },
        theme::text_primary(),
        star_rect.min.x - 4.0 - text_left,
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );

    if star.clicked() {
        pass.keep_open(Cmd::ToggleFavorite(script.id.clone()));
    } else if response.clicked() {
        pass.run(Cmd::Select(script.id.clone()));
    }
    row_menu(ui, &response, script, show_in, pass);
}

/// The right-click menu of a row: what can be done to that script.
fn row_menu(
    ui: &mut Ui,
    response: &Response,
    script: &LocalScript,
    show_in: &str,
    pass: &mut Pass,
) {
    let groups = script_groups(script.script_type, script.is_favorite, show_in);
    let rows: Vec<(&str, Option<&str>)> = groups
        .iter()
        .flatten()
        .map(|row| (row.label, None))
        .collect();
    let shown = kit::fitted_context_menu(ui, response, &rows, |ui| {
        for (index, group) in groups.iter().enumerate() {
            if index > 0 {
                kit::menu_separator(ui);
            }
            for row in group {
                if kit::menu_item(ui, row.icon, row.label, true, row.destructive) {
                    let command = row.action.command(&script.id);
                    if row.action.stays_in_list() {
                        pass.keep_open(command);
                    } else {
                        pass.run(command);
                    }
                    ui.close();
                }
            }
        }
    });
    pass.menu_open |= shown.is_some();
}

/// New Script, the community link and how many scripts there are.
fn footer(ui: &mut Ui, count: usize, pass: &mut Pass) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let new = kit::push_button(ui, Some(Icon::Plus), "New Script", true)
            .on_hover_text(shortcut::NEW_SCRIPT.hint("Create a new script"));
        if new.clicked() {
            pass.run(Cmd::NewScript);
        }
        community_link(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(kit::text(
                format!("{count} {}", if count == 1 { "script" } else { "scripts" }),
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
            ui.add_space(4.0);
        });
    });
    ui.add_space(2.0);
}

/// "Community scripts" with a leading arrow glyph; opens the TradingView scripts page.
fn community_link(ui: &mut Ui) -> Response {
    let color = theme::text_secondary();
    let label = kit::galley(
        ui,
        "Community scripts",
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        color,
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(
        vec2(label.size().x + 14.0, ROW_HEIGHT - 6.0),
        Sense::click(),
    );
    let response = response.on_hover_text("Browse community scripts on TradingView");
    let color = if response.hovered() {
        theme::text_primary()
    } else {
        color
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.min.x + 5.0, rect.center().y), Vec2::splat(10.0)),
        Icon::ArrowUpRightSquare,
        color,
    );
    ui.painter().galley(
        pos2(rect.min.x + 14.0, rect.center().y - label.size().y / 2.0),
        label,
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
        let indicators: Vec<&str> = groups[1]
            .1
            .iter()
            .map(|&index| scripts[index].name.as_str())
            .collect();
        assert_eq!(indicators, ["Beta", "Alpha"]);
        assert_eq!(scripts[groups[0].1[0]].name, "Beta");
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
                .map(|index| scripts[index].name.clone())
                .collect()
        };
        assert_eq!(names("ALP"), ["Alpha"]);
        assert_eq!(names("open"), ["Beta"]);
        assert_eq!(names("  "), ["Beta", "Alpha"]);
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn the_groups_are_worked_out_again_only_when_the_search_or_the_library_changes() {
        let mut scripts = vec![
            script("a", "Alpha", ScriptType::Indicator, false, 1, "plot(close)"),
            script("b", "Beta", ScriptType::Indicator, false, 2, "plot(open)"),
        ];
        let mut picker = Picker::default();
        let first = picker.groups(&scripts);
        assert!(
            Arc::ptr_eq(&first, &picker.groups(&scripts)),
            "a frame later"
        );

        picker.search = "open".to_owned();
        let searched = picker.groups(&scripts);
        assert!(!Arc::ptr_eq(&first, &searched));
        assert_eq!(searched.len(), 1);
        assert!(Arc::ptr_eq(&searched, &picker.groups(&scripts)));

        // A script saved with other text has a newer modified time; a rename or a star counts too.
        scripts[0].source = "plot(open)".to_owned();
        scripts[0].modified_at_ms = 9;
        let saved = picker.groups(&scripts);
        assert!(!Arc::ptr_eq(&searched, &saved));
        assert_eq!(saved[0].1.len(), 2);
        scripts[1].is_favorite = true;
        assert!(!Arc::ptr_eq(&saved, &picker.groups(&scripts)));
    }
}
