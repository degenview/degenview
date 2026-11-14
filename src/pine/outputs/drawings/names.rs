//! The Pine constants a drawing takes by name (`line.style_dashed`, `size.small`,
//! `position.top_right`, …) and text sizes given as a constant or in points.

use super::{FromName, parse};
use crate::pine::outputs::{
    LabelStyle, LineExtend, LineStyle, Size, TablePosition, TextAlign, YLocation,
};
use crate::pine::runtime::value::RuntimeValue;

/// A text size given as a `size.*` constant or as a number of points (0 is `size.auto`); `fallback`
/// when the script passed nothing or something that is neither.
pub(super) fn size_of(value: Option<&RuntimeValue>, fallback: Size) -> Size {
    match value {
        Some(RuntimeValue::Int(0)) => Size::Auto,
        Some(RuntimeValue::Int(points)) if *points > 0 => {
            Size::Points(u16::try_from(*points).unwrap_or(u16::MAX))
        }
        Some(value) => parse(value.text_value(), fallback, fallback),
        None => fallback,
    }
}

impl FromName for LineStyle {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "line.style_solid" => Some(Self::Solid),
            "line.style_dotted" => Some(Self::Dotted),
            "line.style_dashed" => Some(Self::Dashed),
            "line.style_arrow_left" => Some(Self::ArrowLeft),
            "line.style_arrow_right" => Some(Self::ArrowRight),
            "line.style_arrow_both" => Some(Self::ArrowBoth),
            _ => None,
        }
    }
}

impl FromName for LineExtend {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "extend.none" => Some(Self::None),
            "extend.left" => Some(Self::Left),
            "extend.right" => Some(Self::Right),
            "extend.both" => Some(Self::Both),
            _ => None,
        }
    }
}

impl FromName for LabelStyle {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "label.style_none" => Some(Self::None),
            "label.style_label_down" => Some(Self::LabelDown),
            "label.style_label_up" => Some(Self::LabelUp),
            "label.style_label_left" => Some(Self::LabelLeft),
            "label.style_label_right" => Some(Self::LabelRight),
            "label.style_label_center" => Some(Self::LabelCenter),
            "label.style_label_lower_left" => Some(Self::LabelLowerLeft),
            "label.style_label_lower_right" => Some(Self::LabelLowerRight),
            "label.style_label_upper_left" => Some(Self::LabelUpperLeft),
            "label.style_label_upper_right" => Some(Self::LabelUpperRight),
            "label.style_circle" => Some(Self::Circle),
            "label.style_square" => Some(Self::Square),
            "label.style_diamond" => Some(Self::Diamond),
            "label.style_cross" => Some(Self::Cross),
            "label.style_xcross" => Some(Self::Xcross),
            "label.style_flag" => Some(Self::Flag),
            "label.style_triangleup" => Some(Self::TriangleUp),
            "label.style_triangledown" => Some(Self::TriangleDown),
            "label.style_arrowup" => Some(Self::ArrowUp),
            "label.style_arrowdown" => Some(Self::ArrowDown),
            "label.style_text_outline" => Some(Self::TextOutline),
            _ => None,
        }
    }
}

impl FromName for Size {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "size.auto" => Some(Self::Auto),
            "size.tiny" => Some(Self::Tiny),
            "size.small" => Some(Self::Small),
            "size.normal" => Some(Self::Normal),
            "size.large" => Some(Self::Large),
            "size.huge" => Some(Self::Huge),
            _ => None,
        }
    }
}

impl FromName for TextAlign {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "text.align_left" => Some(Self::Left),
            "text.align_center" => Some(Self::Center),
            "text.align_right" => Some(Self::Right),
            "text.align_top" => Some(Self::Top),
            "text.align_bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

impl FromName for YLocation {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "yloc.price" => Some(Self::Price),
            "yloc.abovebar" => Some(Self::AboveBar),
            "yloc.belowbar" => Some(Self::BelowBar),
            _ => None,
        }
    }
}

impl FromName for TablePosition {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "position.top_left" => Some(Self::TopLeft),
            "position.top_center" => Some(Self::TopCenter),
            "position.top_right" => Some(Self::TopRight),
            "position.middle_left" => Some(Self::MiddleLeft),
            "position.middle_center" => Some(Self::MiddleCenter),
            "position.middle_right" => Some(Self::MiddleRight),
            "position.bottom_left" => Some(Self::BottomLeft),
            "position.bottom_center" => Some(Self::BottomCenter),
            "position.bottom_right" => Some(Self::BottomRight),
            _ => None,
        }
    }
}
