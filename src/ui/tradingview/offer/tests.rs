use super::*;
use crate::ui::render_tests::Screen;
use crate::ui::theme::AppTheme;

fn names(count: usize) -> Vec<String> {
    [
        "Scalping desk",
        "Macro weekly with a very long name that cannot fit",
        "Alts",
        "Majors",
        "Swing",
    ]
    .iter()
    .take(count)
    .map(|name| (*name).to_owned())
    .collect()
}

fn found(layouts: usize, watchlists: usize, scripts: usize) -> Contents {
    Contents {
        layouts: names(layouts),
        drawings: 34,
        watchlists: names(watchlists),
        markets: 128,
        scripts: names(scripts),
        lines: 212,
    }
}

#[test]
fn only_what_was_found_gets_a_card() {
    let some = found(1, 0, 2);
    let titles: Vec<_> = cards(&some)
        .into_iter()
        .map(|card| (card.title, card.detail))
        .collect();
    assert_eq!(
        titles,
        [
            ("1 layout".to_owned(), "34 drawings".to_owned()),
            ("2 scripts".to_owned(), "212 lines".to_owned()),
        ]
    );
}

#[test]
fn a_card_lists_a_few_names_and_counts_the_rest() {
    let three = names(3);
    assert_eq!(listed(&three), (&three[..], None));
    let five = names(5);
    assert_eq!(listed(&five), (&five[..2], Some("and 3 more".to_owned())));
}

/// Pictures for a person to look at:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_offer -- --ignored`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_offer() {
    let _theme = theme::RestoreDefaultTheme;
    for (name, contents) in [("full", found(5, 2, 1)), ("one", found(1, 0, 0))] {
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            // A moment with a parcel on its way and the chart half built.
            for (moment, time) in [("early", 1.1), ("late", 4.2)] {
                let mut screen = Screen::new(vec2(WIDTH, 600.0));
                egui_extras::install_image_loaders(&screen.ctx);
                theme::apply(&screen.ctx, app_theme);
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                        body(ui, &contents, time, false, &mut None);
                    }));
                }
                screen.save(
                    &format!("offer-{name}-{moment}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}
