//! The chart-type selector in a chart's header: a button showing the current type's glyph and a
//! menu of every type, grouped, with a filter box. The menu is for one chart card: it reports the
//! pick and the workspace applies it to that chart alone.

use super::icons::{self, Icon};
use super::kit;
use super::theme;
use crate::model::DataSource;
use crate::model::chart_type::{ChartGroup, ChartType};
use eframe::egui::{
    self, Align2, Color32, Id, Key, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

const ROW_HEIGHT: f32 = 26.0;
const MENU_WIDTH: f32 = 214.0;
const GLYPH: f32 = 15.0;

/// The header button. Returns the type picked this frame.
pub fn button(
    ui: &mut egui::Ui,
    current: ChartType,
    source: DataSource,
    chart_id: u64,
) -> Option<ChartType> {
    let size = 27.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let tooltip = format!("Chart type: {}", current.label());
    let response = super::accessibility::label(response, &tooltip).on_hover_text(&tooltip);
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    if ui.is_rect_visible(rect) {
        let fill = if open || response.is_pointer_button_down_on() {
            theme::ink(26)
        } else if response.hovered() {
            theme::ink(14)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        glyph(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(size * 0.6)),
            current,
            theme::text_secondary(),
        );
    }

    let query_id = Id::new(("chart-type-filter", chart_id));
    if !open {
        ui.data_mut(|data| data.remove::<String>(query_id));
    }
    let mut picked = None;
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        // The default closes on any click, which would shut the menu on the search field.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(MENU_WIDTH)
        .show(|ui| {
            ui.set_min_width(MENU_WIDTH - 10.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut query = ui
                .data(|data| data.get_temp::<String>(query_id))
                .unwrap_or_default();
            let filter = kit::search_field(
                ui,
                query_id.with("field"),
                &mut query,
                "Search chart types",
                ui.available_width(),
            );
            if !filter.has_focus()
                && !ui.data(|data| {
                    data.get_temp::<bool>(query_id.with("seen"))
                        .unwrap_or(false)
                })
            {
                filter.request_focus();
            }
            ui.data_mut(|data| data.insert_temp(query_id.with("seen"), true));
            ui.add_space(4.0);
            let matching = matching_types(&query);
            let enter = filter.has_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let mut shown_any = false;
            for group in ChartGroup::ALL {
                let rows: Vec<ChartType> = matching
                    .iter()
                    .copied()
                    .filter(|kind| kind.group() == group)
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                shown_any = true;
                ui.add_space(3.0);
                ui.label(
                    egui::RichText::new(group.label().to_uppercase())
                        .size(9.5)
                        .strong()
                        .color(theme::text_tertiary()),
                );
                ui.add_space(2.0);
                for kind in rows {
                    let reason = kind.unsupported_reason(source);
                    let (clicked, response) = row(ui, kind, kind == current, reason.is_none());
                    if let Some(reason) = reason {
                        response.on_hover_text(reason);
                    }
                    let first_match = enter
                        && matching
                            .iter()
                            .find(|k| k.unsupported_reason(source).is_none())
                            == Some(&kind);
                    if (clicked || first_match) && reason.is_none() {
                        picked = Some(kind);
                        ui.close();
                    }
                }
            }
            if !shown_any {
                ui.label(
                    egui::RichText::new("No matching chart type")
                        .size(11.0)
                        .color(theme::text_tertiary()),
                );
            }
            ui.data_mut(|data| data.insert_temp(query_id, query));
        });
    picked
}

/// The types whose name contains `query`, in the menu's order.
pub fn matching_types(query: &str) -> Vec<ChartType> {
    let needle = query.trim().to_lowercase();
    ChartGroup::ALL
        .into_iter()
        .flat_map(ChartType::in_group)
        .filter(|kind| needle.is_empty() || kind.label().to_lowercase().contains(&needle))
        .collect()
}

fn row(
    ui: &mut egui::Ui,
    kind: ChartType,
    selected: bool,
    enabled: bool,
) -> (bool, egui::Response) {
    let width = ui.available_width();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), sense);
    let highlighted = enabled && response.hovered();
    if highlighted {
        ui.painter()
            .rect_filled(rect, 5.0, theme::selected_tint(true));
    } else if selected {
        ui.painter().rect_filled(rect, 5.0, theme::ink(14));
    }
    let ink = match (enabled, highlighted) {
        (false, _) => theme::text_tertiary(),
        (true, _) => theme::text_primary(),
    };
    glyph(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + 17.0, rect.center().y),
            Vec2::splat(GLYPH),
        ),
        kind,
        ink,
    );
    ui.painter().text(
        pos2(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        kind.label(),
        egui::FontId::proportional(12.5),
        ink,
    );
    if selected {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(rect.right() - 14.0, rect.center().y),
                Vec2::splat(11.0),
            ),
            Icon::Check,
            ink,
        );
    }
    (enabled && response.clicked(), response)
}

/// A miniature of what the type draws, in `ink`, inside `rect`.
pub fn glyph(painter: &egui::Painter, rect: Rect, kind: ChartType, ink: Color32) {
    let at = |x: f32, y: f32| {
        pos2(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let line = Stroke::new(1.3, ink);
    let thin = Stroke::new(1.0, ink);
    let polyline = |points: &[(f32, f32)], stroke: Stroke| {
        painter.add(Shape::line(
            points.iter().map(|(x, y)| at(*x, *y)).collect(),
            stroke,
        ));
    };
    let candle = |x: f32, top: f32, bottom: f32, high: f32, low: f32, half: f32, hollow: bool| {
        painter.line_segment([at(x, high), at(x, top)], thin);
        painter.line_segment([at(x, bottom), at(x, low)], thin);
        let body = Rect::from_min_max(at(x - half, top), at(x + half, bottom));
        if hollow {
            painter.rect_stroke(body, 0.0, thin, StrokeKind::Inside);
        } else {
            painter.rect_filled(body, 0.0, ink);
        }
    };
    let faint = ink.gamma_multiply(0.28);
    match kind {
        ChartType::Candles => {
            candle(0.28, 0.4, 0.78, 0.2, 0.92, 0.14, false);
            candle(0.72, 0.18, 0.55, 0.08, 0.7, 0.14, false);
        }
        ChartType::HollowCandles => {
            candle(0.28, 0.4, 0.78, 0.2, 0.92, 0.14, false);
            candle(0.72, 0.18, 0.55, 0.08, 0.7, 0.14, true);
        }
        ChartType::VolumeCandles => {
            candle(0.22, 0.42, 0.78, 0.25, 0.92, 0.1, false);
            candle(0.66, 0.2, 0.6, 0.1, 0.75, 0.2, false);
        }
        ChartType::Bars => {
            for (x, high, low, open, close) in
                [(0.3, 0.15, 0.85, 0.6, 0.3), (0.72, 0.1, 0.7, 0.55, 0.2)]
            {
                painter.line_segment([at(x, high), at(x, low)], line);
                painter.line_segment([at(x - 0.17, open), at(x, open)], line);
                painter.line_segment([at(x, close), at(x + 0.17, close)], line);
            }
        }
        ChartType::HighLowBars => {
            for (x, high, low) in [(0.2, 0.3, 0.9), (0.5, 0.1, 0.65), (0.8, 0.25, 0.8)] {
                painter.line_segment([at(x, high), at(x, low)], Stroke::new(2.0, ink));
            }
        }
        ChartType::Line => polyline(
            &[(0.05, 0.75), (0.32, 0.4), (0.55, 0.6), (0.95, 0.15)],
            line,
        ),
        ChartType::LineWithMarkers => {
            let points = [(0.1, 0.75), (0.37, 0.4), (0.62, 0.6), (0.9, 0.18)];
            polyline(&points, line);
            for (x, y) in points {
                painter.circle(at(x, y), 2.2, theme::surface(), line);
            }
        }
        ChartType::StepLine => polyline(
            &[
                (0.05, 0.75),
                (0.35, 0.75),
                (0.35, 0.4),
                (0.65, 0.4),
                (0.65, 0.6),
                (0.95, 0.6),
                (0.95, 0.15),
            ],
            line,
        ),
        ChartType::Area => {
            let top = [(0.05, 0.7), (0.32, 0.35), (0.6, 0.55), (0.95, 0.2)];
            let mut mesh = egui::epaint::Mesh::default();
            for pair in top.windows(2) {
                let base = mesh.vertices.len() as u32;
                for (x, y, c) in [
                    (pair[0].0, pair[0].1, faint),
                    (pair[1].0, pair[1].1, faint),
                    (pair[1].0, 0.92, Color32::TRANSPARENT),
                    (pair[0].0, 0.92, Color32::TRANSPARENT),
                ] {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: at(x, y),
                        uv: egui::epaint::WHITE_UV,
                        color: c,
                    });
                }
                mesh.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            painter.add(Shape::mesh(mesh));
            polyline(&top, line);
        }
        ChartType::HlcArea => {
            let high = [(0.05, 0.4), (0.35, 0.15), (0.65, 0.3), (0.95, 0.1)];
            let low = [(0.05, 0.85), (0.35, 0.6), (0.65, 0.8), (0.95, 0.55)];
            let mut band = high.to_vec();
            band.extend(low.iter().rev());
            painter.add(Shape::convex_polygon(
                band.iter().map(|(x, y)| at(*x, *y)).collect(),
                faint,
                Stroke::NONE,
            ));
            polyline(&high, thin);
            polyline(&low, thin);
            polyline(
                &[(0.05, 0.62), (0.35, 0.4), (0.65, 0.55), (0.95, 0.3)],
                line,
            );
        }
        ChartType::Baseline => {
            painter.extend(Shape::dashed_line(
                &[at(0.02, 0.5), at(0.98, 0.5)],
                Stroke::new(1.0, faint),
                2.0,
                2.0,
            ));
            polyline(
                &[
                    (0.05, 0.8),
                    (0.3, 0.62),
                    (0.45, 0.5),
                    (0.65, 0.2),
                    (0.95, 0.35),
                ],
                line,
            );
        }
        ChartType::Columns => {
            for (index, height) in [0.35, 0.6, 0.45, 0.85].into_iter().enumerate() {
                let x = 0.1 + index as f32 * 0.24;
                painter.rect_filled(
                    Rect::from_min_max(at(x, 1.0 - height), at(x + 0.16, 0.95)),
                    0.0,
                    ink,
                );
            }
        }
        ChartType::HeikinAshi => {
            candle(0.2, 0.55, 0.85, 0.5, 0.9, 0.11, false);
            candle(0.5, 0.35, 0.68, 0.3, 0.72, 0.11, false);
            candle(0.8, 0.15, 0.48, 0.1, 0.52, 0.11, false);
        }
        ChartType::Renko => {
            for (x, y) in [(0.1, 0.62), (0.4, 0.38), (0.7, 0.14)] {
                painter.rect_filled(
                    Rect::from_min_max(at(x, y), at(x + 0.22, y + 0.24)),
                    0.0,
                    ink,
                );
            }
        }
        ChartType::Kagi => {
            polyline(
                &[
                    (0.05, 0.75),
                    (0.3, 0.75),
                    (0.3, 0.3),
                    (0.55, 0.3),
                    (0.55, 0.9),
                    (0.8, 0.9),
                    (0.8, 0.15),
                ],
                line,
            );
        }
        ChartType::LineBreak => {
            for (x, top, bottom) in [
                (0.08, 0.5, 0.8),
                (0.32, 0.25, 0.6),
                (0.56, 0.1, 0.4),
                (0.8, 0.35, 0.65),
            ] {
                painter.rect_filled(
                    Rect::from_min_max(at(x, top), at(x + 0.16, bottom)),
                    0.0,
                    ink,
                );
            }
        }
        ChartType::PointAndFigure => {
            for (x, y) in [(0.28, 0.25), (0.28, 0.6)] {
                painter.line_segment([at(x - 0.12, y - 0.12), at(x + 0.12, y + 0.12)], thin);
                painter.line_segment([at(x - 0.12, y + 0.12), at(x + 0.12, y - 0.12)], thin);
            }
            for (x, y) in [(0.72, 0.42), (0.72, 0.78)] {
                painter.circle_stroke(at(x, y), 0.13 * rect.width(), thin);
            }
        }
        ChartType::Range => {
            candle(0.2, 0.55, 0.8, 0.45, 0.8, 0.1, false);
            candle(0.5, 0.3, 0.55, 0.25, 0.55, 0.1, false);
            candle(0.8, 0.5, 0.75, 0.3, 0.75, 0.1, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    #[test]
    fn the_filter_matches_names_in_menu_order() {
        let all = matching_types("");
        assert_eq!(all.len(), ChartType::ALL.len());
        assert_eq!(all[0], ChartType::Candles);
        assert_eq!(matching_types("  hollow "), vec![ChartType::HollowCandles]);
        assert_eq!(
            matching_types("candle"),
            vec![
                ChartType::Candles,
                ChartType::HollowCandles,
                ChartType::VolumeCandles
            ]
        );
        assert!(matching_types("nothing like this").is_empty());
    }

    #[test]
    fn every_type_has_a_glyph_that_paints_without_panicking() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for kind in ChartType::ALL {
                glyph(
                    ui.painter(),
                    Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(16.0)),
                    kind,
                    Color32::WHITE,
                );
            }
        });
        output.textures_delta.clear();
    }

    /// Opens the selector with a click on the button, then clicks at `then` (if given) and runs a
    /// few frames; returns whether a menu is open and every type the frames reported picking
    /// (none: nothing was picked).
    fn open_selector(source: DataSource, then: Option<egui::Pos2>) -> (bool, Vec<ChartType>) {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 700.0));
        let mut time = 0.0;
        let mut picked = Vec::new();
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                if let Some(kind) = button(ui, ChartType::Candles, source, 7) {
                    picked.push(kind);
                }
            });
            output.textures_delta.clear();
        };
        let at = pos2(21.0, 21.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![click(true)]);
        frame(vec![click(false)]);
        for _ in 0..3 {
            frame(Vec::new());
        }
        if let Some(pos) = then {
            let click = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(pos)]);
            frame(vec![click(true)]);
            frame(vec![click(false)]);
            for _ in 0..3 {
                frame(Vec::new());
            }
        }
        (egui::Popup::is_any_open(&ctx), picked)
    }

    #[test]
    fn clicking_the_button_opens_the_menu_without_picking_anything() {
        for source in [DataSource::Binance, DataSource::CoinGecko] {
            let (open, picked) = open_selector(source, None);
            assert!(open, "{source:?}");
            assert!(picked.is_empty());
        }
    }

    #[test]
    fn clicking_the_search_field_keeps_the_menu_open() {
        // The field sits in the menu's top row, just under the button.
        let (open, picked) = open_selector(DataSource::Binance, Some(pos2(60.0, 40.0)));
        assert!(open);
        assert!(picked.is_empty());
    }

    #[test]
    fn clicking_outside_the_menu_closes_it() {
        let (open, picked) = open_selector(DataSource::Binance, Some(pos2(560.0, 660.0)));
        assert!(!open);
        assert!(picked.is_empty());
    }
}
