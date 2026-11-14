//! The chart settings sheet: Indicators, Scripts, Appearance and Canvas above "Remove Chart",
//! the Templates menu and "Done". Every control writes through to the chart as it changes.

mod appearance;
pub(in crate::ui) mod canvas;
pub(in crate::ui) mod compare;
mod indicators;
pub(in crate::ui) mod overlay;
pub mod overlay_popover;
mod scripts;
pub(in crate::ui) mod style;
pub(in crate::ui) mod style_preview;
pub mod templates;

use super::card::ChartCard;
use super::settings::compare::CompareInput;
use super::settings::templates::{TemplateAction, TemplateState};
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::{DataSource, IndicatorConfig};
use crate::store::SharedStore;
use crate::ui::icons::Icon;
use crate::ui::kit::sheet;
use crate::ui::kit::{self, TabItem};
use crate::ui::theme;
use appearance::appearance_page;
use eframe::egui::{self, Align, Id, Layout, Margin, Rect, Sense, Ui, Vec2, vec2};
use indicators::indicators_page;
use scripts::scripts_page;

/// Swift's ideal sheet width.
const WIDTH: f32 = 760.0;
/// The scrolling page's height in a tall window; enough to show most Appearance and Canvas
/// sections at once, while still leaving room around the sheet.
const MAX_PAGE_HEIGHT: f32 = 650.0;
const MIN_PAGE_HEIGHT: f32 = 260.0;
/// What the sheet spends on everything but the page: header, tab bar, two rules and the footer.
const CHROME_HEIGHT: f32 = 250.0;

fn page_height(parent_height: f32) -> f32 {
    (parent_height - CHROME_HEIGHT).clamp(MIN_PAGE_HEIGHT, MAX_PAGE_HEIGHT)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Indicators,
    Scripts,
    Appearance,
    Canvas,
    /// The markets compared on the chart.
    Overlays,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Indicators,
        Self::Scripts,
        Self::Overlays,
        Self::Appearance,
        Self::Canvas,
    ];

    /// The pages a chart from `source` offers: CoinMarketCap's indices take no indicators or
    /// scripts.
    fn for_source(source: DataSource) -> &'static [Self] {
        if source == DataSource::CoinMarketCap {
            &[Self::Appearance, Self::Canvas]
        } else {
            &Self::ALL
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Indicators => "Indicators",
            Self::Scripts => "Scripts",
            Self::Appearance => "Appearance",
            Self::Canvas => "Canvas",
            Self::Overlays => "Compare",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Indicators => Icon::Waveform,
            Self::Scripts => Icon::AngleBrackets,
            Self::Appearance => Icon::Palette,
            Self::Canvas => Icon::MacWindow,
            Self::Overlays => Icon::Compare,
        }
    }
}

#[derive(Default)]
pub struct ChartSettingsState {
    pub tab: Tab,
    /// What the Scripts page's search field holds.
    script_query: String,
    templates: TemplateState,
    /// The Compare page's open cards, search and pending confirmation.
    compare: compare::CompareState,
}

impl ChartSettingsState {
    /// Says what an action on the whole workspace did, in the sheet's footer.
    pub fn say(&mut self, text: impl Into<String>) {
        self.templates.say(text);
    }
}

/// What the sheet reads and edits for one chart.
pub struct ChartSettingsContext<'a> {
    pub title: &'a str,
    pub symbol: &'a str,
    pub source: DataSource,
    /// How many candles the chart has loaded, for the RSI warm-up note.
    pub loaded_candles: usize,
    pub indicators: &'a mut IndicatorConfig,
    pub card: &'a mut ChartCard,
    /// The chart's price-scale mode, which is not part of its appearance.
    pub scale_mode: PriceScaleMode,
    /// Where chart templates are kept.
    pub store: &'a SharedStore,
}

/// What the sheet asks the workspace to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartSettingsAction {
    Done,
    RemoveChart,
    OpenPineStudio,
    /// Open (or put away) the Inputs and Style popover of an applied script, pointing at `Rect`.
    ConfigureScript(u64, Rect),
    /// Give every chart of the workspace this look.
    ApplyToAll(crate::model::chart::template::ApplyScope),
    /// Read the price axis in this mode.
    SetScaleMode(PriceScaleMode),
    /// Change a compared market.
    Overlay(crate::model::overlay::OverlayCommand),
    /// Pick another market to compare, in the symbol picker.
    AddOverlay,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut ChartSettingsState,
    mut c: ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let was_templating = state.templates.has_dialog();
    let page_height = page_height(ctx.content_rect().height());
    let tabs = Tab::for_source(c.source);
    if !tabs.contains(&state.tab) {
        state.tab = tabs[0];
    }
    let ((), clicked_outside) =
        sheet::show_dismissable(ctx, Id::new("chart-settings"), WIDTH, |ui| {
            ui.add_space(24.0);
            sheet::inset(ui, |ui| header(ui, &c));
            ui.add_space(16.0);
            if tabs.len() > 1 {
                tab_bar(ui, state, tabs);
                ui.add_space(16.0);
            }
            rule(ui);
            egui::ScrollArea::vertical()
                .id_salt(("chart-settings-page", state.tab as u8))
                .max_height(page_height)
                .min_scrolled_height(page_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::same(sheet::INSET))
                        .show(ui, |ui| {
                            ui.set_width(WIDTH - 2.0 * f32::from(sheet::INSET));
                            match state.tab {
                                Tab::Indicators => indicators_page(ui, &mut c),
                                Tab::Scripts => {
                                    if let Some(next) = scripts_page(ui, state, &mut c) {
                                        action = Some(next);
                                    }
                                }
                                Tab::Appearance => {
                                    if let Some(next) =
                                        appearance_page(ui, &mut c, &mut state.templates)
                                    {
                                        action = Some(next);
                                    }
                                }
                                Tab::Overlays => {
                                    let input = CompareInput {
                                        card: &*c.card,
                                        chart_name: c.title,
                                        scale_mode: c.scale_mode,
                                        price_based: c
                                            .card
                                            .effective_chart_type(c.source)
                                            .is_synthetic(),
                                    };
                                    if let Some(next) =
                                        compare::page(ui, &mut state.compare, &input)
                                    {
                                        action = Some(next);
                                    }
                                }
                                Tab::Canvas => {
                                    let (store, templates) = (c.store, &mut state.templates);
                                    super::settings::canvas::show(
                                        ui,
                                        c.card,
                                        Id::new(("chart-canvas", c.symbol)),
                                        |ui, card| templates::menu(ui, templates, store, card),
                                    );
                                }
                            }
                        });
                });
            rule(ui);
            sheet::inset(ui, |ui| {
                if let Some(next) = footer(ui, state) {
                    action = Some(next);
                }
            });
        });
    // A click on the dimmed area around the sheet puts it away, like Done.
    if action.is_none() && clicked_outside {
        action = Some(ChartSettingsAction::Done);
    }
    // A template dialog takes Escape first; a second press closes the sheet.
    if let Some(TemplateAction::ApplyToAll(scope)) =
        templates::dialogs(ctx, &mut state.templates, c.store, c.card)
    {
        action = Some(ChartSettingsAction::ApplyToAll(scope));
    }
    if action.is_none()
        && !state.templates.has_dialog()
        && !was_templating
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        action = Some(ChartSettingsAction::Done);
    }
    action
}

/// A hairline across the sheet, between the header, the page and the footer.
fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::separator());
}

fn header(ui: &mut Ui, c: &ChartSettingsContext) {
    let subtitle = format!(
        "{} · Changes apply as you make them",
        c.source.display_name()
    );
    sheet::header_with(ui, c.title, &subtitle, |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        kit::chart_icon(ui, rect.shrink(1.0), c.symbol, c.source, true);
    });
}

/// The icon tab bar, centred under the header.
fn tab_bar(ui: &mut Ui, state: &mut ChartSettingsState, tabs: &[Tab]) {
    let items: Vec<TabItem<Tab>> = tabs
        .iter()
        .map(|&tab| TabItem {
            value: tab,
            title: tab.title(),
            icon: tab.icon(),
            count: 0,
        })
        .collect();
    ui.vertical_centered(|ui| {
        kit::tab_bar_regular(ui, Id::new("chart-settings-tabs"), &items, &mut state.tab);
    });
}

/// The footer row: "Remove Chart" on the left, the last template action's notice, "Done" on the
/// right. (The Templates menu sits beside each page's Reset button.) Returns what a button asked
/// for. Removing is not confirmed: the chart's drawings are kept, so it can be added back.
fn footer(ui: &mut Ui, state: &mut ChartSettingsState) -> Option<ChartSettingsAction> {
    let mut action = None;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if kit::destructive_sheet_button(ui, Icon::Trash, "Remove Chart").clicked() {
            action = Some(ChartSettingsAction::RemoveChart);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if kit::sheet_button(ui, "Done", true, true).clicked() {
                action = Some(ChartSettingsAction::Done);
            }
            // What the last template action did, between the buttons.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                templates::notice(ui, &mut state.templates);
            });
        });
    });
    ui.add_space(14.0);
    action
}

#[cfg(test)]
mod tests;
