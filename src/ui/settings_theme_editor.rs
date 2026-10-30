//! The theme editor of Settings ▸ Script Manager: every colour the editor paints, in groups, with
//! the real editor pinned above so each change is seen where it lands.
//!
//! Changes apply as they are made (the host publishes them and saves a moment later), so there is
//! no Apply: *Reset Changes* puts the theme back as it was when the editor was opened, *Reset Theme*
//! puts it back to the theme it started from (after asking), and *Done* just leaves. A colour's own
//! reset gives back its base theme's colour, not a fixed default.

use crate::host::editor_theme_host::{EditorThemeHost, View};
use crate::model::editor_theme::contrast::low_contrast;
use crate::model::editor_theme::{
    ColorKey, EditorTheme, Palette, Rgba, SYSTEM_FONT, SurfaceColor, SurfaceGroup, SyntaxGroup,
    SyntaxRole, ThemeId, TypographyOverride,
};
use crate::script_manager::theme_preview::ThemePreview;
use crate::ui::icons::{self, Icon};
use crate::ui::settings_script_manager::{Controls, Dialog, TypographyShown, typography_controls};
use crate::ui::settings_window::{FOOTER_HEIGHT, INSET, card, divider, footer_row};
use crate::ui::{kit, theme};
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Tab {
    Syntax,
    Editor,
    Typography,
}

/// The theme being edited, and where the editor was when it was opened.
pub(super) struct Editing {
    pub(super) id: ThemeId,
    /// The theme as it was when the editor opened: what Reset Changes goes back to.
    entry: EditorTheme,
    tab: Tab,
    query: String,
}

impl Editing {
    pub(super) fn new(theme: EditorTheme) -> Self {
        Self {
            id: theme.id,
            entry: theme,
            tab: Tab::Syntax,
            query: String::new(),
        }
    }

    /// The theme was renamed: its new name is what Reset Changes keeps, not a change to undo.
    pub(super) fn renamed(&mut self, name: &str) {
        self.entry.name = name.to_owned();
    }

    /// Whether `theme` differs from how it was opened, apart from the time it was touched.
    fn changed(&self, theme: &EditorTheme) -> bool {
        theme.overrides != self.entry.overrides
            || theme.typography != self.entry.typography
            || theme.name != self.entry.name
    }
}

fn to_color32(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

/// The keys of one group, for a reset.
fn group_keys(group: Group) -> Vec<ColorKey> {
    match group {
        Group::Syntax(group) => SyntaxRole::ALL
            .iter()
            .filter(|role| role.group() == group)
            .map(|role| ColorKey::Syntax(*role))
            .collect(),
        Group::Surface(group) => SurfaceColor::ALL
            .iter()
            .filter(|surface| surface.group() == group)
            .map(|surface| ColorKey::Surface(*surface))
            .collect(),
    }
}

#[derive(Clone, Copy)]
enum Group {
    Syntax(SyntaxGroup),
    Surface(SurfaceGroup),
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Self::Syntax(group) => group.label(),
            Self::Surface(group) => group.label(),
        }
    }
}

/// Whether a colour matches the search: its name, what it is for, or its key.
pub(super) fn matches(query: &str, key: ColorKey) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || key.label().to_lowercase().contains(&query)
        || key.hint().to_lowercase().contains(&query)
        || key.key().to_lowercase().contains(&query)
}

/// Draws the editor. Returns whether to leave it.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut Ui,
    editing: &mut Editing,
    preview: &mut ThemePreview,
    controls: &mut Controls,
    dialog: &mut Option<Dialog>,
    host: &EditorThemeHost,
    view: &View,
) -> bool {
    let Some(theme) = view
        .customs
        .iter()
        .find(|custom| custom.theme.id == editing.id)
        .map(|custom| custom.theme.clone())
    else {
        return true;
    };
    let mut done = false;
    let area = ui.max_rect();
    let bar = Rect::from_min_max(pos2(area.left(), area.bottom() - FOOTER_HEIGHT), area.max);
    ui.painter().hline(
        bar.x_range(),
        bar.top(),
        Stroke::new(1.0, theme::separator()),
    );
    let mut footer = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(INSET, 14.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let changed = editing.changed(&theme);
    let has_overrides = !theme.overrides.is_empty() || !theme.typography.is_empty();
    let mut reset_changes = false;
    let mut reset_theme = false;
    footer_row(&mut footer, None, |ui| {
        if kit::footer_button(
            ui,
            "Done",
            kit::FooterButton::Prominent(kit::ACCENT),
            true,
            84.0,
        )
        .clicked()
        {
            done = true;
        }
        if bar.width() >= COMPACT_FOOTER {
            reset_changes = kit::footer_button(
                ui,
                "Reset Changes",
                kit::FooterButton::Secondary,
                changed,
                0.0,
            )
            .on_hover_text("Put the theme back as it was when you opened it.")
            .clicked();
            reset_theme = kit::footer_button(
                ui,
                "Reset Theme…",
                kit::FooterButton::Secondary,
                has_overrides,
                0.0,
            )
            .on_hover_text("Put every color back to the theme it started from.")
            .clicked();
        } else {
            // Room for one button: the two resets share a menu.
            let more = icons::button(ui, Icon::Ellipsis, 28.0, theme::text_primary(), "Reset");
            kit::menu_popup(&more).show(|ui| {
                kit::menu_scope(ui, 0.0);
                kit::menu_fit(ui, &["Reset Changes", "Reset Theme…"]);
                if kit::menu_item(ui, Icon::Undo, "Reset Changes", changed, false) {
                    reset_changes = true;
                    ui.close();
                }
                if kit::menu_item(ui, Icon::Trash, "Reset Theme…", has_overrides, true) {
                    reset_theme = true;
                    ui.close();
                }
            });
        }
    });
    if reset_changes {
        // Only fails if the theme went away, which the next frame notices.
        let _ = host.restore(editing.entry.clone());
    }
    if reset_theme {
        *dialog = Some(Dialog::ResetAll(editing.id));
    }

    let body_rect = Rect::from_min_max(
        pos2(area.left() + INSET, area.top() + 18.0),
        pos2(area.right() - INSET, bar.top() - 10.0),
    );
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
    body.spacing_mut().item_spacing = vec2(8.0, 0.0);
    if header(&mut body, editing, &theme, dialog) {
        done = true;
    }
    body.add_space(10.0);

    // Room for it, the editor sits beside the colours; otherwise above them. Either way it stays
    // in view while the colours scroll.
    let palette = theme.palette();
    let wide = body_rect.width() >= WIDE;
    if wide {
        let gap = 16.0;
        let left_width = ((body_rect.width() - gap) * 0.5).floor();
        let top = body.cursor().top();
        let left = Rect::from_min_max(
            pos2(body_rect.left(), top),
            pos2(body_rect.left() + left_width, body_rect.bottom()),
        );
        let right = Rect::from_min_max(
            pos2(left.right() + gap, top),
            pos2(body_rect.right(), body_rect.bottom().min(top + 460.0)),
        );
        let mut preview_ui = body.new_child(egui::UiBuilder::new().max_rect(right));
        preview_frame(&mut preview_ui, preview, right.size());
        let mut column = body.new_child(egui::UiBuilder::new().max_rect(left));
        column.spacing_mut().item_spacing = vec2(8.0, 0.0);
        rows_column(&mut column, editing, controls, host, &theme, view, &palette);
    } else {
        let height = (body.available_height() * 0.32).clamp(120.0, 230.0);
        preview_frame(&mut body, preview, vec2(body_rect.width(), height));
        body.add_space(10.0);
        rows_column(&mut body, editing, controls, host, &theme, view, &palette);
    }
    done
}

/// The width from which the editor sits beside the colours instead of above them.
const WIDE: f32 = 700.0;
/// The footer width from which both reset buttons are shown rather than a menu of them.
const COMPACT_FOOTER: f32 = 500.0;
/// The width from which the tabs and the search share a row.
const ONE_ROW: f32 = 470.0;

/// The real editor, framed, in `size`.
fn preview_frame(ui: &mut Ui, preview: &mut ThemePreview, size: egui::Vec2) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, theme::separator()))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(1))
        .show(ui, |ui| {
            preview.show(ui, vec2(size.x - 2.0, size.y - 2.0));
        });
}

/// The tabs, the search and the scrolling colours under them.
fn rows_column(
    ui: &mut Ui,
    editing: &mut Editing,
    controls: &mut Controls,
    host: &EditorThemeHost,
    theme: &EditorTheme,
    view: &View,
    palette: &Palette,
) {
    let width = ui.available_width();
    let counts = [
        count(theme, |key| matches!(key, ColorKey::Syntax(_))),
        count(theme, |key| matches!(key, ColorKey::Surface(_))),
        usize::from(!theme.typography.is_empty()),
    ];
    let items = [
        kit::TabItem {
            value: Tab::Syntax,
            title: "Syntax",
            icon: Icon::Code,
            count: counts[0],
        },
        kit::TabItem {
            value: Tab::Editor,
            title: "Editor",
            icon: Icon::MacWindow,
            count: counts[1],
        },
        kit::TabItem {
            value: Tab::Typography,
            title: "Typography",
            icon: Icon::TextFormat,
            count: counts[2],
        },
    ];
    let searching = editing.tab != Tab::Typography;
    let search = |ui: &mut Ui, query: &mut String, width: f32| {
        kit::search_field(
            ui,
            Id::new("editor-theme-search"),
            query,
            "Search colors",
            width.max(80.0),
        );
    };
    if width >= ONE_ROW {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let tabs = if searching {
                (width * 0.62).floor()
            } else {
                width
            };
            ui.allocate_ui(vec2(tabs, 30.0), |ui| {
                kit::tab_bar(ui, Id::new("editor-theme-tabs"), &items, &mut editing.tab);
            });
            if searching {
                search(ui, &mut editing.query, width - tabs - 8.0);
            }
        });
    } else {
        // Too narrow for icons and a search beside them: plain titles, and the search under.
        let options = [
            (Tab::Syntax, "Syntax"),
            (Tab::Editor, "Editor"),
            (Tab::Typography, "Typography"),
        ];
        kit::segmented_filling(
            ui,
            Id::new("editor-theme-tabs"),
            &options,
            &mut editing.tab,
            width,
        );
        if searching {
            ui.add_space(8.0);
            search(ui, &mut editing.query, width);
        }
    }
    ui.add_space(10.0);
    let remaining = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt(("editor-theme-rows", editing.tab as u8))
        .max_height(remaining)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(width - 8.0);
            match editing.tab {
                Tab::Syntax => {
                    contrast_notice(ui, palette, |key| matches!(key, ColorKey::Syntax(_)));
                    let mut any = false;
                    for group in SyntaxGroup::ALL {
                        any |= group_rows(
                            ui,
                            host,
                            theme,
                            palette,
                            &editing.query,
                            Group::Syntax(group),
                        );
                    }
                    if !any {
                        no_match(ui, &editing.query);
                    }
                }
                Tab::Editor => {
                    contrast_notice(ui, palette, |key| matches!(key, ColorKey::Surface(_)));
                    let mut any = false;
                    for group in SurfaceGroup::ALL {
                        any |= group_rows(
                            ui,
                            host,
                            theme,
                            palette,
                            &editing.query,
                            Group::Surface(group),
                        );
                    }
                    if !any {
                        no_match(ui, &editing.query);
                    }
                }
                Tab::Typography => typography_tab(ui, controls, host, theme, view),
            }
            ui.add_space(8.0);
        });
}

fn count(theme: &EditorTheme, wanted: impl Fn(ColorKey) -> bool) -> usize {
    SyntaxRole::ALL
        .iter()
        .map(|role| ColorKey::Syntax(*role))
        .chain(
            SurfaceColor::ALL
                .iter()
                .map(|surface| ColorKey::Surface(*surface)),
        )
        .filter(|key| wanted(*key) && theme.is_overridden(*key))
        .count()
}

/// The back button with what the theme is based on, and the theme's name with a rename button.
fn header(
    ui: &mut Ui,
    editing: &Editing,
    theme: &EditorTheme,
    dialog: &mut Option<Dialog>,
) -> bool {
    let mut back = false;
    let changes = theme.override_count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, response) = ui.allocate_exact_size(vec2(84.0, 24.0), Sense::click());
        let response = crate::ui::accessibility::label(response, "Back to the themes")
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        // Quiet, like the rest of the page's chrome: neutral, a little stronger under the pointer.
        let ink = if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, theme::ink(14));
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(10.0, 12.0)),
            Icon::ChevronLeft,
            ink,
        );
        let text = kit::galley(ui, "Themes", kit::size::BODY, kit::REGULAR, ink, 60.0);
        ui.painter().galley(
            pos2(rect.left() + 24.0, rect.center().y - text.size().y / 2.0),
            text,
            Color32::PLACEHOLDER,
        );
        back = response.clicked();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(kit::text(
                format!(
                    "Based on {} · {}",
                    theme.base_theme().name(),
                    match changes {
                        0 => "no colors changed".to_owned(),
                        1 => "1 color changed".to_owned(),
                        count => format!("{count} colors changed"),
                    }
                ),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let width = ui.available_width() - 34.0;
        let title = kit::galley(
            ui,
            &format!("Edit Theme: {}", theme.name),
            kit::size::TITLE2 + 1.0,
            kit::SEMIBOLD,
            theme::text_primary(),
            width,
        );
        let (rect, _) = ui.allocate_exact_size(
            vec2(title.size().x.min(width), title.size().y),
            Sense::hover(),
        );
        ui.painter().galley(rect.min, title, Color32::PLACEHOLDER);
        if icons::button(
            ui,
            Icon::Pencil,
            26.0,
            theme::text_secondary(),
            "Rename this theme",
        )
        .clicked()
        {
            *dialog = Some(Dialog::Rename {
                id: editing.id,
                name: theme.name.clone(),
                focus: true,
                error: None,
            });
        }
    });
    back
}

/// A quiet warning, never a block, when colours sit close to the background they are drawn on.
fn contrast_notice(ui: &mut Ui, palette: &Palette, within: impl Fn(ColorKey) -> bool) {
    let low: Vec<&str> = low_contrast(palette)
        .into_iter()
        .filter(|key| within(*key))
        .map(ColorKey::label)
        .collect();
    if low.is_empty() {
        return;
    }
    let names = match low.as_slice() {
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more", rest.len()),
        [] => String::new(),
    };
    kit::notice_card(
        ui,
        Icon::Triangle,
        kit::ORANGE,
        "Some colors are hard to read",
        Some(&format!("Close to the color behind them: {names}.")),
        None,
    );
    ui.add_space(12.0);
}

fn no_match(ui: &mut Ui, query: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(kit::text(
            format!("No colors match “{}”.", query.trim()),
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
}

/// One group: its heading, a menu to reset it, and a card of its colours that match the search.
/// Returns whether anything was shown.
fn group_rows(
    ui: &mut Ui,
    host: &EditorThemeHost,
    theme: &EditorTheme,
    palette: &Palette,
    query: &str,
    group: Group,
) -> bool {
    let keys: Vec<ColorKey> = group_keys(group)
        .into_iter()
        .filter(|key| matches(query, *key))
        .collect();
    if keys.is_empty() {
        return false;
    }
    let changed: Vec<ColorKey> = group_keys(group)
        .into_iter()
        .filter(|key| theme.is_overridden(*key))
        .collect();
    ui.horizontal(|ui| {
        ui.label(kit::text(
            group.label().to_uppercase(),
            kit::size::CAPTION,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = icons::button(
                ui,
                Icon::Ellipsis,
                22.0,
                theme::text_secondary(),
                "Reset this group",
            );
            kit::menu_popup(&more).show(|ui| {
                let label = match changed.len() {
                    0 => "Nothing to reset".to_owned(),
                    1 => "Reset 1 color".to_owned(),
                    count => format!("Reset {count} colors"),
                };
                // As wide as its one row needs, not a guessed width.
                kit::menu_scope(ui, 0.0);
                kit::menu_fit(ui, &[&label]);
                if kit::menu_item(ui, Icon::Undo, &label, !changed.is_empty(), false) {
                    let _ = host.reset_colors(theme.id, &changed);
                    ui.close();
                }
            });
        });
    });
    ui.add_space(4.0);
    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                divider(ui);
            }
            color_row(ui, host, theme, palette, *key);
        }
    });
    ui.add_space(16.0);
    true
}

/// One colour: its swatch (which opens the colour picker), its name, its value, and a reset that
/// is there only while it differs from the base theme's.
fn color_row(
    ui: &mut Ui,
    host: &EditorThemeHost,
    theme: &EditorTheme,
    palette: &Palette,
    key: ColorKey,
) {
    let color = palette.get(key);
    let overridden = theme.is_overridden(key);
    let translucent = matches!(key, ColorKey::Surface(surface) if surface.translucent());
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        let alpha = if translucent {
            egui::color_picker::Alpha::OnlyBlend
        } else {
            egui::color_picker::Alpha::Opaque
        };
        if let Some(picked) = kit::color_well(ui, vec2(34.0, 22.0), key.label(), to_color32(color), alpha) {
            let [r, g, b, a] = picked.to_srgba_unmultiplied();
            let _ = host.set_color(theme.id, key, Rgba::new(r, g, b, if translucent { a } else { 255 }));
        }
        let label = ui.label(kit::text(
            key.label(),
            kit::size::BODY,
            if overridden { kit::MEDIUM } else { kit::REGULAR },
            theme::text_primary(),
        ));
        label.on_hover_text(key.hint());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if overridden {
                if icons::button(
                    ui,
                    Icon::Undo,
                    22.0,
                    theme::text_secondary(),
                    &format!("Reset to {}'s {}", theme.base_theme().name(), color_hex(theme.base_theme().palette().get(key))),
                )
                .clicked()
                {
                    let _ = host.reset_colors(theme.id, &[key]);
                }
            } else {
                ui.allocate_space(vec2(22.0, 22.0));
            }
            ui.label(
                kit::text(color_hex(color), kit::size::CAPTION + 0.5, kit::REGULAR, theme::text_secondary())
                    .monospace(),
            )
            .on_hover_text("Click the swatch to change it. The picker takes #RRGGBB, #RRGGBBAA, RGB values and system colors.");
        });
    });
}

fn color_hex(color: Rgba) -> String {
    color.to_hex()
}

fn typography_tab(
    ui: &mut Ui,
    controls: &mut Controls,
    host: &EditorThemeHost,
    theme: &EditorTheme,
    view: &View,
) {
    ui.add(
        egui::Label::new(kit::text(
            "Fonts are part of how the editor looks, so a theme can bring its own. Anything it does not set uses your own editor setting.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.add_space(12.0);
    let own = &theme.typography;
    let shown = TypographyShown {
        // A theme that names the system font shows it as System Mono (the picker's `None`).
        family: match own.font_family.as_deref() {
            Some(SYSTEM_FONT) => None,
            Some(name) => Some(name),
            None => view.typography.font_family.as_deref(),
        },
        size: own.font_size.unwrap_or(view.typography.font_size),
        line_height: own.line_height.unwrap_or(view.typography.line_height),
        resettable: [
            own.font_family.is_some(),
            own.font_size.is_some(),
            own.line_height.is_some(),
        ],
        status: &view.font,
        fonts: &view.fonts,
        fonts_ready: view.fonts_ready,
    };
    let change = typography_controls(ui, controls, "theme", &shown);
    let mut next: TypographyOverride = own.clone();
    let mut touched = false;
    if let Some(family) = change.family {
        // A font picked here is the theme's own, and so is the system's: it is named, not left
        // unset (the reset beside the row is what unsets it).
        next.font_family = Some(family.unwrap_or_else(|| SYSTEM_FONT.to_owned()));
        touched = true;
    }
    if let Some(size) = change.size {
        next.font_size = Some(size);
        touched = true;
    }
    if let Some(height) = change.line_height {
        next.line_height = Some(height);
        touched = true;
    }
    for (index, reset) in change.reset.iter().enumerate() {
        if *reset {
            match index {
                0 => next.font_family = None,
                1 => next.font_size = None,
                _ => next.line_height = None,
            }
            touched = true;
        }
    }
    if touched {
        let _ = host.set_theme_typography(theme.id, next);
    }
}

#[cfg(test)]
mod tests;
