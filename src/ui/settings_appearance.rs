//! Settings ▸ Appearance: the app theme, picked from small previews of a window in each theme.

use crate::model::tab::title::TabTitleStyle;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme::{self, AppTheme};
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2,
};

const PREVIEW_HEIGHT: f32 = 92.0;
const TAB_PREVIEW_HEIGHT: f32 = 64.0;
const TILE_GAP: f32 = 12.0;
/// Each tile keeps this width however wide the window is; extra room stays empty on the right.
const TILE_WIDTH: f32 = 170.0;

/// The theme tiles. Returns the theme picked this frame.
pub fn body(ui: &mut egui::Ui, current: AppTheme) -> Option<AppTheme> {
    let mut picked = None;
    super::settings_window::section(
        ui,
        "Theme",
        Some(&format!(
            "System follows your {}'s light or dark setting.",
            crate::platform::computer()
        )),
        |ui| {
            let width = TILE_WIDTH * 3.0 + TILE_GAP * 2.0;
            let (row, _) = ui.allocate_exact_size(
                Vec2::new(width, 10.0 + PREVIEW_HEIGHT + 10.0 + 18.0 + 10.0),
                Sense::hover(),
            );
            for (index, theme) in AppTheme::ALL.into_iter().enumerate() {
                let rect = Rect::from_min_size(
                    pos2(
                        row.left() + index as f32 * (TILE_WIDTH + TILE_GAP),
                        row.top(),
                    ),
                    Vec2::new(TILE_WIDTH, row.height()),
                );
                if tile(ui, rect, theme, theme == current) {
                    picked = Some(theme);
                }
            }
        },
    );
    picked.filter(|theme| *theme != current)
}

/// How chart tabs name themselves. Returns the choice made this frame. TradingView's own
/// setting is a layout name in place of the symbol; the price shown with the symbol is this
/// app's addition. Each choice is a tile with a miniature tab strip showing what it makes of a
/// tab, as the theme tiles above show a window.
pub fn tabs(ui: &mut egui::Ui, current: TabTitleStyle) -> Option<TabTitleStyle> {
    let mut picked = None;
    super::settings_window::section(
        ui,
        "Tabs",
        Some(
            "What a chart tab shows. The layout name and the price follow the tab's focused chart.",
        ),
        |ui| {
            let width = TILE_WIDTH * 3.0 + TILE_GAP * 2.0;
            let (row, _) = ui.allocate_exact_size(
                Vec2::new(width, 10.0 + TAB_PREVIEW_HEIGHT + 10.0 + 18.0 + 10.0),
                Sense::hover(),
            );
            for (index, style) in [
                TabTitleStyle::Full,
                TabTitleStyle::SymbolOnly,
                TabTitleStyle::LayoutOnly,
            ]
            .into_iter()
            .enumerate()
            {
                let rect = Rect::from_min_size(
                    pos2(
                        row.left() + index as f32 * (TILE_WIDTH + TILE_GAP),
                        row.top(),
                    ),
                    Vec2::new(TILE_WIDTH, row.height()),
                );
                if tab_tile(ui, rect, style, style == current) {
                    picked = Some(style);
                }
            }
            if current != TabTitleStyle::default() {
                ui.add_space(10.0);
                let restore = ui.add(
                    egui::Label::new(
                        egui::RichText::new("Restore default")
                            .size(kit::size::SUBHEADLINE + 1.0)
                            .underline()
                            .color(theme::text_secondary()),
                    )
                    .sense(Sense::click()),
                );
                if restore
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    picked = Some(TabTitleStyle::default());
                }
            }
        },
    );
    picked.filter(|style| *style != current)
}

/// The words a tab shows in `style`, as they read on a strip: the price is the app's addition.
fn tab_sample(style: TabTitleStyle) -> &'static str {
    match style {
        TabTitleStyle::Full => "BTC-USD 73,986 / Default",
        TabTitleStyle::SymbolOnly => "BTC-USD",
        TabTitleStyle::LayoutOnly => "Default",
    }
}

fn tab_label(style: TabTitleStyle) -> &'static str {
    match style {
        TabTitleStyle::Full => "Symbol, price and layout",
        TabTitleStyle::SymbolOnly => "Symbol only",
        TabTitleStyle::LayoutOnly => "Layout name only",
    }
}

/// One tab title style: a miniature tab strip with the title as it would read, and its name.
fn tab_tile(ui: &mut egui::Ui, rect: Rect, style: TabTitleStyle, selected: bool) -> bool {
    let response = ui
        .interact(
            rect,
            egui::Id::new(("appearance-tab-title", tab_label(style))),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label_as(
        response,
        egui::WidgetType::RadioButton,
        format!("{} tab title", tab_label(style)),
    );
    let painter = ui.painter();
    paint_tile_frame(painter, rect, selected, response.hovered());

    let preview = Rect::from_min_size(
        rect.min + Vec2::splat(10.0),
        Vec2::new(rect.width() - 20.0, TAB_PREVIEW_HEIGHT),
    );
    painter.rect_filled(preview, 8.0, theme::background());
    // The strip, and the open tab in it.
    let strip = Rect::from_min_size(preview.min, Vec2::new(preview.width(), 26.0));
    painter.rect_filled(
        strip,
        CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        },
        theme::ink(8),
    );
    let text = kit::galley(
        ui,
        tab_sample(style),
        9.5,
        kit::MEDIUM,
        theme::text_primary(),
        preview.width() - 36.0,
    );
    let tab = Rect::from_min_size(
        strip.min + Vec2::new(6.0, 4.0),
        Vec2::new(
            (10.0 + 6.0 + text.size().x + 12.0).min(strip.width() - 12.0),
            strip.height() - 8.0,
        ),
    );
    painter.rect_filled(tab, 6.0, theme::ink(16));
    painter.circle_filled(
        pos2(tab.left() + 10.0, tab.center().y),
        4.0,
        Color32::from_rgb(247, 147, 26),
    );
    painter.galley(
        pos2(tab.left() + 20.0, tab.center().y - text.size().y / 2.0),
        text,
        Color32::PLACEHOLDER,
    );
    // The chart under it, as faint strokes.
    for (index, length) in [0.62_f32, 0.4, 0.52].into_iter().enumerate() {
        painter.hline(
            preview.left() + 10.0..=preview.left() + 10.0 + (preview.width() - 20.0) * length,
            strip.bottom() + 12.0 + index as f32 * 8.0,
            Stroke::new(2.0, theme::ink(10)),
        );
    }
    painter.rect_stroke(
        preview,
        8.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );

    paint_tile_label(
        ui,
        preview,
        tab_label(style),
        selected,
        preview.bottom() + 10.0 + 9.0,
    );
    response.clicked()
}

/// A tile's plate: lightly filled, with a stronger edge when it is the one chosen.
fn paint_tile_frame(painter: &egui::Painter, rect: Rect, selected: bool, hovered: bool) {
    let fill = if selected {
        theme::selected_tint(false)
    } else if hovered {
        kit::primary(0.07)
    } else {
        kit::primary(0.03)
    };
    painter.rect_filled(rect, 12.0, fill);
    painter.rect_stroke(
        rect,
        12.0,
        if selected {
            theme::selected_stroke()
        } else {
            Stroke::new(1.0, kit::primary(0.12))
        },
        StrokeKind::Inside,
    );
}

/// The name under a tile's picture, with the round mark at its right end that says which is chosen.
fn paint_tile_label(ui: &egui::Ui, preview: Rect, label: &str, selected: bool, y: f32) {
    let painter = ui.painter();
    let title = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        preview.width() - 24.0,
    );
    painter.galley(
        pos2(preview.left() + 2.0, y - title.size().y / 2.0),
        title,
        Color32::PLACEHOLDER,
    );
    let check = Rect::from_center_size(pos2(preview.right() - 7.0, y), Vec2::splat(14.0));
    if selected {
        icons::paint(painter, check, Icon::CheckCircleFill, theme::text_primary());
    } else {
        icons::paint(
            painter,
            check,
            Icon::Circle,
            theme::text_secondary().gamma_multiply(0.6),
        );
    }
}

fn icon(theme: AppTheme) -> Icon {
    match theme {
        AppTheme::System => Icon::CircleLeftHalfFill,
        AppTheme::Light => Icon::SunMaxFill,
        AppTheme::Dark => Icon::MoonFill,
    }
}

/// One theme as a miniature window in that theme (System shows both halves), with its name.
fn tile(ui: &mut egui::Ui, rect: Rect, theme: AppTheme, selected: bool) -> bool {
    let response = ui
        .interact(
            rect,
            egui::Id::new(("appearance-theme", theme.title())),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label_as(
        response,
        egui::WidgetType::RadioButton,
        format!("{} theme", theme.title()),
    );
    let painter = ui.painter();
    paint_tile_frame(painter, rect, selected, response.hovered());

    let preview = Rect::from_min_size(
        rect.min + Vec2::splat(10.0),
        Vec2::new(rect.width() - 20.0, PREVIEW_HEIGHT),
    );
    match theme {
        AppTheme::Light => mini_window(painter, preview, false, CornerRadius::same(8)),
        AppTheme::Dark => mini_window(painter, preview, true, CornerRadius::same(8)),
        AppTheme::System => {
            let (left, right) = preview.split_left_right_at_fraction(0.5);
            mini_window(painter, left, false, corners(8, true, false));
            mini_window(painter, right, true, corners(8, false, true));
        }
    }
    painter.rect_stroke(
        preview,
        8.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );

    let label_y = preview.bottom() + 10.0 + 9.0;
    let tint = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(preview.left() + 7.0, label_y), Vec2::splat(13.0)),
        icon(theme),
        tint,
    );
    let title = kit::galley(
        ui,
        theme.title(),
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        preview.width() - 44.0,
    );
    painter.galley(
        pos2(preview.left() + 20.0, label_y - title.size().y / 2.0),
        title,
        Color32::PLACEHOLDER,
    );
    let check = Rect::from_center_size(pos2(preview.right() - 7.0, label_y), Vec2::splat(14.0));
    if selected {
        icons::paint(painter, check, Icon::CheckCircleFill, theme::text_primary());
    } else {
        icons::paint(
            painter,
            check,
            Icon::Circle,
            theme::text_secondary().gamma_multiply(0.6),
        );
    }
    response.clicked()
}

fn corners(radius: u8, left: bool, right: bool) -> CornerRadius {
    let left = if left { radius } else { 0 };
    let right = if right { radius } else { 0 };
    CornerRadius {
        nw: left,
        sw: left,
        ne: right,
        se: right,
    }
}

/// A stylised window in fixed light or dark colours: title bar with traffic lights, a rising
/// line chart on a card and two cards under it.
fn mini_window(painter: &egui::Painter, rect: Rect, dark: bool, radius: CornerRadius) {
    let gray = |white: f32| {
        let v = (white * 255.0).round() as u8;
        Color32::from_rgb(v, v, v)
    };
    let background = if dark { gray(0.13) } else { gray(0.96) };
    let bar = if dark { gray(0.2) } else { gray(0.88) };
    let card = if dark { gray(0.2) } else { Color32::WHITE };

    painter.rect_filled(rect, radius, background);
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), 14.0)),
        CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            sw: 0,
            se: 0,
        },
        bar,
    );
    for (index, color) in [
        Color32::from_rgb(255, 95, 87),
        Color32::from_rgb(254, 188, 46),
        Color32::from_rgb(40, 200, 64),
    ]
    .into_iter()
    .enumerate()
    {
        painter.circle_filled(
            pos2(
                rect.left() + 6.0 + 2.5 + index as f32 * 8.0,
                rect.top() + 7.0,
            ),
            2.5,
            color.gamma_multiply(0.85),
        );
    }

    let width = rect.width() - 16.0;
    let chart = Rect::from_min_size(rect.min + Vec2::new(8.0, 22.0), Vec2::new(width, 34.0));
    painter.rect_filled(chart, 4.0, card);
    let at = |x: f32, y: f32| Pos2::new(chart.left() + x, chart.top() + y);
    painter.add(egui::Shape::line(
        vec![
            at(4.0, 28.0),
            at(width * 0.3, 20.0),
            at(width * 0.5, 24.0),
            at(width * 0.75, 10.0),
            at(width - 4.0, 14.0),
        ],
        Stroke::new(1.5, Color32::from_rgb(52, 199, 89)),
    ));

    let half = (width - 4.0) / 2.0;
    for index in 0..2 {
        painter.rect_filled(
            Rect::from_min_size(
                rect.min + Vec2::new(8.0 + index as f32 * (half + 4.0), 62.0),
                Vec2::new(half, 22.0),
            ),
            3.0,
            card,
        );
    }
}
