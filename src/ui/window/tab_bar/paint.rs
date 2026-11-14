//! Painting the strip: a tab's text for the room it has, the separators between tabs, the tab
//! itself with its price, and the fades at the strip's edges.

use super::geometry::{close_rect, slot_of};
use super::{
    ICON_SIZE, LOGO_SIZE, Look, SEPARATOR_HEIGHT, TAB_GAP, TEXT_SIZE, TabBar, TabView, UNDERLINE,
};
use crate::model::InstrumentId;
use crate::model::tab;
use crate::model::tab::title::{Detail, Segments, TabChart};
use crate::model::tab::windows::TabId;
use crate::model::watchlist::quote::Direction;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CornerRadius, Painter, Rect, Shadow, Stroke, StrokeKind, Ui, Vec2, pos2,
};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

impl TabBar<'_> {
    /// The tab's text for the room it has, cached until the quote, the layout, the style or the
    /// width class changes so an idle frame builds no strings.
    fn segments(
        &self,
        ctx: &egui::Context,
        tab: &TabView,
        chart: &TabChart,
        detail: Detail,
    ) -> Arc<Segments> {
        let key = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            chart.source.hash(&mut hasher);
            chart.symbol.hash(&mut hasher);
            chart.title.hash(&mut hasher);
            chart.layout.hash(&mut hasher);
            chart.custom_title.hash(&mut hasher);
            (self.style as u8, detail as u8, self.quotes.revision()).hash(&mut hasher);
            hasher.finish()
        };
        let id = self.id("segments").with(tab.id);
        if let Some((cached, segments)) = ctx.data(|data| data.get_temp::<(u64, Arc<Segments>)>(id))
            && cached == key
        {
            return segments;
        }
        let quote = chart
            .wants_quote()
            .then(|| {
                self.quotes
                    .get(&InstrumentId::new(chart.source, &chart.symbol))
            })
            .flatten();
        let segments = Arc::new(tab::title::compose(
            self.style,
            chart,
            quote.as_ref(),
            detail,
        ));
        ctx.data_mut(|data| data.insert_temp(id, (key, Arc::clone(&segments))));
        segments
    }

    /// A hairline in the gap between two neighbouring tabs that are both at rest: neither is the
    /// showing tab, being dragged or under the pointer (those have a fill of their own), and
    /// nothing is on its way in between them. It follows the animated rects, so it slides with
    /// the tabs.
    pub(super) fn paint_separators(&self, ui: &Ui, rects: &[Rect], gap: Option<usize>) {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let dragged = self.drag.dragging.map(|(tab, _)| tab);
        let at_rest = |index: usize| {
            let tab = &self.tabs[index];
            tab.id != self.active
                && Some(tab.id) != dragged
                && !pointer.is_some_and(|pointer| rects[index].contains(pointer))
        };
        let painter = ui.painter();
        for (index, pair) in rects.windows(2).enumerate() {
            if slot_of(index + 1, gap) != slot_of(index, gap) + 1
                || !at_rest(index)
                || !at_rest(index + 1)
            {
                continue;
            }
            let rect = pair[0];
            let x = (rect.right() + TAB_GAP / 2.0).round() + 0.5;
            let cy = rect.center().y - 1.0;
            painter.vline(
                x,
                (cy - SEPARATOR_HEIGHT / 2.0)..=(cy + SEPARATOR_HEIGHT / 2.0),
                Stroke::new(1.0, theme::separator()),
            );
        }
    }

    pub(super) fn paint_tab(
        &self,
        ui: &Ui,
        painter: &Painter,
        rect: Rect,
        tab: &TabView,
        look: Look,
    ) {
        let fade = |color: Color32| color.gamma_multiply(look.opacity);
        let raised = look.active || look.lifted;
        let corners = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        if look.lifted {
            painter.add(
                Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 0,
                    color: Color32::from_black_alpha(70),
                }
                .as_shape(rect, corners),
            );
        }
        let fill = if raised {
            theme::background()
        } else if look.hovered {
            theme::ink(14)
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, corners, fade(fill));
        if look.lifted {
            painter.rect_stroke(
                rect,
                corners,
                Stroke::new(1.0, fade(theme::separator())),
                StrokeKind::Inside,
            );
        } else if look.active {
            // Joins the tab to the content below by hiding the strip's bottom rule under it.
            painter.hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                Stroke::new(1.5, theme::background()),
            );
        }
        if let Some(link) = tab.link {
            // The only coloured thing in the strip: the Tab Linking colour the tab is tagged
            // with. A tab with no colour has no line, showing or not.
            let line = Rect::from_min_max(
                pos2(rect.left() + 10.0, rect.bottom() - UNDERLINE),
                pos2(rect.right() - 10.0, rect.bottom()),
            );
            painter.rect_filled(
                line,
                UNDERLINE / 2.0,
                fade(crate::ui::window::tab_link_menu::swatch(link)),
            );
        }

        let primary = fade(if raised || look.hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        });
        let secondary = fade(theme::text_secondary());
        let weight = if raised { kit::SEMIBOLD } else { kit::MEDIUM };
        let close_rect = close_rect(rect);
        let right = close_rect.left() - 6.0;
        let cy = rect.center().y - 1.0;
        let narrow = rect.width() < Detail::SYMBOL_WIDTH;
        let mut x = rect.left() + if narrow { 9.0 } else { 12.0 };

        match &tab.chart {
            Some(chart) => {
                let detail = Detail::for_width(rect.width());
                let segments = self.segments(ui.ctx(), tab, chart, detail);
                let logo =
                    Rect::from_center_size(pos2(x + LOGO_SIZE / 2.0, cy), Vec2::splat(LOGO_SIZE));
                kit::chart_icon(ui, logo, &chart.symbol, chart.source, false);
                x = logo.right() + 8.0;
                if detail > Detail::Logo {
                    let text = kit::galley(
                        ui,
                        &segments.text,
                        TEXT_SIZE,
                        weight,
                        primary,
                        (right - x).max(0.0),
                    );
                    painter.galley(pos2(x, cy - text.size().y / 2.0), text.clone(), primary);
                    x += text.size().x;

                    if let Some(block) = &segments.price {
                        x += 9.0;
                        x = self.paint_price(
                            ui,
                            painter,
                            (x, cy, right),
                            (tab.id, chart, detail),
                            block,
                            (primary, weight, look.opacity),
                        );
                    }
                    if let Some(layout) = &segments.layout
                        && right - x > 48.0
                    {
                        x += 9.0;
                        let slash = kit::galley(
                            ui,
                            "/",
                            TEXT_SIZE,
                            kit::REGULAR,
                            secondary.gamma_multiply(0.7),
                            12.0,
                        );
                        painter.galley(
                            pos2(x, cy - slash.size().y / 2.0),
                            slash.clone(),
                            secondary,
                        );
                        x += slash.size().x + 9.0;
                        // Not cut with an ellipsis: the name runs on to the close button's edge
                        // and fades into the tab's own fill, like the strip's ends.
                        let name = kit::galley(
                            ui,
                            layout,
                            TEXT_SIZE,
                            kit::REGULAR,
                            secondary,
                            f32::INFINITY,
                        );
                        let room = (right - x).max(0.0);
                        let overflows = name.size().x > room;
                        let clipped = painter.with_clip_rect(painter.clip_rect().intersect(
                            Rect::from_min_max(pos2(x, rect.top()), pos2(right, rect.bottom())),
                        ));
                        clipped.galley(pos2(x, cy - name.size().y / 2.0), name, secondary);
                        if overflows {
                            let mut base = if raised {
                                theme::background()
                            } else {
                                theme::chrome()
                            };
                            if look.hovered && !raised {
                                base = base.blend(theme::ink(14));
                            }
                            let width = NAME_FADE.min(room);
                            fade_strip(
                                painter,
                                (right - width, right),
                                (cy - 10.0, cy + 10.0),
                                fade(base),
                            );
                        }
                    }
                }
            }
            None => {
                let icon_rect =
                    Rect::from_center_size(pos2(x + ICON_SIZE / 2.0, cy), Vec2::splat(ICON_SIZE));
                icons::paint(painter, icon_rect, tab.icon, primary);
                x = icon_rect.right() + 7.0;
                if narrow {
                    // Just the glyph: the tooltip names the tab.
                } else {
                    let text = kit::galley(
                        ui,
                        &tab.title,
                        TEXT_SIZE,
                        weight,
                        primary,
                        (right - x).max(0.0),
                    );
                    painter.galley(pos2(x, cy - text.size().y / 2.0), text, primary);
                }
            }
        }

        if look.hovered || look.close_hovered || look.active {
            if look.close_hovered {
                painter.rect_filled(close_rect, 5.0, theme::ink(26));
            }
            icons::paint(
                painter,
                Rect::from_center_size(close_rect.center(), Vec2::splat(9.0)),
                Icon::Xmark,
                primary,
            );
        }
    }

    /// `▲ 142.92 +0.96%` from `x`; returns where the next part starts. The block keeps the
    /// widest it has been for this market and width class, so the layout name after it does
    /// not shuffle as the digits change.
    fn paint_price(
        &self,
        ui: &Ui,
        painter: &Painter,
        (x, cy, right): (f32, f32, f32),
        (tab, chart, detail): (TabId, &TabChart, Detail),
        block: &tab::title::PriceBlock,
        (ink, weight, opacity): (Color32, f32, f32),
    ) -> f32 {
        let dim = if block.stale { 0.55 } else { 1.0 };
        let tone = match block.direction {
            Direction::Up => theme::UP,
            Direction::Down => theme::DOWN,
            Direction::Flat => theme::text_secondary(),
        }
        .gamma_multiply(opacity * dim);
        let ink = ink.gamma_multiply(dim);
        let clip = painter.with_clip_rect(Rect::from_min_max(
            pos2(x, cy - 12.0),
            pos2(right, cy + 12.0),
        ));
        let mut at = x;
        // The arrow's room is kept when the price is unchanged, so a tick across zero is not a jump.
        const ARROW: f32 = 7.0;
        let mid = pos2(at + ARROW / 2.0, cy);
        match block.direction {
            Direction::Up => {
                clip.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(mid.x, mid.y - 3.0),
                        pos2(mid.x + 3.5, mid.y + 3.0),
                        pos2(mid.x - 3.5, mid.y + 3.0),
                    ],
                    tone,
                    Stroke::NONE,
                ));
            }
            Direction::Down => {
                clip.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(mid.x, mid.y + 3.0),
                        pos2(mid.x + 3.5, mid.y - 3.0),
                        pos2(mid.x - 3.5, mid.y - 3.0),
                    ],
                    tone,
                    Stroke::NONE,
                ));
            }
            Direction::Flat => {}
        }
        at += ARROW + 4.0;
        let price = kit::digits(
            ui,
            &block.price,
            TEXT_SIZE,
            weight.min(kit::MEDIUM),
            ink,
            f32::INFINITY,
        );
        clip.galley(pos2(at, cy - price.size().y / 2.0), price.clone(), ink);
        at += price.size().x;
        if let Some(change) = &block.change {
            at += 6.0;
            let change = kit::digits(ui, change, TEXT_SIZE, kit::MEDIUM, tone, f32::INFINITY);
            clip.galley(pos2(at, cy - change.size().y / 2.0), change.clone(), tone);
            at += change.size().x;
        }
        let key = self
            .id("price-width")
            .with((tab, chart.source, &chart.symbol, detail as u8));
        let widest = ui.ctx().data_mut(|data| {
            let widest = data.get_temp_mut_or_insert_with(key, || 0.0f32);
            *widest = widest.max(at - x);
            *widest
        });
        x + widest
    }
}

/// A soft edge on a strip whose tabs run on past it: the strip's colour fading in over them.
pub(super) fn fade_edge(painter: &Painter, clip: Rect, left: bool) {
    const WIDTH: f32 = 18.0;
    let solid = theme::chrome();
    let (from, to) = if left {
        (clip.left() + WIDTH, clip.left())
    } else {
        (clip.right() - WIDTH, clip.right())
    };
    fade_strip(painter, (from, to), (clip.top(), clip.bottom()), solid);
}

/// How far a tab's layout name fades out before the close button's edge.
const NAME_FADE: f32 = 28.0;

/// A horizontal gradient from clear at `x.0` to `solid` at `x.1` (either may be the larger),
/// between `y.0` and `y.1`.
fn fade_strip(painter: &Painter, x: (f32, f32), y: (f32, f32), solid: Color32) {
    let clear = solid.gamma_multiply(0.0);
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    mesh.vertices.extend([
        egui::epaint::Vertex {
            pos: pos2(x.0, y.0),
            uv,
            color: clear,
        },
        egui::epaint::Vertex {
            pos: pos2(x.1, y.0),
            uv,
            color: solid,
        },
        egui::epaint::Vertex {
            pos: pos2(x.1, y.1),
            uv,
            color: solid,
        },
        egui::epaint::Vertex {
            pos: pos2(x.0, y.1),
            uv,
            color: clear,
        },
    ]);
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}
