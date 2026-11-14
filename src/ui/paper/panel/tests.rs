use super::*;
use crate::model::paper::{PaperAccountSettings, PaperTradingError};
use crate::store::{SharedStore, Store};
use std::sync::{Arc, Mutex};

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
}

fn account(host: &mut PaperHost) -> u64 {
    host.create_account(
        "Main",
        PaperCurrency::Usd,
        100_000.0,
        PaperAccountSettings::default(),
    )
    .expect("account")
}

#[test]
fn a_bulk_action_reports_what_it_could_not_do() {
    let ok: Result<u64, PaperHostError> = Ok(1);
    let failed: Result<u64, PaperHostError> =
        Err(PaperHostError::Engine(PaperTradingError::NoMarketData));
    assert_eq!(
        bulk_failure_text("Closed", "positions", &[ok.clone(), ok.clone()]),
        None
    );
    let text = bulk_failure_text(
        "Closed",
        "positions",
        &[ok.clone(), failed.clone(), failed.clone()],
    )
    .unwrap();
    assert!(text.starts_with("Closed 1 of 3 positions."), "{text}");
    assert!(text.ends_with("(and 1 more)"), "{text}");
}

#[test]
fn exporting_with_nothing_to_export_says_so_and_opens_no_dialog() {
    let mut host = PaperHost::load(store());
    let ctx = egui::Context::default();
    let mut panel = PaperPanel::default();

    panel.begin_export(&ctx, &host, None);
    assert!(
        panel
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("Create a paper account"))
    );

    let id = account(&mut host);
    panel.notice = None;
    panel.begin_export(&ctx, &host, Some(id));
    let notice = panel.notice.as_ref().expect("the user is told");
    assert!(
        notice.text.starts_with("Nothing to export yet"),
        "{}",
        notice.text
    );
    assert!(!notice.is_error);
    assert!(
        panel.export_job.is_none(),
        "no folder dialog for an empty account"
    );
}

#[test]
fn an_info_notice_fades_but_an_error_stays_until_dismissed() {
    let mut notice = PanelNotice::info("done");
    assert!(!notice.expired());
    notice.since -= PanelNotice::LINGER;
    assert!(notice.expired());
    let mut error = PanelNotice::error("disk full");
    error.since -= PanelNotice::LINGER * 10;
    assert!(!error.expired());
}

/// The notice line, as pictures: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib ui::paper::panel::tests::gallery -- --ignored`.
#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_notice() {
    use crate::ui::render_tests::Screen;
    let _restore = crate::ui::theme::RestoreDefaultTheme;
    let mut host = PaperHost::load(store());
    account(&mut host);
    for (name, notice) in [
        (
            "empty",
            PanelNotice::info(
                "Nothing to export yet. Orders, fills and closed trades show up here once you trade.",
            ),
        ),
        (
            "done",
            PanelNotice::info(
                "Exported 3 orders, 2 fills and 1 closed trade to \u{201c}Documents\u{201d}.",
            ),
        ),
        (
            "error",
            PanelNotice::error(
                "Could not write /Volumes/x/paper-orders.csv: Read-only file system",
            ),
        ),
    ] {
        for light in [false, true] {
            let mut screen = Screen::new(egui::vec2(900.0, 260.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut panel = PaperPanel {
                notice: Some(PanelNotice {
                    text: notice.text.clone(),
                    ..notice
                }),
                ..PaperPanel::default()
            };
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    crate::ui::theme::sync(ui.ctx());
                    egui::Panel::bottom(egui::Id::new("g"))
                        .exact_size(260.0)
                        .frame(egui::Frame::new().fill(crate::ui::kit::window()))
                        .show(ui, |ui| {
                            panel.ui(
                                ui,
                                &mut host,
                                &crate::ui::trade::panel_header::sample_bar(
                                    crate::model::broker::venue::TradingVenue::Paper,
                                    crate::trading::model::VenueKind::Simulated,
                                    "Paper",
                                ),
                            );
                        });
                }));
            }
            screen.save(
                &format!("notice-{name}-{}", if light { "light" } else { "dark" }),
                last.expect("ran"),
            );
        }
    }
}
