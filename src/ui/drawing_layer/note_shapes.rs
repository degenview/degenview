//! The text and note tools projected onto the plot: a box of text, a comment's bubble, a price
//! label's tag, a flag and a pin with the note that opens from it. Boxes are sized to their text
//! with the painting context's own fonts (`Projection::measure`), so the shape a click is tested
//! against, the handles and the toolbar's place are the ones the text is painted in.

use super::Projection;
use super::draw::TEXT_PAD;
use super::edit::Part;
use super::shape::{HandleSpot, Popup, Screen, Styled, TextPlace};
use crate::model::chart_type::LineDash;
use crate::model::drawing_style::DrawingStyle;
use crate::model::{Anchor, AnnotationKind, Geometry, LeaderKind};
use crate::ui::annotation_geometry as shapes;
use eframe::egui::{Pos2, Vec2, pos2, vec2};

/// What an empty box measures, so it still has a size to see and to click.
const PLACEHOLDER: &str = "Text";
/// The corner radius of a bubble, and of the plain box of the Text tool.
const BUBBLE_CORNER: f32 = 6.0;
const BOX_CORNER: f32 = 3.0;
const TAG_CORNER: f32 = 4.0;
/// The radius of a pin's stud.
const STUD: f32 = 3.0;

/// Builds the screen shape of a text or note tool. `anchors` are the geometry's anchors on
/// screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    text: &str,
    proj: &Projection,
    anchors: &[Pos2],
) -> Option<Screen> {
    let stored = *geometry.anchors().first()?;
    let at = *anchors.first()?;
    let screen = match geometry {
        Geometry::Annotation { kind, .. } => match kind {
            AnnotationKind::Text => text_box(style, text, proj, at),
            AnnotationKind::Comment => comment(style, text, proj, at),
            AnnotationKind::PriceLabel => price_label(style, proj, at, stored),
            AnnotationKind::FlagMark => flag(at),
            AnnotationKind::Pin => pin(style, text, proj, at),
        },
        Geometry::Leader { kind, .. } => {
            leader(*kind, style, text, proj, [at, *anchors.get(1)?], stored)
        }
        Geometry::Signpost { .. } => signpost(style, text, proj, at, *anchors.get(1)?),
        _ => return None,
    };
    screen.finish(6.0)
}

fn anchor_handle(pos: Pos2) -> HandleSpot {
    HandleSpot {
        part: Part::Anchor(0),
        pos,
        control: false,
    }
}

/// The handle that moves a label: the second anchor, by the pointer's travel.
fn label_handle(pos: Pos2) -> HandleSpot {
    HandleSpot {
        part: Part::Label,
        pos,
        control: true,
    }
}

/// The handle that drags a box's width, on the middle of its right edge.
fn width_handle(rect: eframe::egui::Rect) -> HandleSpot {
    HandleSpot {
        part: Part::Width,
        pos: rect.right_center(),
        control: true,
    }
}

/// How a box takes its width from its text.
#[derive(Clone, Copy, PartialEq)]
enum Width {
    /// As wide as the text.
    Text,
    /// As wide as the style says, however short the text.
    Fixed,
    /// As wide as the text, but no wider than the style says: longer text wraps.
    UpTo,
}

/// The size of the box that holds `shown`, and as tall as the lines it comes to. An empty text
/// measures as a placeholder.
fn dimensions(proj: &Projection, shown: &str, style: &DrawingStyle, width: Width) -> Vec2 {
    let shown = if shown.trim().is_empty() {
        PLACEHOLDER
    } else {
        shown
    };
    let natural = proj.measure.size(shown, &style.text, f32::INFINITY);
    let natural = shapes::box_size(natural, TEXT_PAD, vec2(0.0, 0.0));
    let outer = style.wrapped_width();
    if width == Width::Text || (width == Width::UpTo && natural.x <= outer) {
        return natural;
    }
    let size = proj
        .measure
        .size(shown, &style.text, outer - 2.0 * TEXT_PAD);
    vec2(outer, size.y.ceil() + 2.0 * TEXT_PAD)
}

/// The width rule of a box whose style may ask for wrapping.
fn wrapping(style: &DrawingStyle) -> Width {
    if style.wrap {
        Width::Fixed
    } else {
        Width::Text
    }
}

/// A screen with the background, border and click area of a box, its text inside it.
fn boxed(outline: Vec<Pos2>, rect: eframe::egui::Rect) -> Screen {
    let mut screen = Screen::empty();
    screen.fill = outline.clone();
    screen.frames.push(outline);
    screen.hit.push(rect);
    screen.text = Some(TextPlace::Inside(rect));
    screen
}

/// The Text tool: a box with its top-left corner on the anchor.
fn text_box(style: &DrawingStyle, text: &str, proj: &Projection, at: Pos2) -> Screen {
    let size = dimensions(proj, text, style, wrapping(style));
    let rect = eframe::egui::Rect::from_min_size(at, size);
    let mut screen = boxed(shapes::bubble(rect, BOX_CORNER, None, 0.0), rect);
    screen.handles = vec![anchor_handle(at), width_handle(rect)];
    screen
}

/// The Comment tool: a bubble above the anchor, its tail pointing at it.
fn comment(style: &DrawingStyle, text: &str, proj: &Projection, at: Pos2) -> Screen {
    let size = dimensions(proj, text, style, wrapping(style));
    let rect = shapes::comment_rect(at, size);
    let outline = shapes::bubble(rect, BUBBLE_CORNER, Some(at), shapes::COMMENT_TAIL);
    let mut screen = boxed(outline, rect);
    screen.handles = vec![anchor_handle(at), width_handle(rect)];
    screen
}

/// The Price label tool: a tag to the right of the anchor, pointing at it, with the price the
/// anchor stands at. The price is worked out here, from the anchor, in the chart's own format,
/// so it follows the anchor and is never stored.
fn price_label(style: &DrawingStyle, proj: &Projection, at: Pos2, stored: Anchor) -> Screen {
    let price = proj.format.format(stored.price);
    let size = dimensions(proj, &price, style, Width::Text);
    let rect = shapes::price_label_rect(at, size);
    let outline = shapes::bubble(rect, TAG_CORNER, Some(at), shapes::PRICE_LABEL_TAIL);
    let mut screen = boxed(outline, rect);
    screen.heading = Some(price);
    screen.handles = vec![anchor_handle(at)];
    screen
}

/// The radius of the dot where a note's leader meets its anchor, and of a signpost's plain marker
/// and of its emoji's disc.
const DOT: f32 = 4.0;
const SIGNPOST_DOT: f32 = 4.5;
const EMOJI_DISC: f32 = 11.0;
/// The width of a callout's tail where it leaves the bubble.
const CALLOUT_TAIL: f32 = 14.0;
/// The room (px) kept between a signpost's marker and its plate.
const SIGNPOST_GAP: f32 = 6.0;

/// The point of `rect` nearest `point`.
fn nearest_on(rect: eframe::egui::Rect, point: Pos2) -> Pos2 {
    pos2(
        point.x.clamp(rect.left(), rect.right()),
        point.y.clamp(rect.top(), rect.bottom()),
    )
}

/// The Note, Price note and Callout tools: a box centred on the second anchor, pointing at the
/// first. A note and a price note join it with a leader ending in a dot; a callout is a bubble
/// whose tail reaches the anchor. A price note writes the price of its anchor above its own text,
/// worked out here, in the chart's own format, so it follows the anchor and is never stored.
fn leader(
    kind: LeaderKind,
    style: &DrawingStyle,
    text: &str,
    proj: &Projection,
    [at, label]: [Pos2; 2],
    stored: Anchor,
) -> Screen {
    let heading = (kind == LeaderKind::PriceNote).then(|| proj.format.format(stored.price));
    let shown = match (&heading, text.trim().is_empty()) {
        (Some(heading), true) => heading.clone(),
        (Some(heading), false) => format!("{heading}\n{text}"),
        (None, _) => text.to_owned(),
    };
    let width = if kind == LeaderKind::PriceNote {
        Width::Text
    } else {
        wrapping(style)
    };
    let size = dimensions(proj, &shown, style, width);
    let rect = eframe::egui::Rect::from_center_size(label, size);
    let mut screen = match kind {
        LeaderKind::Callout => {
            let outline = shapes::bubble(rect, BUBBLE_CORNER, Some(at), CALLOUT_TAIL);
            boxed(outline, rect)
        }
        LeaderKind::Note | LeaderKind::PriceNote => {
            let corner = if kind == LeaderKind::PriceNote {
                TAG_CORNER
            } else {
                BUBBLE_CORNER
            };
            let mut screen = boxed(shapes::bubble(rect, corner, None, 0.0), rect);
            let join = nearest_on(rect, at);
            if join.distance(at) > 0.5 {
                screen.lines.push(vec![at, join]);
            }
            screen.discs.push((at, DOT, style.stroke.color));
            screen
        }
    };
    screen.heading = heading;
    screen.handles = vec![anchor_handle(at), label_handle(rect.center_top())];
    // Dragging the width turns wrapping on, so a box that does not wrap has the handle too.
    if kind != LeaderKind::PriceNote {
        screen.handles.push(width_handle(rect));
    }
    screen
}

/// The Signpost tool: a marker at the anchor, a stem straight up or down from it and a plate
/// with the text at the end of the stem. The plate is centred on the label's height, kept clear
/// of the marker, on the side of the anchor the label is on (above, when they are level).
fn signpost(style: &DrawingStyle, text: &str, proj: &Projection, at: Pos2, label: Pos2) -> Screen {
    let size = dimensions(proj, text, style, Width::Text);
    let marker = if style.emoji_shown {
        EMOJI_DISC
    } else {
        SIGNPOST_DOT
    };
    let side = if label.y > at.y { 1.0 } else { -1.0 };
    let reach = (label.y - at.y)
        .abs()
        .max(size.y / 2.0 + marker + SIGNPOST_GAP);
    let rect = eframe::egui::Rect::from_center_size(pos2(at.x, at.y + side * reach), size);
    let mut screen = boxed(shapes::bubble(rect, TAG_CORNER + 1.0, None, 0.0), rect);
    let near = if side < 0.0 {
        rect.bottom()
    } else {
        rect.top()
    };
    // The stem and the plain marker are the plate's colour: a signpost has the one.
    screen.styled.push(Styled {
        path: vec![pos2(at.x, at.y + side * marker), pos2(at.x, near)],
        color: style.fill.color,
        width: style.stroke.width,
        dash: LineDash::Solid,
    });
    if style.emoji_shown {
        screen.discs.push((at, marker, style.emoji_fill));
        screen.glyph = Some((at, style.emoji.glyph(), marker * 1.35));
    } else {
        screen.discs.push((at, marker, style.fill.color));
    }
    screen.handles = vec![anchor_handle(at), label_handle(rect.left_center())];
    screen
}

/// The Image tool: a picture whose top-left corner is the anchor, at a size of its own. The
/// rectangle is clickable whether or not the picture has arrived, and has the eight handles of a
/// box.
pub(super) fn picture(
    image: &crate::model::image_content::ImageData,
    anchors: &[Pos2],
) -> Option<Screen> {
    let rect = eframe::egui::Rect::from_min_size(
        *anchors.first()?,
        eframe::egui::vec2(image.size[0], image.size[1]),
    );
    let mut screen = Screen::empty();
    screen.hit.push(rect);
    screen.picture = Some(super::shape::PictureDraw {
        rect,
        asset: image.asset.clone(),
        name: image.name.clone(),
        opacity: image.clamped_opacity(),
    });
    screen.handles = super::shape::box_handles(rect);
    screen.finish(6.0)
}

/// The Post tool: a card whose top-left corner is the anchor. It is clickable all over,
/// and has the anchor and its bottom-right corner to grab.
pub(super) fn card(card: &crate::model::link_card::LinkCard, anchors: &[Pos2]) -> Option<Screen> {
    let at = *anchors.first()?;
    let rect =
        eframe::egui::Rect::from_min_size(at, eframe::egui::vec2(card.size[0], card.size[1]));
    let mut screen = Screen::empty();
    screen.hit.push(rect);
    screen.card = Some(super::shape::CardDraw {
        rect,
        card: card.clone(),
    });
    screen.handles = vec![
        anchor_handle(at),
        HandleSpot {
            part: Part::Box(super::edit::BoxHandle::BottomRight),
            pos: rect.right_bottom(),
            control: false,
        },
    ];
    screen.finish(6.0)
}

/// The Flag mark tool: a pole planted at the anchor, a flag at its top.
fn flag(at: Pos2) -> Screen {
    let (pole, flag) = shapes::flag(at);
    let mut screen = Screen::empty();
    screen.lines.push(pole.to_vec());
    screen.solids.push(flag);
    screen.handles = vec![anchor_handle(at)];
    screen
}

/// The Pin tool: a pin planted at the anchor. Its note is the `popup`, laid out whether or not it
/// is open, so clicking the pin has nothing to measure first.
fn pin(style: &DrawingStyle, text: &str, proj: &Projection, at: Pos2) -> Screen {
    let (drop, head) = shapes::pin(at);
    // The note fits its text, and wraps only past the width the style allows.
    let width = if style.wrap { Width::UpTo } else { Width::Text };
    let size = dimensions(proj, text, style, width);
    let rect = shapes::pin_note_rect(at, size, proj.plot);
    // Its tail reaches towards the pin: the top of the head when the note is above it, the
    // point when the note had to open below.
    let tip = if rect.bottom() < at.y {
        pos2(at.x, head.y - shapes::PIN_RADIUS - 1.0)
    } else {
        pos2(at.x, at.y + 1.0)
    };
    let mut screen = Screen::empty();
    screen.solids.push(drop);
    screen.studs.push((head, STUD));
    screen.popup = Some(Popup {
        rect,
        outline: shapes::bubble(rect, BUBBLE_CORNER, Some(tip), shapes::PIN_NOTE_TAIL),
    });
    screen.text = Some(TextPlace::Inside(rect));
    screen.handles = vec![anchor_handle(at)];
    screen
}
