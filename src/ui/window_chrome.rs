//! The window frame around the tab strip. A tab window has no native title bar: the strip is the
//! title bar, as in Firefox. macOS keeps its traffic lights, drawn by AppKit over the strip's left
//! end (`leading_inset` keeps tabs clear of them). Windows and Linux have no decorations; the
//! strip draws minimize / maximize / close at its right end (`trailing_inset`) and, where the
//! window system does not provide resize borders, [`edge_at`] finds the edge under the pointer.

use super::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, ViewportBuilder, pos2,
    vec2, viewport::ResizeDirection,
};

/// Room the native traffic lights take at the strip's left end (macOS).
const TRAFFIC_LIGHTS_WIDTH: f32 = 78.0;
const CONTROL_WIDTH: f32 = 46.0;
const CONTROL_COUNT: f32 = 3.0;
/// Width of the band along a window edge that starts a resize.
pub const RESIZE_MARGIN: f32 = 5.0;

/// A click on one of the drawn window buttons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    Minimize,
    ToggleMaximize,
    Close,
}

/// The window's application id (Wayland `app_id`, X11 class): what a Linux launcher or taskbar
/// matches to the `.desktop` file, and so to its icon. Permanent; see `docs/flatpak.md`.
pub const APP_ID: &str = "es.nico.degenview";

/// The app icon with its corners rounded, for the window systems that draw an icon as it is.
#[cfg(not(target_os = "macos"))]
pub fn app_icon() -> std::sync::Arc<egui::IconData> {
    use std::sync::{Arc, OnceLock};
    static ICON: OnceLock<Arc<egui::IconData>> = OnceLock::new();
    Arc::clone(ICON.get_or_init(|| {
        let bytes = include_bytes!("../../assets/brand/DegenView-512-rounded.png");
        Arc::new(eframe::icon_data::from_png_bytes(bytes).expect("the bundled icon decodes"))
    }))
}

/// Gives a window the app id and icon the first (hidden) viewport has, so a taskbar or Alt+Tab
/// shows the app's icon for every window, not only the one eframe made. macOS windows share the
/// Dock's icon, so they are left alone.
pub fn branded(builder: ViewportBuilder) -> ViewportBuilder {
    #[cfg(target_os = "macos")]
    {
        builder
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder.with_app_id(APP_ID).with_icon(app_icon())
    }
}

/// Whether tab windows round their own corners (`window_corners`): on Linux, where they have no
/// native frame to do it, and the system composites, so a transparent window can show what is behind
/// its corners. A Wayland session always composites; on X11 a compositor must own the screen's
/// `_NET_WM_CM_S<n>` selection, or the corners would come out black. Decided once, at startup: a
/// compositor started later is not noticed. macOS rounds its windows itself, and Windows is not
/// done yet.
pub fn rounded_corners() -> bool {
    #[cfg(target_os = "linux")]
    {
        static ROUNDED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ROUNDED.get_or_init(|| {
            std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
                || x11_compositing()
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether the X server has a compositing manager, which says so by owning `_NET_WM_CM_S<screen>`
/// (EWMH). Any trouble reaching the server counts as no.
#[cfg(target_os = "linux")]
fn x11_compositing() -> bool {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    let Ok((connection, screen)) = x11rb::connect(None) else {
        return false;
    };
    let owner = connection
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|atom| connection.get_selection_owner(atom.atom).ok())
        .and_then(|cookie| cookie.reply().ok());
    owner.is_some_and(|reply| reply.owner != x11rb::NONE)
}

/// Rounds the corners of the tab window being drawn, when it should be: last in the frame, over
/// everything, and not while maximized or fullscreen.
pub fn paint_rounded_corners(ctx: &egui::Context) {
    let (maximized, fullscreen) = ctx.input(|input| {
        let viewport = input.viewport();
        (
            viewport.maximized == Some(true),
            viewport.fullscreen == Some(true),
        )
    });
    if super::window_corners::corners_wanted(rounded_corners(), maximized, fullscreen) {
        super::window_corners::erase(ctx, super::window_corners::WINDOW_CORNER_RADIUS);
    }
}

/// Turns a tab window's viewport into one without a native title bar.
pub fn apply(builder: ViewportBuilder) -> ViewportBuilder {
    if cfg!(target_os = "macos") {
        // The content runs under a transparent title bar; the traffic lights stay.
        builder
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
    } else {
        branded(builder)
            .with_decorations(false)
            // See through to the desktop at the rounded corners (`window_corners`).
            .with_transparent(rounded_corners())
    }
}

/// Strip room kept free on the left. Fullscreen windows have no traffic lights.
pub const fn leading_inset(fullscreen: bool) -> f32 {
    if cfg!(target_os = "macos") && !fullscreen {
        TRAFFIC_LIGHTS_WIDTH
    } else {
        0.0
    }
}

/// Strip room kept free on the right, for the drawn window buttons.
pub const fn trailing_inset() -> f32 {
    if cfg!(target_os = "macos") {
        0.0
    } else {
        CONTROL_WIDTH * CONTROL_COUNT
    }
}

/// Whether this window draws its own resize borders (no decorations, and no native border to grab).
pub const fn resizes_itself() -> bool {
    cfg!(all(not(target_os = "macos"), not(windows)))
}

/// Draws the window buttons in `strip` (the whole strip) and reports a click.
pub fn window_controls(ui: &mut Ui, strip: Rect, maximized: bool) -> Option<Control> {
    if trailing_inset() == 0.0 {
        return None;
    }
    let mut clicked = None;
    let painter = ui.painter().clone();
    let controls = [
        (Control::Minimize, "Minimize"),
        (
            Control::ToggleMaximize,
            if maximized { "Restore" } else { "Maximize" },
        ),
        (Control::Close, "Close"),
    ];
    for (index, (control, name)) in controls.into_iter().enumerate() {
        let left = strip.right() - CONTROL_WIDTH * (CONTROL_COUNT - index as f32);
        let rect = Rect::from_min_size(
            pos2(left, strip.top()),
            vec2(CONTROL_WIDTH, strip.height() - 1.0),
        );
        let response = ui.interact(rect, Id::new(("window-control", name)), Sense::click());
        let response = super::accessibility::label(response, name).on_hover_text(name);
        let danger = control == Control::Close;
        if response.hovered() {
            painter.rect_filled(
                rect,
                0.0,
                if danger {
                    Color32::from_rgb(196, 43, 28)
                } else {
                    theme::ink(18)
                },
            );
        }
        let color = if danger && response.hovered() {
            Color32::WHITE
        } else {
            theme::text_primary()
        };
        paint_glyph(&painter, rect.center(), control, maximized, color);
        if response.clicked() {
            clicked = Some(control);
        }
    }
    clicked
}

fn paint_glyph(
    painter: &egui::Painter,
    center: Pos2,
    control: Control,
    maximized: bool,
    c: Color32,
) {
    let stroke = Stroke::new(1.0, c);
    let half = 5.0;
    match control {
        Control::Minimize => {
            painter.hline(
                center.x - half..=center.x + half,
                center.y.round() + 0.5,
                stroke,
            );
        }
        Control::ToggleMaximize if maximized => {
            // Two overlapping squares: the front one, and the corner of the one behind it.
            let front = Rect::from_min_size(
                pos2(center.x - half, center.y - half + 2.0),
                Vec2::splat(8.0),
            );
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
            let back = [
                pos2(front.left() + 2.0, front.top()),
                pos2(front.left() + 2.0, front.top() - 2.0),
                pos2(front.right() + 2.0, front.top() - 2.0),
                pos2(front.right() + 2.0, front.bottom() - 2.0),
                pos2(front.right(), front.bottom() - 2.0),
            ];
            painter.add(egui::Shape::line(back.to_vec(), stroke));
        }
        Control::ToggleMaximize => {
            let square = Rect::from_center_size(center, Vec2::splat(half * 2.0 - 1.0));
            painter.rect_stroke(square, 0.0, stroke, egui::StrokeKind::Inside);
        }
        Control::Close => {
            let r = half - 0.5;
            painter.line_segment([center + vec2(-r, -r), center + vec2(r, r)], stroke);
            painter.line_segment([center + vec2(-r, r), center + vec2(r, -r)], stroke);
        }
    }
}

/// The resize direction for a pointer at `pos` in a window of `size`, within `margin` of an edge.
/// Corners win over edges.
pub fn edge_at(pos: Pos2, size: Vec2, margin: f32) -> Option<ResizeDirection> {
    if pos.x < 0.0 || pos.y < 0.0 || pos.x > size.x || pos.y > size.y {
        return None;
    }
    let west = pos.x < margin;
    let east = pos.x > size.x - margin;
    let north = pos.y < margin;
    let south = pos.y > size.y - margin;
    match (north, south, west, east) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (true, _, _, true) => Some(ResizeDirection::NorthEast),
        (_, true, true, _) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, ..) => Some(ResizeDirection::North),
        (_, true, ..) => Some(ResizeDirection::South),
        (_, _, true, _) => Some(ResizeDirection::West),
        (_, _, _, true) => Some(ResizeDirection::East),
        _ => None,
    }
}

fn cursor_for(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// Resize borders for a window that has none of its own (Linux): the cursor over an edge, and a
/// press there hands the resize to the window system.
pub fn handle_resize_edges(ctx: &egui::Context) {
    if !resizes_itself() {
        return;
    }
    let (pointer, size, pressed, locked) = ctx.input(|input| {
        let viewport = input.viewport();
        (
            input.pointer.hover_pos(),
            input.content_rect().size(),
            input.pointer.primary_pressed(),
            viewport.maximized == Some(true) || viewport.fullscreen == Some(true),
        )
    });
    if locked {
        return;
    }
    let Some(direction) = pointer.and_then(|pos| edge_at(pos, size, RESIZE_MARGIN)) else {
        return;
    };
    ctx.set_cursor_icon(cursor_for(direction));
    if pressed {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_keeps_clear_of_the_window_buttons() {
        if cfg!(target_os = "macos") {
            assert_eq!(leading_inset(false), TRAFFIC_LIGHTS_WIDTH);
            assert_eq!(leading_inset(true), 0.0);
            assert_eq!(trailing_inset(), 0.0);
        } else {
            assert_eq!(leading_inset(false), 0.0);
            assert_eq!(trailing_inset(), CONTROL_WIDTH * CONTROL_COUNT);
        }
    }

    #[test]
    fn edges_and_corners_start_the_matching_resize() {
        let size = vec2(800.0, 600.0);
        let m = RESIZE_MARGIN;
        assert_eq!(edge_at(pos2(400.0, 300.0), size, m), None);
        assert_eq!(
            edge_at(pos2(400.0, 1.0), size, m),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            edge_at(pos2(400.0, 599.0), size, m),
            Some(ResizeDirection::South)
        );
        assert_eq!(
            edge_at(pos2(1.0, 300.0), size, m),
            Some(ResizeDirection::West)
        );
        assert_eq!(
            edge_at(pos2(799.0, 300.0), size, m),
            Some(ResizeDirection::East)
        );
        assert_eq!(
            edge_at(pos2(1.0, 1.0), size, m),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            edge_at(pos2(799.0, 1.0), size, m),
            Some(ResizeDirection::NorthEast)
        );
        assert_eq!(
            edge_at(pos2(1.0, 599.0), size, m),
            Some(ResizeDirection::SouthWest)
        );
        assert_eq!(
            edge_at(pos2(799.0, 599.0), size, m),
            Some(ResizeDirection::SouthEast)
        );
    }

    #[test]
    fn a_pointer_outside_the_window_resizes_nothing() {
        let size = vec2(800.0, 600.0);
        assert_eq!(edge_at(pos2(-1.0, 300.0), size, RESIZE_MARGIN), None);
        assert_eq!(edge_at(pos2(400.0, 601.0), size, RESIZE_MARGIN), None);
    }
}
