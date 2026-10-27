//! Pine chart output types: the data structs `PineRuntimeState` holds and the view renders,
//! ported from `Pine/Model/*.swift`.

// `display.*`: where a plot is shown, as a bit mask.
pub const DISPLAY_PANE: u32 = 1;
pub const DISPLAY_DATA_WINDOW: u32 = 2;
pub const DISPLAY_PRICE_SCALE: u32 = 4;
pub const DISPLAY_STATUS_LINE: u32 = 8;
pub const DISPLAY_HIDDEN: u32 = 0;
pub const DISPLAY_ALL: u32 =
    DISPLAY_PANE | DISPLAY_DATA_WINDOW | DISPLAY_PRICE_SCALE | DISPLAY_STATUS_LINE;

/// Drawings can be made with `na` coordinates and given real ones later. A missing bar index is
/// `MISSING_INDEX`, a missing price NaN.
pub const MISSING_INDEX: i64 = i64::MIN;

pub fn index_is_known(index: i64) -> bool {
    index != MISSING_INDEX
}

pub fn price_is_known(price: f64) -> bool {
    price.is_finite()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlotStyle {
    Line,
    Stepline,
    Histogram,
    Columns,
    Area,
    Circles,
    Cross,
    /// A step line with a diamond wherever the value changes.
    SteplineDiamond,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    Auto,
    Tiny,
    Small,
    Normal,
    Large,
    Huge,
    /// A text size given as a number: TradingView's point size, where `size.normal` is 14.
    Points(u16),
}

impl Size {
    /// The sizes `size.*` names.
    pub const NAMED: [Size; 6] = [
        Self::Auto,
        Self::Tiny,
        Self::Small,
        Self::Normal,
        Self::Large,
        Self::Huge,
    ];

    /// The `size.*` constant, or None for a size given as a number.
    pub fn pine_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => "size.auto",
            Self::Tiny => "size.tiny",
            Self::Small => "size.small",
            Self::Normal => "size.normal",
            Self::Large => "size.large",
            Self::Huge => "size.huge",
            Self::Points(_) => return None,
        })
    }
}

/// `text.align_*`: how a label's lines, or a box's text, are placed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
    Top,
    Bottom,
}

impl TextAlign {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::Left => "text.align_left",
            Self::Center => "text.align_center",
            Self::Right => "text.align_right",
            Self::Top => "text.align_top",
            Self::Bottom => "text.align_bottom",
        }
    }
}

/// `yloc.*`: where a label's y comes from. `price` is its own `y`; above or below the bar it sits at
/// the bar's high or low and `y` is ignored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum YLocation {
    #[default]
    Price,
    AboveBar,
    BelowBar,
}

impl YLocation {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::Price => "yloc.price",
            Self::AboveBar => "yloc.abovebar",
            Self::BelowBar => "yloc.belowbar",
        }
    }
}

/// `extend.*`: how far a line or box runs on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LineExtend {
    #[default]
    None,
    Left,
    Right,
    Both,
}

impl LineExtend {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::None => "extend.none",
            Self::Left => "extend.left",
            Self::Right => "extend.right",
            Self::Both => "extend.both",
        }
    }

    pub fn extends_left(self) -> bool {
        matches!(self, Self::Left | Self::Both)
    }

    pub fn extends_right(self) -> bool {
        matches!(self, Self::Right | Self::Both)
    }
}

/// `line.style_*` / `label.style_*` drawing style.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LineStyle {
    #[default]
    Solid,
    Dotted,
    Dashed,
    ArrowLeft,
    ArrowRight,
    ArrowBoth,
}

impl LineStyle {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::Solid => "line.style_solid",
            Self::Dotted => "line.style_dotted",
            Self::Dashed => "line.style_dashed",
            Self::ArrowLeft => "line.style_arrow_left",
            Self::ArrowRight => "line.style_arrow_right",
            Self::ArrowBoth => "line.style_arrow_both",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerKind {
    Shape,
    Character,
}

/// `shape.*`: the marker glyph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerShape {
    Xcross,
    Cross,
    Circle,
    TriangleUp,
    TriangleDown,
    Flag,
    ArrowUp,
    ArrowDown,
    LabelUp,
    LabelDown,
    Square,
    Diamond,
}

impl MarkerShape {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::Xcross => "shape.xcross",
            Self::Cross => "shape.cross",
            Self::Circle => "shape.circle",
            Self::TriangleUp => "shape.triangleup",
            Self::TriangleDown => "shape.triangledown",
            Self::Flag => "shape.flag",
            Self::ArrowUp => "shape.arrowup",
            Self::ArrowDown => "shape.arrowdown",
            Self::LabelUp => "shape.labelup",
            Self::LabelDown => "shape.labeldown",
            Self::Square => "shape.square",
            Self::Diamond => "shape.diamond",
        }
    }

    pub fn points_down(self) -> bool {
        matches!(self, Self::TriangleDown | Self::ArrowDown | Self::LabelDown)
    }
}

/// `location.*`: where a marker sits relative to its bar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerLocation {
    AboveBar,
    BelowBar,
    Top,
    Bottom,
    Absolute,
}

impl MarkerLocation {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::AboveBar => "location.abovebar",
            Self::BelowBar => "location.belowbar",
            Self::Top => "location.top",
            Self::Bottom => "location.bottom",
            Self::Absolute => "location.absolute",
        }
    }

    pub fn is_below(self) -> bool {
        matches!(self, Self::BelowBar | Self::Bottom)
    }
}

/// `label.style_*`: how a label's text and marker are drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelStyle {
    None,
    LabelDown,
    LabelUp,
    LabelLeft,
    LabelRight,
    LabelCenter,
    LabelLowerLeft,
    LabelLowerRight,
    LabelUpperLeft,
    LabelUpperRight,
    Circle,
    Square,
    Diamond,
    Cross,
    Xcross,
    Flag,
    TriangleUp,
    TriangleDown,
    ArrowUp,
    ArrowDown,
    TextOutline,
}

impl LabelStyle {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::None => "label.style_none",
            Self::LabelDown => "label.style_label_down",
            Self::LabelUp => "label.style_label_up",
            Self::LabelLeft => "label.style_label_left",
            Self::LabelRight => "label.style_label_right",
            Self::LabelCenter => "label.style_label_center",
            Self::LabelLowerLeft => "label.style_label_lower_left",
            Self::LabelLowerRight => "label.style_label_lower_right",
            Self::LabelUpperLeft => "label.style_label_upper_left",
            Self::LabelUpperRight => "label.style_label_upper_right",
            Self::Circle => "label.style_circle",
            Self::Square => "label.style_square",
            Self::Diamond => "label.style_diamond",
            Self::Cross => "label.style_cross",
            Self::Xcross => "label.style_xcross",
            Self::Flag => "label.style_flag",
            Self::TriangleUp => "label.style_triangleup",
            Self::TriangleDown => "label.style_triangledown",
            Self::ArrowUp => "label.style_arrowup",
            Self::ArrowDown => "label.style_arrowdown",
            Self::TextOutline => "label.style_text_outline",
        }
    }
}

/// `position.*`: where a table sits on the plot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TablePosition {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl TablePosition {
    pub fn pine_name(self) -> &'static str {
        match self {
            Self::TopLeft => "position.top_left",
            Self::TopCenter => "position.top_center",
            Self::TopRight => "position.top_right",
            Self::MiddleLeft => "position.middle_left",
            Self::MiddleCenter => "position.middle_center",
            Self::MiddleRight => "position.middle_right",
            Self::BottomLeft => "position.bottom_left",
            Self::BottomCenter => "position.bottom_center",
            Self::BottomRight => "position.bottom_right",
        }
    }

    pub fn vertical(self) -> Vertical {
        match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => Vertical::Top,
            Self::MiddleLeft | Self::MiddleCenter | Self::MiddleRight => Vertical::Middle,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => Vertical::Bottom,
        }
    }

    pub fn horizontal(self) -> Horizontal {
        match self {
            Self::TopLeft | Self::MiddleLeft | Self::BottomLeft => Horizontal::Left,
            Self::TopCenter | Self::MiddleCenter | Self::BottomCenter => Horizontal::Center,
            Self::TopRight | Self::MiddleRight | Self::BottomRight => Horizontal::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Vertical {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Horizontal {
    Left,
    Center,
    Right,
}

// MARK: Output structs

#[derive(Clone, Debug, PartialEq)]
pub struct PlotOutput {
    pub id: usize,
    pub title: Option<String>,
    pub values: Vec<Option<f64>>,
    pub color: u32,
    pub line_width: usize,
    pub style: PlotStyle,
    pub colors: Vec<Option<u32>>,
    pub display: u32,
    pub hist_base: f64,
    /// `plot(offset = n)`: bars the plot is shifted by (positive moves it later). Applied when the
    /// run's output is read; values pushed past the last bar have no bar to sit on.
    pub offset: i64,
    /// `force_overlay`: draw on the main chart even when the script has its own pane.
    pub force_overlay: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineOutput {
    pub id: usize,
    pub x1: i64,
    pub y1: f64,
    pub x2: i64,
    pub y2: f64,
    pub color: u32,
    pub width: usize,
    pub style: LineStyle,
    pub extend: LineExtend,
    pub time_anchored: bool,
}

impl LineOutput {
    pub fn is_complete(&self) -> bool {
        index_is_known(self.x1)
            && index_is_known(self.x2)
            && price_is_known(self.y1)
            && price_is_known(self.y2)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelOutput {
    pub id: usize,
    pub x: i64,
    pub y: f64,
    pub text: String,
    pub color: Option<u32>,
    pub text_color: u32,
    pub style: LabelStyle,
    pub size: Size,
    pub tooltip: Option<String>,
    pub text_align: TextAlign,
    pub yloc: YLocation,
    pub time_anchored: bool,
}

impl LabelOutput {
    /// A label placed above or below its bar needs no price.
    pub fn is_complete(&self) -> bool {
        index_is_known(self.x) && (self.yloc != YLocation::Price || price_is_known(self.y))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxOutput {
    pub id: usize,
    pub left: i64,
    pub top: f64,
    pub right: i64,
    pub bottom: f64,
    pub border_color: Option<u32>,
    pub border_width: usize,
    pub background_color: Option<u32>,
    pub border_style: LineStyle,
    pub extend: LineExtend,
    pub text: String,
    pub text_color: u32,
    pub text_size: Size,
    pub text_horizontal_align: TextAlign,
    pub text_vertical_align: TextAlign,
    pub time_anchored: bool,
}

impl BoxOutput {
    pub fn is_complete(&self) -> bool {
        index_is_known(self.left)
            && index_is_known(self.right)
            && price_is_known(self.top)
            && price_is_known(self.bottom)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableOutput {
    pub id: usize,
    pub position: TablePosition,
    pub columns: usize,
    pub rows: usize,
    pub background_color: Option<u32>,
    pub border_color: Option<u32>,
    pub border_width: usize,
    pub frame_color: Option<u32>,
    pub frame_width: usize,
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableCell {
    pub column: usize,
    pub row: usize,
    pub text: String,
    pub text_color: u32,
    pub background_color: Option<u32>,
    pub text_size: Size,
    pub column_span: usize,
    pub row_span: usize,
    pub text_horizontal_align: TextAlign,
    pub text_vertical_align: TextAlign,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorOutput {
    pub id: usize,
    pub title: Option<String>,
    pub colors: Vec<Option<u32>>,
    /// `offset`: bars the colours are shifted by (positive moves them later). Applied when the
    /// run's output is read.
    pub offset: i64,
    /// `show_last`: only the last this many bars keep their colour.
    pub show_last: Option<usize>,
}

impl ColorOutput {
    /// The colours with `offset` and `show_last` applied. A bar left without a colour (`na`)
    /// keeps its default look.
    pub fn finished(mut self) -> Self {
        let offset = std::mem::take(&mut self.offset);
        if offset != 0 {
            let length = self.colors.len() as i64;
            let mut shifted = vec![None; self.colors.len()];
            for (index, color) in self.colors.iter().enumerate() {
                let target = index as i64 + offset;
                if (0..length).contains(&target) {
                    shifted[target as usize] = *color;
                }
            }
            self.colors = shifted;
        }
        if let Some(last) = self.show_last.take() {
            let hidden = self.colors.len().saturating_sub(last);
            for color in &mut self.colors[..hidden] {
                *color = None;
            }
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HorizontalLine {
    pub id: usize,
    pub value: f64,
    pub color: u32,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandleOutput {
    pub id: usize,
    pub title: Option<String>,
    pub bars: Vec<Option<CandleBar>>,
    pub display: u32,
    /// `plotbar()`: OHLC bars (a stem with open and close ticks) rather than candles.
    pub bar_style: bool,
    /// `editable = false` keeps the output out of the Style settings.
    pub editable: bool,
    /// `show_last`: only the last this many bars are drawn.
    pub show_last: Option<usize>,
    pub force_overlay: bool,
}

impl CandleOutput {
    /// The bars with `show_last` applied.
    pub fn finished(mut self) -> Self {
        if let Some(last) = self.show_last.take() {
            let hidden = self.bars.len().saturating_sub(last);
            for bar in &mut self.bars[..hidden] {
                *bar = None;
            }
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandleBar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub color: Option<u32>,
    pub wick_color: Option<u32>,
    pub border_color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinefillOutput {
    pub id: usize,
    pub line1: usize,
    pub line2: usize,
    pub color: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillGradient {
    pub top: f64,
    pub bottom: f64,
    pub top_color: u32,
    pub bottom_color: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FillOutput {
    pub id: usize,
    pub plot_a: usize,
    pub plot_b: usize,
    pub colors: Vec<Option<u32>>,
    pub title: Option<String>,
    pub gradients: Vec<Option<FillGradient>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerOutput {
    pub id: usize,
    pub kind: MarkerKind,
    pub title: Option<String>,
    pub values: Vec<bool>,
    pub character: Option<String>,
    pub color: u32,
    pub location: MarkerLocation,
    pub style: MarkerShape,
    pub prices: Vec<Option<f64>>,
    pub colors: Vec<Option<u32>>,
    pub size: Size,
    pub display: u32,
    pub force_overlay: bool,
}

/// `plotarrow()`: an up arrow below the bar for each positive value, a down arrow above it for each
/// negative one, longer the larger the value.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowOutput {
    pub id: usize,
    pub title: Option<String>,
    pub values: Vec<Option<f64>>,
    pub up_colors: Vec<Option<u32>>,
    pub down_colors: Vec<Option<u32>>,
    /// Bars the arrows are shifted by (positive moves them later). Applied when the run's output
    /// is read; an arrow pushed past the last bar has no bar to sit on.
    pub offset: i64,
    /// `show_last`: only the last this many bars keep their arrow.
    pub show_last: Option<usize>,
    /// The shortest and the longest arrow, in pixels.
    pub min_height: f64,
    pub max_height: f64,
    pub display: u32,
    pub force_overlay: bool,
}

impl ArrowOutput {
    /// The arrows with `offset` and then `show_last` applied.
    pub fn finished(mut self) -> Self {
        let offset = std::mem::take(&mut self.offset);
        let length = self.values.len();
        if offset != 0 {
            let place = |index: usize| {
                let target = index as i64 + offset;
                (0..length as i64)
                    .contains(&target)
                    .then_some(target as usize)
            };
            let mut values = vec![None; length];
            let mut up = vec![None; self.up_colors.len()];
            let mut down = vec![None; self.down_colors.len()];
            for index in 0..length {
                let Some(target) = place(index) else {
                    continue;
                };
                values[target] = self.values[index];
                if let (Some(slot), Some(color)) = (up.get_mut(target), self.up_colors.get(index)) {
                    *slot = *color;
                }
                if let (Some(slot), Some(color)) =
                    (down.get_mut(target), self.down_colors.get(index))
                {
                    *slot = *color;
                }
            }
            self.values = values;
            self.up_colors = up;
            self.down_colors = down;
        }
        if let Some(last) = self.show_last.take() {
            let hidden = length.saturating_sub(last);
            for value in self.values.iter_mut().take(hidden) {
                *value = None;
            }
        }
        self
    }

    /// The longest absolute value over the whole series, which `max_height` belongs to.
    pub fn largest(&self) -> f64 {
        self.values
            .iter()
            .flatten()
            .fold(0.0, |largest, value| largest.max(value.abs()))
    }

    /// The arrow's length in pixels for `value`: `min_height` for the smallest, `max_height` at
    /// `largest`, in proportion between.
    pub fn height(&self, value: f64, largest: f64) -> f64 {
        if largest <= 0.0 {
            return self.min_height;
        }
        self.min_height + (self.max_height - self.min_height) * (value.abs() / largest)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolylineOutput {
    pub id: usize,
    pub points: Vec<PolylinePoint>,
    pub closed: bool,
    pub line_color: Option<u32>,
    pub fill_color: Option<u32>,
    pub style: LineStyle,
    pub width: usize,
    pub curved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolylinePoint {
    pub index: i64,
    pub price: f64,
}

impl PlotOutput {
    /// The plot with `offset` applied: the value of bar `i` shown on bar `i + offset`.
    pub fn shifted(mut self) -> Self {
        let offset = std::mem::take(&mut self.offset);
        if offset == 0 {
            return self;
        }
        let length = self.values.len() as i64;
        let place = |index: usize| {
            let target = index as i64 + offset;
            (0..length).contains(&target).then_some(target as usize)
        };
        let mut values = vec![None; self.values.len()];
        let mut colors = vec![None; self.colors.len()];
        for (index, value) in self.values.iter().enumerate() {
            if let Some(target) = place(index) {
                values[target] = *value;
            }
        }
        for (index, color) in self.colors.iter().enumerate() {
            if let Some(target) = place(index)
                && target < colors.len()
            {
                colors[target] = *color;
            }
        }
        self.values = values;
        self.colors = colors;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_and_extend_helpers() {
        assert!(MarkerShape::TriangleDown.points_down());
        assert!(!MarkerShape::Circle.points_down());
        assert!(MarkerLocation::BelowBar.is_below());
        assert!(!MarkerLocation::AboveBar.is_below());
        assert!(LineExtend::Both.extends_left() && LineExtend::Both.extends_right());
        assert!(!LineExtend::Right.extends_left());
    }

    #[test]
    fn table_position_parts() {
        assert_eq!(TablePosition::TopLeft.vertical(), Vertical::Top);
        assert_eq!(TablePosition::TopLeft.horizontal(), Horizontal::Left);
        assert_eq!(TablePosition::MiddleCenter.vertical(), Vertical::Middle);
        assert_eq!(TablePosition::BottomRight.horizontal(), Horizontal::Right);
    }

    #[test]
    fn drawing_completeness() {
        let line = LineOutput {
            id: 0,
            x1: 1,
            y1: 10.0,
            x2: MISSING_INDEX,
            y2: f64::NAN,
            color: 0,
            width: 1,
            style: LineStyle::Solid,
            extend: LineExtend::None,
            time_anchored: false,
        };
        assert!(!line.is_complete());
    }

    fn arrows(values: &[Option<f64>]) -> ArrowOutput {
        ArrowOutput {
            id: 0,
            title: None,
            values: values.to_vec(),
            up_colors: vec![Some(1); values.len()],
            down_colors: vec![Some(2); values.len()],
            offset: 0,
            show_last: None,
            min_height: 5.0,
            max_height: 100.0,
            display: DISPLAY_ALL,
            force_overlay: false,
        }
    }

    #[test]
    fn an_arrows_length_is_in_proportion_to_the_largest_value() {
        let output = arrows(&[Some(2.0), Some(-8.0), None, Some(4.0)]);
        let largest = output.largest();
        assert_eq!(largest, 8.0);
        assert_eq!(output.height(8.0, largest), 100.0);
        assert_eq!(
            output.height(-8.0, largest),
            100.0,
            "the sign does not matter"
        );
        assert_eq!(output.height(0.0, largest), 5.0);
        assert_eq!(output.height(4.0, largest), 52.5);
        assert_eq!(
            output.height(1.0, 0.0),
            5.0,
            "no non-zero value: the minimum"
        );
    }

    #[test]
    fn arrows_are_shifted_by_their_offset_and_trimmed_to_the_last_bars() {
        let mut output = arrows(&[Some(1.0), Some(-2.0), Some(3.0), Some(4.0)]);
        output.up_colors = vec![Some(10), Some(11), Some(12), Some(13)];
        output.offset = 1;
        let shifted = output.clone().finished();
        assert_eq!(shifted.values, [None, Some(1.0), Some(-2.0), Some(3.0)]);
        assert_eq!(shifted.up_colors, [None, Some(10), Some(11), Some(12)]);
        assert_eq!(shifted.offset, 0, "applied once");
        output.offset = -1;
        assert_eq!(
            output.clone().finished().values,
            [Some(-2.0), Some(3.0), Some(4.0), None]
        );
        output.offset = 0;
        output.show_last = Some(2);
        assert_eq!(output.finished().values, [None, None, Some(3.0), Some(4.0)]);
    }
}
