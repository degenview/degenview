//! The account's numbers across the top of the orders panel: one big figure and a row of quieter
//! ones, drawn from [`AccountStats`] so every venue's strip looks the same. The row scrolls
//! sideways instead of clipping when the window is narrow.

use eframe::egui::{self, Color32};

use crate::trading::model::{AccountStats, Hero, HeroNote, StatTone, Trend};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};

/// Height of the strip: the hero's three lines between 9 pt of padding.
pub const STRIP_HEIGHT: f32 = 62.0;
/// What the hero's three lines and a cell's two take, for centring them in the strip.
const HERO_HEIGHT: f32 = 46.0;
const CELL_HEIGHT: f32 = 31.0;

/// The colour of a figure: `quiet` is what a neutral one is drawn in.
pub fn tone_color(tone: StatTone, quiet: Color32) -> Color32 {
    match tone {
        StatTone::Neutral => quiet,
        StatTone::Profit => theme::UP,
        StatTone::Loss | StatTone::Critical => theme::DOWN,
        StatTone::Thin => theme::REPLAY,
    }
}

pub fn show(ui: &mut egui::Ui, stats: Option<&AccountStats>) {
    let secondary = theme::text_secondary();
    egui::ScrollArea::horizontal()
        .id_salt("trade-stats")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            let row = egui::vec2(ui.available_width(), STRIP_HEIGHT);
            ui.allocate_ui_with_layout(
                row,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    ui.add_space(14.0);
                    hero(ui, stats.map(|stats| &stats.hero));
                    for cell in stats.iter().flat_map(|stats| stats.cells.iter()) {
                        ui.add_space(14.0);
                        let (line, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 28.0), egui::Sense::hover());
                        ui.painter().rect_filled(line, 0.0, theme::separator());
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add_space((STRIP_HEIGHT - CELL_HEIGHT) / 2.0);
                            ui.label(kit::text(&cell.title, 10.0, kit::REGULAR, secondary));
                            let color = if cell.value == "—" {
                                secondary
                            } else {
                                tone_color(cell.tone, theme::text_primary())
                            };
                            ui.add(
                                egui::Label::new(kit::text(&cell.value, 12.5, kit::MEDIUM, color))
                                    .extend(),
                            )
                            .on_hover_text(&cell.help);
                        });
                    }
                    ui.add_space(14.0);
                },
            );
        });
}

/// The big figure, with its note underneath.
fn hero(ui: &mut egui::Ui, hero: Option<&Hero>) {
    let secondary = theme::text_secondary();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space((STRIP_HEIGHT - HERO_HEIGHT) / 2.0);
        let title = hero.map_or("Equity", |hero| hero.title.as_str());
        ui.label(kit::text(title, 10.5, kit::REGULAR, secondary));
        let (value, color) = match hero {
            Some(hero) => (hero.value.as_str(), theme::text_primary()),
            None => ("—", secondary),
        };
        let figure = ui.label(kit::text(value, 16.0, kit::SEMIBOLD, color));
        if let Some(help) = hero.and_then(|hero| hero.help.as_deref()) {
            figure.on_hover_text(help);
        }
        // `horizontal` is 18 pt tall unless told otherwise; this line is a 10.5 pt caption.
        ui.spacing_mut().interact_size.y = 13.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            match hero.and_then(|hero| hero.note.as_ref()) {
                Some(note) => note_line(ui, note),
                None => {
                    ui.label(kit::text(" ", 10.5, kit::REGULAR, secondary));
                }
            }
        });
    });
}

fn note_line(ui: &mut egui::Ui, note: &HeroNote) {
    let secondary = theme::text_secondary();
    let tint = tone_color(note.tone, secondary);
    let arrow = match note.trend {
        Trend::Up => Some(Icon::ArrowUpRight),
        Trend::Down => Some(Icon::ArrowDownRight),
        // A flat figure gets a dash only when it is a gain or a loss that is flat; a count has none.
        Trend::Flat if !note.label.is_empty() => Some(Icon::Minus),
        Trend::Flat => None,
    };
    if let Some(arrow) = arrow {
        icons::inline(ui, arrow, 10.0, tint);
    }
    ui.label(kit::text(&note.text, 10.5, kit::MEDIUM, tint));
    if !note.label.is_empty() {
        ui.label(kit::text(&note.label, 10.5, kit::REGULAR, secondary));
    }
}
