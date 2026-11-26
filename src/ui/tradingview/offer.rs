//! The offer to import, made once at launch: what was found, and the way in.

use super::words;
use crate::i18n::{tr, tr_n};
use crate::model::tradingview::library::Contents;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::dialog::{self, Dialog, Outcome};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Sense, Ui, Vec2};

const ICON: f32 = 16.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Choice {
    Import,
    Dismiss,
}

/// A kind of thing there is to bring over.
struct Row {
    icon: Icon,
    count: String,
    detail: String,
}

fn rows(found: &Contents) -> Vec<Row> {
    let drawings = found.drawings;
    let kinds = [
        (
            found.layouts,
            Icon::BarChart,
            words::layouts(found.layouts),
            if drawings == 0 {
                tr!("Charts and indicators").to_owned()
            } else {
                tr_n!(
                    drawings,
                    "Charts, indicators and {count} drawing",
                    "Charts, indicators and {count} drawings"
                )
            },
        ),
        (
            found.watchlists,
            Icon::List,
            words::watchlists(found.watchlists),
            tr!("Markets and sections").to_owned(),
        ),
        (
            found.scripts,
            Icon::Code,
            words::scripts(found.scripts),
            tr!("Pine code you wrote or saved").to_owned(),
        ),
    ];
    kinds
        .into_iter()
        .filter(|(count, ..)| *count > 0)
        .map(|(_, icon, count, detail)| Row {
            icon,
            count,
            detail,
        })
        .collect()
}

pub fn show(ctx: &egui::Context, found: &Contents) -> Option<Choice> {
    let mut spec = Dialog::new(
        Icon::Download,
        theme::text_secondary(),
        tr!("Import from TradingView?"),
        tr!("The TradingView app on this computer has things you can bring over:"),
        tr!("Choose What to Import"),
    );
    spec.dismiss = tr!("No Thanks");
    // It appears unasked, so a key pressed for something else must not answer it.
    spec.cautious = true;
    dialog::show(ctx, Id::new("tradingview-offer"), &spec, |ui| {
        body(ui, found);
    })
    .map(|outcome| match outcome {
        Outcome::Accepted => Choice::Import,
        Outcome::Dismissed | Outcome::Leading => Choice::Dismiss,
    })
}

fn body(ui: &mut Ui, found: &Contents) {
    ui.add_space(14.0);
    for row in rows(found) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (mark, _) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::hover());
            icons::paint(ui.painter(), mark, row.icon, theme::text_secondary());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                line(ui, &row.count, size::BODY, theme::text_primary());
                line(ui, &row.detail, size::CAPTION, theme::text_secondary());
            });
        });
        ui.add_space(10.0);
    }
    ui.add_space(4.0);
    line(
        ui,
        tr!("You choose what comes over and see a preview first. Nothing in TradingView changes."),
        size::CAPTION,
        theme::text_secondary(),
    );
    ui.add_space(6.0);
    line(
        ui,
        tr!("This is only asked once. Later, choose Import from TradingView in the layout menu."),
        size::CAPTION,
        theme::text_secondary(),
    );
}

fn line(ui: &mut Ui, text: &str, size: f32, color: egui::Color32) {
    ui.add(egui::Label::new(kit::text(text, size, REGULAR, color)).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_was_found_is_listed() {
        let found = Contents {
            layouts: 1,
            drawings: 0,
            watchlists: 0,
            scripts: 2,
        };
        let listed: Vec<_> = rows(&found)
            .into_iter()
            .map(|row| (row.count, row.detail))
            .collect();
        assert_eq!(
            listed,
            [
                ("1 layout".into(), "Charts and indicators".into()),
                ("2 scripts".into(), "Pine code you wrote or saved".into()),
            ]
        );
    }
}
