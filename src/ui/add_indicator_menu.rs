//! The menu behind a chart header's "+": a search field that has the keyboard on open, the
//! built-in indicators not yet applied, then the saved scripts under a separator. A pick is
//! reported as an [`Action`] and the workspace applies it to that chart alone.

use super::chart_indicator_strip::Action;
use super::icons::Icon;
use super::kit::{self, MenuRow};
use super::script_catalog;
use super::theme;
use crate::model::{BuiltIn, DataSource, IndicatorConfig};
use crate::script_manager::model::{LocalScript, ScriptType};
use eframe::egui::{self, Key, Modifiers, RectAlign, Response, Ui};
use std::sync::Arc;

const WIDTH: f32 = 272.0;
/// The list scrolls beyond this; the search field and the footer stay put.
const LIST_HEIGHT: f32 = 268.0;
/// A new menu asks for the search field's focus this many frames: the first one only measures the
/// popup, and an `egui::Area` takes no focus while it does.
const FOCUS_FRAMES: u8 = 3;

/// One pickable row.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    BuiltIn(BuiltIn),
    Script(Arc<LocalScript>),
}

impl Entry {
    fn label(&self) -> &str {
        match self {
            Self::BuiltIn(kind) => kind.menu_title(),
            Self::Script(script) => &script.name,
        }
    }

    fn action(&self) -> Action {
        match self {
            Self::BuiltIn(kind) => Action::Add(*kind),
            Self::Script(script) => Action::AddScript(script.clone()),
        }
    }
}

/// What the menu offers for `query`: the built-ins whose title contains it, then the scripts that
/// match (see [`script_catalog::matching`]). Both lists keep their own order.
pub fn entries(query: &str, built_ins: &[BuiltIn], scripts: &[Arc<LocalScript>]) -> Vec<Entry> {
    let needle = query.trim().to_lowercase();
    let mut found: Vec<Entry> = built_ins
        .iter()
        .filter(|kind| needle.is_empty() || kind.menu_title().to_lowercase().contains(&needle))
        .map(|kind| Entry::BuiltIn(*kind))
        .collect();
    found.extend(
        script_catalog::matching(scripts, query)
            .into_iter()
            .map(|script| Entry::Script(script.clone())),
    );
    found
}

/// The index after pressing ↓ (`+1`) or ↑ (`-1`) with `len` rows, wrapping at both ends.
fn step(current: usize, by: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + by).rem_euclid(len as isize) as usize
}

#[derive(Clone, Default)]
struct State {
    query: String,
    highlighted: usize,
    frames: u8,
}

/// What the menu needs to know about the chart it belongs to.
pub struct Env<'a> {
    pub config: &'a IndicatorConfig,
    pub source: DataSource,
    /// The saved scripts already on the chart, by script id.
    pub on_chart: &'a [String],
}

/// Opens the menu from `button` while it is clicked. Writes the pick into `action`.
pub fn show(button: &Response, env: &Env, action: &mut Option<Action>) {
    let popup_id = button.id.with("add");
    let state_id = popup_id.with("state");
    let ctx = button.ctx.clone();
    if !egui::Popup::is_id_open(&ctx, popup_id) {
        ctx.data_mut(|data| data.remove::<State>(state_id));
    }
    kit::menu_popup(button)
        .id(popup_id)
        .align(RectAlign::BOTTOM_START)
        .gap(4.0)
        .show(|ui| {
            kit::menu_scope(ui, WIDTH);
            ui.set_width(WIDTH);
            let mut state = ui
                .data(|data| data.get_temp::<State>(state_id))
                .unwrap_or_default();
            body(ui, env, &mut state, action);
            ui.data_mut(|data| data.insert_temp(state_id, state));
        });
}

fn body(ui: &mut Ui, env: &Env, state: &mut State, action: &mut Option<Action>) {
    let has_scripts_section = !env.source.is_line_chart();
    let built_ins: Vec<BuiltIn> = BuiltIn::ALL
        .into_iter()
        .filter(|kind| !(env.source.is_line_chart() && *kind == BuiltIn::Volume))
        .filter(|kind| !kind.applied(env.config))
        .collect();
    let scripts = if has_scripts_section {
        script_catalog::scripts(ui.ctx())
    } else {
        Arc::default()
    };

    // The arrows and Return work while the search field has the focus. They are taken before the
    // field is drawn, so it does not move its caret for them.
    let (down, up, enter) = ui.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    let before = state.query.clone();
    let field = kit::search_field(
        ui,
        ui.id().with("search"),
        &mut state.query,
        if has_scripts_section {
            "Search indicators and scripts"
        } else {
            "Search indicators"
        },
        ui.available_width(),
    );
    if state.frames < FOCUS_FRAMES {
        state.frames += 1;
        field.request_focus();
    }
    if state.query != before {
        state.highlighted = 0;
    }
    ui.add_space(4.0);

    let shown = entries(&state.query, &built_ins, &scripts);
    if down || up {
        state.highlighted = step(state.highlighted, if down { 1 } else { -1 }, shown.len());
    }
    state.highlighted = state.highlighted.min(shown.len().saturating_sub(1));
    let moved = down || up || state.query != before;

    let mut picked = if enter {
        shown.get(state.highlighted).cloned()
    } else {
        None
    };
    let mut hovered = None;
    egui::ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            list(
                ui,
                env,
                &state.query,
                &shown,
                state.highlighted,
                moved,
                |entry, clicked, hover| {
                    if clicked {
                        picked = Some(entry.clone());
                    }
                    if hover {
                        hovered = Some(entry.clone());
                    }
                },
            );
        });
    if let Some(hovered) = hovered
        && let Some(index) = shown.iter().position(|entry| *entry == hovered)
    {
        // The pointer and the arrows share one highlight, so they never light two rows.
        state.highlighted = index;
    }

    kit::menu_separator(ui);
    if has_scripts_section && kit::menu_item(ui, Icon::Curlybraces, "Script Manager…", true, false)
    {
        *action = Some(Action::OpenScriptSettings);
        ui.close();
    }
    if kit::menu_item(ui, Icon::Sliders, "All Indicator Settings…", true, false) {
        *action = Some(Action::OpenIndicatorSettings);
        ui.close();
    }

    if let Some(entry) = picked {
        *action = Some(entry.action());
        ui.close();
    } else if enter {
        // Nothing to pick: Return must not drop the search field's focus.
        field.request_focus();
    }
}

/// Draws the sections and reports `(entry, clicked, hovered)` for each row.
fn list(
    ui: &mut Ui,
    env: &Env,
    query: &str,
    shown: &[Entry],
    highlighted: usize,
    scroll_to_highlight: bool,
    mut on_row: impl FnMut(&Entry, bool, bool),
) {
    let searching = !query.trim().is_empty();
    let has_built_ins = shown.iter().any(|e| matches!(e, Entry::BuiltIn(_)));
    let has_scripts = shown.iter().any(|e| matches!(e, Entry::Script(_)));
    let mut row = |ui: &mut Ui, index: usize, entry: &Entry| {
        let (icon, hint, tip) = match entry {
            Entry::BuiltIn(kind) => (kind.icon(), None, None),
            Entry::Script(script) => {
                let on_chart = env.on_chart.contains(&script.id);
                (
                    match script.script_type {
                        ScriptType::Strategy => Icon::Cycle,
                        _ => Icon::ChartLine,
                    },
                    on_chart.then_some("On chart"),
                    Some(if on_chart {
                        "Add another copy to this chart"
                    } else {
                        "Add to this chart"
                    }),
                )
            }
        };
        let response = kit::menu_row(
            ui,
            MenuRow {
                icon: Some(icon),
                shortcut: hint,
                focused: index == highlighted,
                ..MenuRow::new(entry.label())
            },
        );
        if index == highlighted && scroll_to_highlight {
            response.scroll_to_me(None);
        }
        let hover =
            response.hovered() && ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO);
        let clicked = response.clicked();
        if let Some(tip) = tip {
            response.on_hover_text(tip);
        }
        on_row(entry, clicked, hover);
    };

    let mut index = 0;
    if has_built_ins {
        kit::menu_heading(ui, "Built-in");
        for entry in shown.iter().filter(|e| matches!(e, Entry::BuiltIn(_))) {
            row(ui, index, entry);
            index += 1;
        }
    }
    if env.source.is_line_chart() {
        // Scripts do not run on a line chart, so there is no section for them.
    } else if has_scripts {
        if has_built_ins {
            kit::menu_separator(ui);
        }
        kit::menu_heading(ui, "Scripts");
        for entry in shown.iter().filter(|e| matches!(e, Entry::Script(_))) {
            row(ui, index, entry);
            index += 1;
        }
    } else if !searching {
        if has_built_ins {
            kit::menu_separator(ui);
        }
        kit::menu_heading(ui, "Scripts");
        quiet(ui, "No saved scripts yet");
    }
    if shown.is_empty() && searching {
        quiet(ui, &format!("No matches for “{}”", query.trim()));
    }
}

/// A dim line for a list with nothing to pick.
fn quiet(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(kit::text(
            text,
            kit::size::BODY,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_manager::model::ScriptType;
    use crate::ui::script_catalog::sample;

    fn scripts() -> Vec<Arc<LocalScript>> {
        ["Squeeze Momentum", "Supertrend", "VWAP"]
            .iter()
            .enumerate()
            .map(|(i, name)| Arc::new(sample(&i.to_string(), name, ScriptType::Indicator)))
            .collect()
    }

    fn labels(found: &[Entry]) -> Vec<&str> {
        found.iter().map(Entry::label).collect()
    }

    #[test]
    fn built_ins_come_first_then_scripts() {
        let found = entries("", &[BuiltIn::Rsi, BuiltIn::Ema], &scripts());
        let kinds: Vec<bool> = found
            .iter()
            .map(|e| matches!(e, Entry::BuiltIn(_)))
            .collect();
        assert_eq!(kinds, [true, true, false, false, false]);
    }

    #[test]
    fn a_query_filters_both_lists() {
        let found = entries("su", &[BuiltIn::Rsi, BuiltIn::Bollinger], &scripts());
        assert_eq!(labels(&found), ["Supertrend"]);
        let found = entries("RSI", &[BuiltIn::Rsi, BuiltIn::Ema], &scripts());
        assert_eq!(found, [Entry::BuiltIn(BuiltIn::Rsi)]);
        assert!(entries("zzz", &[BuiltIn::Rsi], &scripts()).is_empty());
    }

    #[test]
    fn a_picked_row_becomes_the_matching_action() {
        let list = scripts();
        assert_eq!(
            Entry::BuiltIn(BuiltIn::Ema).action(),
            Action::Add(BuiltIn::Ema)
        );
        assert_eq!(
            Entry::Script(list[1].clone()).action(),
            Action::AddScript(list[1].clone())
        );
    }

    #[test]
    fn the_highlight_wraps_at_both_ends() {
        assert_eq!(step(0, -1, 3), 2);
        assert_eq!(step(2, 1, 3), 0);
        assert_eq!(step(1, 1, 3), 2);
        assert_eq!(step(0, 1, 0), 0);
    }

    fn shape_text(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                out.push_str(text.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_text(shape, out)),
            _ => {}
        }
    }

    /// Opens the menu on the first frame and returns the context, the popup's id and everything the
    /// last frame wrote.
    fn open(source: DataSource, on_chart: &[String]) -> (egui::Context, egui::Id, String) {
        let ctx = egui::Context::default();
        script_catalog::publish(
            &ctx,
            vec![
                sample("a", "Alpha", ScriptType::Indicator),
                sample("b", "Beta", ScriptType::Strategy),
            ],
        );
        let config = IndicatorConfig::default();
        let env = Env {
            config: &config,
            source,
            on_chart,
        };
        let mut button_id = egui::Id::NULL;
        let mut text = String::new();
        for frame in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let (_, response) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                button_id = response.id.with("add");
                if frame == 0 {
                    egui::Popup::open_id(ui.ctx(), button_id);
                }
                show(&response, &env, &mut None);
            });
            text.clear();
            for clipped in &output.shapes {
                shape_text(&clipped.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        (ctx, button_id, text)
    }

    #[test]
    fn the_menu_opens_with_the_search_field_focused_and_lists_built_ins_then_scripts() {
        let (ctx, popup_id, text) = open(DataSource::Binance, &["a".to_owned()]);
        assert!(egui::Popup::is_id_open(&ctx, popup_id));
        assert!(
            ctx.memory(|memory| memory.focused()).is_some(),
            "the search field has the keyboard"
        );
        let (built_in, scripts, alpha, on_chart, footer) = (
            text.find("BUILT-IN").expect("built-in heading"),
            text.find("SCRIPTS").expect("scripts heading"),
            text.find("Alpha").expect("a script"),
            text.find("On chart").expect("the applied marker"),
            text.find("All Indicator Settings").expect("footer"),
        );
        assert!(
            built_in < scripts && scripts < alpha && alpha < footer,
            "{text}"
        );
        assert!(
            on_chart > alpha && on_chart < text.find("Beta").unwrap(),
            "{text}"
        );
        assert!(text.contains("Search indicators and scripts"), "{text}");
    }

    #[test]
    fn a_line_chart_gets_no_script_section() {
        let (_, _, text) = open(DataSource::CoinMarketCap, &[]);
        assert!(
            !text.contains("SCRIPTS") && !text.contains("Alpha"),
            "{text}"
        );
        assert!(text.contains("Search indicators"), "{text}");
        assert!(!text.contains("Script Manager"), "{text}");
    }

    /// The menu as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_menu() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        for (name, typed) in [
            ("add-menu", ""),
            ("add-menu-search", "sq"),
            ("add-menu-none", "zzz"),
        ] {
            let mut screen = Screen::new(egui::vec2(360.0, 440.0));
            script_catalog::publish(
                &screen.ctx,
                [
                    ("Squeeze Momentum", ScriptType::Indicator),
                    ("Supertrend Pro", ScriptType::Indicator),
                    ("VWAP Bands", ScriptType::Indicator),
                    ("Mean Reversion", ScriptType::Strategy),
                    ("Breakout Strategy", ScriptType::Strategy),
                    ("Liquidity Squeeze", ScriptType::Indicator),
                ]
                .iter()
                .enumerate()
                .map(|(i, (name, kind))| sample(&i.to_string(), name, *kind))
                .collect(),
            );
            let config = IndicatorConfig {
                show_rsi: true,
                ..IndicatorConfig::default()
            };
            let on_chart = ["0".to_owned()];
            let env = Env {
                config: &config,
                source: DataSource::Binance,
                on_chart: &on_chart,
            };
            let mut last = None;
            for frame in 0..8 {
                let events = if frame == 4 && !typed.is_empty() {
                    vec![egui::Event::Text(typed.to_owned())]
                } else {
                    Vec::new()
                };
                last = Some(screen.run(events, |ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let (_, response) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        if frame == 0 {
                            egui::Popup::open_id(ui.ctx(), response.id.with("add"));
                        }
                        show(&response, &env, &mut None);
                    });
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
