//! The settings of one compared market in a popover pointing at its row on the chart's overlay
//! card, the way an applied script's settings open from its chip. The body is the same one the
//! Compare page of the chart settings draws ([`overlay_settings::body`]).
//!
//! Edits are live, one undo step each, like the Compare page and unlike the script popover: an
//! overlay's look is cheap to redraw and the chart is the preview. Closing it (Esc, a press
//! outside) keeps what was changed.

use super::icons::Icon;
use super::kit::{self, SEMIBOLD};
use super::overlay_settings;
use super::popover::{self, Placement};
use super::theme;
use crate::model::overlay::{ChartOverlay, OverlayCommand, OverlayId, OverlayScale};
use crate::model::price_scale::PriceScaleMode;
use eframe::egui::{Align, Context, Id, Layout, Rect, Sense, vec2};

/// The name column, the controls and the panel's padding.
const WIDTH: f32 = 312.0;
const PADDING: i8 = 16;

/// What the popover wants after a frame.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Open,
    Close,
    /// An edit to make now; the popover stays open.
    Command(OverlayCommand),
}

/// Which market's settings are open, and the row they point at.
pub struct OverlaySettingsPopover {
    chart_id: u64,
    overlay_id: OverlayId,
    anchor: Rect,
}

impl OverlaySettingsPopover {
    pub fn open(chart_id: u64, overlay_id: OverlayId, anchor: Rect) -> Self {
        Self {
            chart_id,
            overlay_id,
            anchor,
        }
    }

    pub fn is_for(&self, chart_id: u64, overlay_id: OverlayId) -> bool {
        self.chart_id == chart_id && self.overlay_id == overlay_id
    }

    pub fn chart_id(&self) -> u64 {
        self.chart_id
    }

    pub fn overlay_id(&self) -> OverlayId {
        self.overlay_id
    }

    /// Draws the panel for `overlay`, as the chart has it now. `own_scales` are the markets with a
    /// scale of their own on the chart and `mode` is the chart's axis.
    pub fn show(
        &self,
        ctx: &Context,
        overlay: &ChartOverlay,
        own_scales: &[(OverlayId, String)],
        mode: PriceScaleMode,
    ) -> Outcome {
        let id = Id::new(("overlay-settings-popover", self.chart_id, self.overlay_id));
        let mut outcome = Outcome::Open;
        let dismissed = popover::show_placed(
            ctx,
            id,
            self.anchor,
            Placement::Start,
            WIDTH,
            PADDING,
            true,
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // The market, then where it trades, small and quiet beside it.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // The market's logo, as the chart's tab shows it.
                    let (logo, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    kit::chart_icon(
                        ui,
                        logo,
                        &overlay.instrument.symbol,
                        overlay.instrument.source,
                        false,
                    );
                    ui.label(kit::text(
                        overlay.name(),
                        kit::size::BODY + 1.0,
                        SEMIBOLD,
                        theme::text_primary(),
                    ));
                    ui.label(kit::text(
                        overlay.instrument.source.display_name(),
                        kit::size::CAPTION + 1.0,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                });
                ui.add_space(14.0);
                if let Some(command) = overlay_settings::body(
                    ui,
                    overlay,
                    own_scales,
                    mode,
                    overlay_settings::Density::Compact,
                ) {
                    outcome = Outcome::Command(command);
                }
                ui.add_space(14.0);
                // Hide and Remove together at the trailing edge, the destructive one last.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if kit::destructive_sheet_button(ui, Icon::Trash, "Remove").clicked() {
                        outcome = Outcome::Command(OverlayCommand::Remove(overlay.id));
                    }
                    if kit::sheet_button(
                        ui,
                        if overlay.visible { "Hide" } else { "Show" },
                        false,
                        true,
                    )
                    .clicked()
                    {
                        outcome = Outcome::Command(OverlayCommand::SetVisible(
                            overlay.id,
                            !overlay.visible,
                        ));
                    }
                });
            },
        );
        match outcome {
            Outcome::Open if dismissed => Outcome::Close,
            other => other,
        }
    }
}

fn open_id() -> Id {
    Id::new("open-overlay-settings-popover")
}

/// Which market's popover is up, as (chart id, overlay id), so the card keeps its row showing
/// meanwhile. Written by the workspace each frame, read by the chart card.
pub fn set_open(ctx: &Context, open: Option<(u64, OverlayId)>) {
    ctx.data_mut(|data| data.insert_temp(open_id(), open));
}

pub fn open(ctx: &Context) -> Option<(u64, OverlayId)> {
    ctx.data(|data| data.get_temp::<Option<(u64, OverlayId)>>(open_id()))
        .flatten()
}

/// The markets that have a scale of their own, for the "Scale of X" choices.
pub fn own_scales<'a>(
    overlays: impl IntoIterator<Item = &'a ChartOverlay>,
) -> Vec<(OverlayId, String)> {
    overlays
        .into_iter()
        .filter_map(|overlay| match overlay.scale {
            OverlayScale::Own(group) if group == overlay.id => Some((overlay.id, overlay.name())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::overlay::OverlayList;
    use crate::model::{DataSource, InstrumentId};
    use eframe::egui::{self, pos2, vec2};

    fn list() -> OverlayList {
        let main = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let mut list = OverlayList::new();
        for symbol in ["ETHUSDT", "SOLUSDT"] {
            list.add(&main, InstrumentId::new(DataSource::Binance, symbol), None);
        }
        list
    }

    fn draw(popover: &OverlaySettingsPopover, list: &OverlayList) -> Outcome {
        let ctx = Context::default();
        let overlay = list.get(popover.overlay_id()).unwrap();
        let own = own_scales(list.iter());
        let mut outcome = Outcome::Open;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                outcome = popover.show(ui.ctx(), overlay, &own, PriceScaleMode::Percent);
            });
            output.textures_delta.clear();
        }
        outcome
    }

    #[test]
    fn it_stays_open_and_is_for_its_market() {
        let list = list();
        let id = list.iter().next().unwrap().id;
        let popover = OverlaySettingsPopover::open(
            7,
            id,
            Rect::from_min_size(pos2(40.0, 40.0), vec2(120.0, 18.0)),
        );
        assert!(popover.is_for(7, id));
        assert!(!popover.is_for(8, id));
        assert_eq!(draw(&popover, &list), Outcome::Open);
    }

    /// A picture of the popover, to look at: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib
    /// gallery_of_the_overlay_popover -- --ignored`.
    #[test]
    #[ignore = "writes a picture when DEGENVIEW_GALLERY_DIR names a folder"]
    fn gallery_of_the_overlay_popover() {
        use crate::ui::render_tests::Screen;
        let list = list();
        let id = list.iter().next().unwrap().id;
        let overlay = list.get(id).unwrap();
        let own = own_scales(list.iter());
        let mut screen = Screen::new(vec2(640.0, 460.0));
        let popover = OverlaySettingsPopover::open(
            1,
            id,
            Rect::from_min_size(pos2(40.0, 24.0), vec2(220.0, 16.0)),
        );
        let mut last = None;
        for _ in 0..4 {
            last = Some(screen.run(Vec::new(), |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, crate::ui::theme::chrome());
                popover.show(ui.ctx(), overlay, &own, PriceScaleMode::Percent);
            }));
        }
        screen.save("overlay-popover", last.unwrap());
    }

    #[test]
    fn own_scales_lists_only_the_markets_that_own_one() {
        let mut list = list();
        let (first, second) = {
            let mut ids = list.iter().map(|overlay| overlay.id);
            (ids.next().unwrap(), ids.next().unwrap())
        };
        assert!(own_scales(list.iter()).is_empty());
        list.apply(OverlayCommand::SetScale(
            first,
            crate::model::overlay::ScaleChoice::Own,
        ));
        list.apply(OverlayCommand::SetScale(
            second,
            crate::model::overlay::ScaleChoice::SharedWith(first),
        ));
        let owners = own_scales(list.iter());
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].0, first);
    }
}
