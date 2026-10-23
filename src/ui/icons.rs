//! Painter-drawn glyphs standing in for the SF Symbols the Swift app uses. Every icon is drawn
//! on a 16×16 grid, scaled to the target rect and tinted with one colour, so they stay crisp at
//! any DPI and need no font or image loader.

use eframe::egui::{
    Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, epaint::PathShape, vec2,
};

/// The glyph set the Script Manager uses; a few (eye, braces, bell) wait for its phase-2 panels.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    SidebarLeft,
    SidebarRight,
    ChartLine,
    ChartLineFlat,
    ChartUptrend,
    RectLeft,
    RectTop,
    RectBottom,
    Star,
    StarFill,
    Plus,
    Magnifier,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    ChevronLeft,
    Maximize,
    Restore,
    ChevronUpDown,
    Eye,
    EyeSlash,
    Curlybraces,
    Check,
    CheckCircleFill,
    XmarkOctagonFill,
    Triangle,
    TriangleFill,
    XmarkCircleFill,
    Xmark,
    Sliders,
    CircleDashed,
    Key,
    Books,
    ArrowUpRightSquare,
    CopyDoc,
    Clipboard,
    Bell,
    BellBadge,
    BellFill,
    Clock,
    Download,
    Switch,
    Calendar,
    Hash,
    TextFormat,
    List,
    Palette,
    BarChart,
    Waveform,
    Cycle,
    Bands,
    Circle,
    BitcoinCircle,
    BuildingColumns,
    Percent,
    Gauge,
    GaugeHigh,
    Sparkles,
    Lightbulb,
    ClockArrow,
    Flame,
    Diamond,
    ArrowDownRightCircle,
    CheckmarkSeal,
    PersonCircle,
    SportsCourt,
    CloudSun,
    Briefcase,
    PieChart,
    Layers,
    Exchange,
    Banknote,
    ArrowUpRight,
    ArrowDownRight,
    ArrowUp,
    ArrowDown,
    Pencil,
    PauseFill,
    PlayFill,
    Ellipsis,
    Repeat,
    OneCircle,
    Trash,
    BellSlash,
    PauseCircle,
    CheckCircle,
    ClockBadge,
    LineDiagonal,
    LineTrend,
    FibPoints,
    Paintbrush,
    SunMaxFill,
    MoonFill,
    CircleLeftHalfFill,
    Ruler,
    ArrowLeftRight,
    SpeakerWave,
    MacWindow,
    InfoCircle,
    LockFill,
    SplitColumns,
    Tag,
    Bolt,
    BoltSlash,
    Link,
    Camera,
    /// A price axis: its line and the ticks along it.
    PriceScale,
    /// The X (formerly Twitter) mark.
    XLogo,
    ChainLinked,
    ChainBroken,
    DocText,
    ShareUp,
    Minus,
    RectFill,
    RectDashed,
    Undo,
    Highlighter,
    ArrowMarkerTool,
    ArrowTool,
    ArrowMarkUp,
    ArrowMarkDown,
    ShapeRectangle,
    ShapeRotatedRect,
    ShapePath,
    ShapeCircle,
    ShapeEllipse,
    ShapePolyline,
    ShapeTriangle,
    ShapeArc,
    ShapeCurve,
    ShapeDoubleCurve,
    LineRay,
    LineInfo,
    LineExtended,
    LineAngle,
    LineHorizontal,
    LineHorizontalRay,
    LineVertical,
    LineCross,
    ChannelParallel,
    ChannelRegression,
    ChannelFlat,
    ChannelDisjoint,
    ForkAndrews,
    ForkSchiff,
    ForkModifiedSchiff,
    ForkInside,
    PatternXabcd,
    PatternCypher,
    PatternHeadShoulders,
    PatternAbcd,
    PatternTriangle,
    PatternThreeDrives,
    WaveImpulse,
    WaveCorrection,
    WaveTriangle,
    WaveDouble,
    WaveTriple,
    CycleLines,
    CycleArcs,
    CycleSine,
    Magnet,
    KeepDrawing,
    LockOpen,
    BringForward,
    SendBackward,
    TextTool,
    NoteTool,
    PriceNoteTool,
    PinTool,
    TableTool,
    CalloutTool,
    CommentTool,
    PriceLabelTool,
    SignpostTool,
    FlagTool,
    ImageTool,
}

/// Stroke width on the 16 grid.
const W: f32 = 1.35;

/// Draws `icon` centred in `rect`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let scale = side / 16.0;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let p = |x: f32, y: f32| origin + vec2(x, y) * scale;
    let stroke = Stroke::new((W * scale).max(1.0), color);
    let line = |points: &[Pos2]| {
        painter.add(Shape::Path(PathShape::line(points.to_vec(), stroke)));
    };
    let closed = |points: &[Pos2]| {
        painter.add(Shape::Path(PathShape::closed_line(points.to_vec(), stroke)));
    };
    let fill_poly = |points: Vec<Pos2>| {
        // Fan from the centroid so concave star-shaped polygons fill without artifacts.
        let centre = points.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / points.len() as f32;
        let centre = centre.to_pos2();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(centre, color);
        for point in &points {
            mesh.colored_vertex(*point, color);
        }
        let n = points.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        painter.add(Shape::mesh(mesh));
    };
    let round = |r: f32| CornerRadius::same((r * scale).round().clamp(0.0, 255.0) as u8);
    let rect_of = |a: Pos2, b: Pos2| Rect::from_two_pos(a, b);
    let circle = |c: Pos2, r: f32, fill: bool| {
        if fill {
            painter.circle_filled(c, r * scale, color);
        } else {
            painter.circle_stroke(c, r * scale, stroke);
        }
    };
    let star_points = |outer: f32, inner: f32| -> Vec<Pos2> {
        (0..10)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                let r = if i % 2 == 0 { outer } else { inner };
                p(8.0 + r * angle.cos(), 8.6 + r * angle.sin())
            })
            .collect()
    };

    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        let steps = (((to - from).abs() / 12.0).ceil() as usize).max(4);
        (0..=steps)
            .map(|k| {
                let a = (from + (to - from) * k as f32 / steps as f32).to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    // A rounded bar of half-length `half` and radius `r` about (`cx`, `cy`), turned `theta` degrees.
    let capsule = |cx: f32, cy: f32, half: f32, r: f32, theta: f32| -> Vec<Pos2> {
        let (dx, dy) = (
            theta.to_radians().cos() * half,
            theta.to_radians().sin() * half,
        );
        let mut points = arc(cx + dx, cy + dy, r, theta - 90.0, theta + 90.0);
        points.extend(arc(cx - dx, cy - dy, r, theta + 90.0, theta + 270.0));
        points
    };
    let sparkle = |cx: f32, cy: f32, r: f32| {
        let points = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = if i % 2 == 0 { r } else { r * 0.3 };
                p(cx + d * a.cos(), cy + d * a.sin())
            })
            .collect();
        fill_poly(points);
    };
    // A character of a wave's name, drawn as strokes in a box `h` tall with its top-left at
    // (`x`, `y`): crisp at any size, with no font to load.
    let glyph = |ch: char, x: f32, y: f32, h: f32| {
        let k = h / 5.0;
        let strokes: &[&[(f32, f32)]] = match ch {
            '1' => &[&[(0.4, 1.2), (1.6, 0.0), (1.6, 5.0)]],
            '5' => &[&[
                (2.7, 0.0),
                (0.5, 0.0),
                (0.3, 2.3),
                (1.5, 2.0),
                (2.5, 2.8),
                (2.7, 4.2),
                (1.6, 5.0),
                (0.0, 4.5),
            ]],
            'A' => &[
                &[(0.0, 5.0), (1.4, 0.0), (2.8, 5.0)],
                &[(0.5, 3.4), (2.3, 3.4)],
            ],
            'C' => &[&[
                (2.7, 0.9),
                (1.7, 0.0),
                (0.7, 0.3),
                (0.0, 2.5),
                (0.7, 4.7),
                (1.7, 5.0),
                (2.7, 4.1),
            ]],
            'E' => &[
                &[(2.7, 0.0), (0.0, 0.0), (0.0, 5.0), (2.7, 5.0)],
                &[(0.0, 2.5), (2.1, 2.5)],
            ],
            'W' => &[&[(0.0, 0.0), (0.8, 5.0), (1.6, 1.8), (2.4, 5.0), (3.2, 0.0)]],
            'Y' => &[
                &[(0.0, 0.0), (1.5, 2.6), (3.0, 0.0)],
                &[(1.5, 2.6), (1.5, 5.0)],
            ],
            'Z' => &[&[(0.0, 0.0), (2.8, 0.0), (0.0, 5.0), (2.8, 5.0)]],
            _ => &[],
        };
        let thin = Stroke::new((W * 0.8 * scale).max(1.0), color);
        for points in strokes {
            let path: Vec<Pos2> = points
                .iter()
                .map(|(dx, dy)| p(x + dx * k, y + dy * k))
                .collect();
            painter.add(Shape::Path(PathShape::line(path, thin)));
        }
    };

    match icon {
        Icon::Briefcase => {
            painter.rect_stroke(
                rect_of(p(1.5, 5.0), p(14.5, 13.5)),
                round(1.8),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(5.3, 5.0), p(5.3, 2.7), p(10.7, 2.7), p(10.7, 5.0)]);
            line(&[p(1.8, 8.3), p(6.7, 9.4), p(9.3, 9.4), p(14.2, 8.3)]);
        }
        Icon::PieChart => {
            line(&arc(7.6, 8.3, 5.6, -64.0, 296.0));
            line(&[p(8.0, 8.0), p(8.0, 2.6)]);
            line(&[p(8.0, 8.0), p(13.3, 8.0)]);
        }
        Icon::Layers => {
            for (left, top, right, bottom) in [(3.0, 2.0, 12.5, 10.0), (1.5, 5.0, 11.0, 13.0)] {
                painter.rect_stroke(
                    rect_of(p(left, top), p(right, bottom)),
                    round(1.5),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Exchange => {
            line(&[p(2.0, 5.0), p(13.3, 5.0), p(10.6, 2.4)]);
            line(&[p(13.3, 5.0), p(10.6, 7.6)]);
            line(&[p(14.0, 11.0), p(2.7, 11.0), p(5.4, 8.4)]);
            line(&[p(2.7, 11.0), p(5.4, 13.6)]);
        }
        Icon::Banknote => {
            painter.rect_stroke(
                rect_of(p(1.2, 3.5), p(14.8, 12.5)),
                round(1.5),
                stroke,
                StrokeKind::Middle,
            );
            circle(p(8.0, 8.0), 2.5, false);
            line(&[p(3.0, 6.0), p(4.0, 6.0)]);
            line(&[p(12.0, 10.0), p(13.0, 10.0)]);
        }
        Icon::SidebarRight => {
            painter.rect_stroke(
                rect_of(p(1.5, 2.5), p(14.5, 13.5)),
                round(2.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(10.0, 2.8), p(10.0, 13.2)]);
        }
        Icon::SidebarLeft => {
            painter.rect_stroke(
                rect_of(p(1.5, 2.5), p(14.5, 13.5)),
                round(2.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(6.0, 2.8), p(6.0, 13.2)]);
        }
        Icon::ChartLine | Icon::ChartLineFlat => {
            line(&[p(2.0, 2.0), p(2.0, 14.0), p(14.0, 14.0)]);
            if icon == Icon::ChartLine {
                line(&[p(4.5, 10.5), p(7.0, 7.0), p(9.5, 9.0), p(13.0, 4.0)]);
            } else {
                line(&[p(4.5, 9.0), p(13.0, 9.0)]);
            }
        }
        Icon::ChartUptrend => {
            line(&[p(2.0, 2.0), p(2.0, 14.0), p(14.0, 14.0)]);
            line(&[p(4.5, 11.0), p(8.0, 7.5), p(10.0, 9.5), p(13.5, 5.0)]);
            line(&[p(11.0, 4.8), p(13.5, 5.0), p(13.3, 7.5)]);
        }
        Icon::RectLeft | Icon::RectTop | Icon::RectBottom => {
            let outer = rect_of(p(1.5, 2.5), p(14.5, 13.5));
            let half = match icon {
                Icon::RectLeft => rect_of(p(1.5, 2.5), p(8.0, 13.5)),
                Icon::RectTop => rect_of(p(1.5, 2.5), p(14.5, 8.0)),
                _ => rect_of(p(1.5, 8.0), p(14.5, 13.5)),
            };
            painter.rect_filled(
                half.shrink(0.6 * scale),
                round(1.8),
                color.gamma_multiply(0.55),
            );
            painter.rect_stroke(outer, round(2.5), stroke, StrokeKind::Middle);
        }
        Icon::Star => closed(&star_points(6.6, 3.0)),
        Icon::StarFill => fill_poly(star_points(6.9, 3.2)),
        Icon::Minus => line(&[p(3.0, 8.0), p(13.0, 8.0)]),
        Icon::RectFill => {
            painter.rect_filled(rect_of(p(2.0, 3.5), p(14.0, 12.5)), round(1.8), color);
        }
        Icon::RectDashed => {
            for (a, b) in [
                ((2.0, 3.5), (5.6, 3.5)),
                ((7.0, 3.5), (9.0, 3.5)),
                ((10.4, 3.5), (14.0, 3.5)),
                ((2.0, 12.5), (5.6, 12.5)),
                ((7.0, 12.5), (9.0, 12.5)),
                ((10.4, 12.5), (14.0, 12.5)),
                ((2.0, 5.2), (2.0, 10.8)),
                ((14.0, 5.2), (14.0, 10.8)),
            ] {
                line(&[p(a.0, a.1), p(b.0, b.1)]);
            }
        }
        Icon::Undo => {
            line(&[p(5.6, 2.8), p(2.6, 5.8), p(5.6, 8.8)]);
            let mut bow = vec![p(2.8, 5.8)];
            bow.extend(arc(8.4, 9.4, 4.6, -125.0, 70.0));
            line(&bow);
        }
        Icon::Plus => {
            line(&[p(8.0, 3.0), p(8.0, 13.0)]);
            line(&[p(3.0, 8.0), p(13.0, 8.0)]);
        }
        Icon::Magnifier => {
            circle(p(7.0, 7.0), 4.4, false);
            line(&[p(10.3, 10.3), p(13.6, 13.6)]);
        }
        Icon::ChevronDown => line(&[p(3.5, 6.0), p(8.0, 10.5), p(12.5, 6.0)]),
        Icon::ChevronUp => line(&[p(3.5, 10.0), p(8.0, 5.5), p(12.5, 10.0)]),
        Icon::ChevronRight => line(&[p(6.0, 3.5), p(10.5, 8.0), p(6.0, 12.5)]),
        Icon::ChevronLeft => line(&[p(10.0, 3.5), p(5.5, 8.0), p(10.0, 12.5)]),
        // Four corners pointing out, and in.
        Icon::Maximize => {
            line(&[p(2.5, 6.0), p(2.5, 2.5), p(6.0, 2.5)]);
            line(&[p(10.0, 2.5), p(13.5, 2.5), p(13.5, 6.0)]);
            line(&[p(13.5, 10.0), p(13.5, 13.5), p(10.0, 13.5)]);
            line(&[p(6.0, 13.5), p(2.5, 13.5), p(2.5, 10.0)]);
        }
        Icon::Restore => {
            line(&[p(2.0, 5.5), p(5.5, 5.5), p(5.5, 2.0)]);
            line(&[p(10.5, 2.0), p(10.5, 5.5), p(14.0, 5.5)]);
            line(&[p(14.0, 10.5), p(10.5, 10.5), p(10.5, 14.0)]);
            line(&[p(5.5, 14.0), p(5.5, 10.5), p(2.0, 10.5)]);
        }
        Icon::ChevronUpDown => {
            line(&[p(4.5, 6.2), p(8.0, 2.8), p(11.5, 6.2)]);
            line(&[p(4.5, 9.8), p(8.0, 13.2), p(11.5, 9.8)]);
        }
        Icon::Eye | Icon::EyeSlash => {
            line(&[
                p(1.5, 8.0),
                p(4.0, 5.0),
                p(8.0, 3.8),
                p(12.0, 5.0),
                p(14.5, 8.0),
                p(12.0, 11.0),
                p(8.0, 12.2),
                p(4.0, 11.0),
                p(1.5, 8.0),
            ]);
            circle(p(8.0, 8.0), 2.0, false);
            if icon == Icon::EyeSlash {
                line(&[p(3.0, 13.5), p(13.0, 2.5)]);
            }
        }
        Icon::Curlybraces => {
            line(&[
                p(6.5, 2.5),
                p(5.2, 3.2),
                p(5.2, 6.6),
                p(3.5, 8.0),
                p(5.2, 9.4),
                p(5.2, 12.8),
                p(6.5, 13.5),
            ]);
            line(&[
                p(9.5, 2.5),
                p(10.8, 3.2),
                p(10.8, 6.6),
                p(12.5, 8.0),
                p(10.8, 9.4),
                p(10.8, 12.8),
                p(9.5, 13.5),
            ]);
        }
        Icon::Check => line(&[p(3.0, 8.5), p(6.5, 12.0), p(13.0, 4.5)]),
        Icon::CheckCircleFill => {
            circle(p(8.0, 8.0), 7.0, true);
            let knock = Stroke::new((1.6 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.add(Shape::Path(PathShape::line(
                vec![p(4.8, 8.2), p(7.1, 10.5), p(11.3, 5.8)],
                knock,
            )));
        }
        Icon::XmarkOctagonFill => {
            let pts: Vec<Pos2> = (0..8)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_8 + i as f32 * std::f32::consts::FRAC_PI_4;
                    p(8.0 + 7.4 * a.cos(), 8.0 + 7.4 * a.sin())
                })
                .collect();
            fill_poly(pts);
            let knock = Stroke::new((1.6 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(5.5, 5.5), p(10.5, 10.5)], knock);
            painter.line_segment([p(10.5, 5.5), p(5.5, 10.5)], knock);
        }
        Icon::Triangle => closed(&[p(8.0, 2.0), p(14.5, 13.5), p(1.5, 13.5)]),
        Icon::TriangleFill => {
            fill_poly(vec![p(8.0, 1.6), p(14.8, 13.8), p(1.2, 13.8)]);
            let knock = Stroke::new((1.5 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(8.0, 6.0), p(8.0, 9.6)], knock);
            painter.circle_filled(p(8.0, 11.6), 0.85 * scale, Color32::from_black_alpha(200));
        }
        Icon::XmarkCircleFill => {
            circle(p(8.0, 8.0), 7.0, true);
            let knock = Stroke::new((1.5 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(5.6, 5.6), p(10.4, 10.4)], knock);
            painter.line_segment([p(10.4, 5.6), p(5.6, 10.4)], knock);
        }
        Icon::Xmark => {
            line(&[p(3.5, 3.5), p(12.5, 12.5)]);
            line(&[p(12.5, 3.5), p(3.5, 12.5)]);
        }
        Icon::Sliders => {
            for (y, x) in [(3.5, 10.0), (8.0, 5.5), (12.5, 9.5)] {
                line(&[p(2.0, y), p(14.0, y)]);
                painter.circle_filled(p(x, y), 1.9 * scale, color);
            }
        }
        Icon::CircleDashed => {
            for i in 0..8 {
                let a0 = i as f32 * std::f32::consts::FRAC_PI_4;
                let a1 = a0 + std::f32::consts::FRAC_PI_4 * 0.55;
                let pts: Vec<Pos2> = (0..=4)
                    .map(|k| {
                        let a = a0 + (a1 - a0) * k as f32 / 4.0;
                        p(8.0 + 6.0 * a.cos(), 8.0 + 6.0 * a.sin())
                    })
                    .collect();
                line(&pts);
            }
        }
        Icon::Key => {
            circle(p(5.0, 10.5), 3.0, false);
            line(&[p(7.2, 8.3), p(13.5, 2.0)]);
            line(&[p(11.5, 4.0), p(13.5, 6.0)]);
        }
        Icon::Books => {
            for (x0, x1) in [(2.0, 5.0), (6.0, 9.0)] {
                painter.rect_stroke(
                    rect_of(p(x0, 2.5), p(x1, 13.5)),
                    round(0.8),
                    stroke,
                    StrokeKind::Middle,
                );
            }
            line(&[
                p(10.3, 3.5),
                p(13.2, 2.8),
                p(14.6, 12.3),
                p(11.5, 13.0),
                p(10.3, 3.5),
            ]);
        }
        Icon::ArrowUpRightSquare => {
            line(&[
                p(7.0, 2.5),
                p(3.0, 2.5),
                p(3.0, 13.0),
                p(13.5, 13.0),
                p(13.5, 9.0),
            ]);
            line(&[p(7.5, 8.5), p(14.0, 2.0)]);
            line(&[p(9.5, 2.0), p(14.0, 2.0), p(14.0, 6.5)]);
        }
        Icon::Clipboard => {
            painter.rect_stroke(
                rect_of(p(3.5, 3.5), p(12.5, 14.5)),
                round(1.5),
                stroke,
                StrokeKind::Middle,
            );
            painter.rect_stroke(
                rect_of(p(6.0, 1.8), p(10.0, 4.6)),
                round(1.0),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(6.0, 8.0), p(10.0, 8.0)]);
            line(&[p(6.0, 10.5), p(10.0, 10.5)]);
        }
        Icon::CopyDoc => {
            painter.rect_stroke(
                rect_of(p(5.5, 5.5), p(13.5, 14.0)),
                round(1.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[
                p(3.0, 10.5),
                p(2.5, 10.5),
                p(2.5, 2.5),
                p(10.0, 2.5),
                p(10.0, 3.5),
            ]);
        }
        Icon::Bell | Icon::BellBadge | Icon::BellFill => {
            let outline = [
                p(3.5, 11.5),
                p(4.5, 10.0),
                p(4.5, 7.0),
                p(5.5, 4.5),
                p(8.0, 3.5),
                p(10.5, 4.5),
                p(11.5, 7.0),
                p(11.5, 10.0),
                p(12.5, 11.5),
            ];
            if icon == Icon::BellFill {
                fill_poly(outline.to_vec());
            } else {
                closed(&outline);
            }
            line(&[p(6.5, 13.3), p(9.5, 13.3)]);
            if icon == Icon::BellBadge {
                painter.circle_filled(p(12.5, 4.0), 2.8 * scale, super::theme::surface());
                painter.circle_filled(p(12.5, 4.0), 1.7 * scale, color);
            }
        }
        Icon::Clock => {
            circle(p(8.0, 8.0), 6.2, false);
            line(&[p(8.0, 4.5), p(8.0, 8.0), p(10.5, 9.5)]);
        }
        Icon::Download => {
            line(&[p(8.0, 2.5), p(8.0, 10.0)]);
            line(&[p(4.8, 7.0), p(8.0, 10.2), p(11.2, 7.0)]);
            line(&[p(2.5, 11.0), p(2.5, 13.5), p(13.5, 13.5), p(13.5, 11.0)]);
        }
        Icon::Switch => {
            painter.rect_stroke(
                rect_of(p(1.5, 4.5), p(14.5, 11.5)),
                round(3.5),
                stroke,
                StrokeKind::Middle,
            );
            painter.circle_filled(p(11.0, 8.0), 2.0 * scale, color);
        }
        Icon::Calendar => {
            painter.rect_stroke(
                rect_of(p(2.0, 3.5), p(14.0, 14.0)),
                round(2.0),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(2.0, 6.8), p(14.0, 6.8)]);
            line(&[p(5.2, 2.0), p(5.2, 4.8)]);
            line(&[p(10.8, 2.0), p(10.8, 4.8)]);
        }
        Icon::Hash => {
            line(&[p(6.0, 2.5), p(4.5, 13.5)]);
            line(&[p(11.5, 2.5), p(10.0, 13.5)]);
            line(&[p(2.5, 5.8), p(13.5, 5.8)]);
            line(&[p(2.0, 10.2), p(13.0, 10.2)]);
        }
        Icon::TextFormat => {
            line(&[p(2.5, 13.0), p(6.5, 3.0), p(10.5, 13.0)]);
            line(&[p(4.0, 9.5), p(9.0, 9.5)]);
            line(&[p(11.0, 13.0), p(14.0, 13.0)]);
        }
        Icon::List => {
            for y in [4.0, 8.0, 12.0] {
                painter.circle_filled(p(3.0, y), 0.9 * scale, color);
                line(&[p(6.0, y), p(14.0, y)]);
            }
        }
        Icon::BarChart => {
            for (x, top) in [(3.0, 8.0), (7.0, 3.5), (11.0, 6.0)] {
                painter.rect_filled(rect_of(p(x, top), p(x + 2.4, 13.5)), round(0.8), color);
            }
        }
        Icon::Waveform => line(&[
            p(1.0, 8.0),
            p(4.0, 8.0),
            p(6.0, 3.0),
            p(8.0, 13.0),
            p(10.0, 6.0),
            p(11.5, 8.0),
            p(15.0, 8.0),
        ]),
        Icon::Cycle => {
            let arc = |from: f32, to: f32| -> Vec<Pos2> {
                (0..=10)
                    .map(|k| {
                        let a = (from + (to - from) * k as f32 / 10.0).to_radians();
                        p(8.0 + 5.6 * a.cos(), 8.0 + 5.6 * a.sin())
                    })
                    .collect()
            };
            line(&arc(200.0, 340.0));
            line(&arc(20.0, 160.0));
            line(&[p(12.6, 2.6), p(13.8, 5.8), p(10.4, 6.0)]);
            line(&[p(3.4, 13.4), p(2.2, 10.2), p(5.6, 10.0)]);
        }
        Icon::Bands => {
            line(&[
                p(1.5, 5.0),
                p(5.0, 3.0),
                p(8.0, 5.0),
                p(11.0, 3.0),
                p(14.5, 5.0),
            ]);
            line(&[
                p(1.5, 11.0),
                p(5.0, 13.0),
                p(8.0, 11.0),
                p(11.0, 13.0),
                p(14.5, 11.0),
            ]);
            line(&[
                p(1.5, 8.0),
                p(5.0, 8.0),
                p(8.0, 8.0),
                p(11.0, 8.0),
                p(14.5, 8.0),
            ]);
        }
        Icon::Palette => {
            circle(p(8.0, 8.0), 6.4, false);
            for (x, y) in [(5.2, 6.2), (8.0, 4.8), (10.8, 6.4)] {
                painter.circle_filled(p(x, y), 0.9 * scale, color);
            }
        }
        Icon::Circle => circle(p(8.0, 8.0), 6.6, false),
        Icon::BitcoinCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            let thin = Stroke::new((1.15 * scale).max(1.0), color);
            let mut b = vec![p(6.3, 8.0), p(6.3, 4.7), p(8.9, 4.7)];
            b.extend(arc(8.9, 6.35, 1.65, -90.0, 90.0));
            b.push(p(6.3, 8.0));
            b.push(p(9.3, 8.0));
            b.extend(arc(9.3, 9.65, 1.65, -90.0, 90.0));
            b.extend([p(6.3, 11.3), p(6.3, 8.0)]);
            painter.add(Shape::Path(PathShape::line(b, thin)));
            for x in [7.2, 8.7] {
                painter.line_segment([p(x, 3.6), p(x, 4.7)], thin);
                painter.line_segment([p(x, 11.3), p(x, 12.4)], thin);
            }
        }
        Icon::BuildingColumns => {
            closed(&[p(1.8, 5.6), p(8.0, 2.0), p(14.2, 5.6)]);
            for x in [4.0, 8.0, 12.0] {
                line(&[p(x, 7.2), p(x, 11.6)]);
            }
            line(&[p(1.8, 13.6), p(14.2, 13.6)]);
        }
        Icon::Percent => {
            line(&[p(12.8, 3.0), p(3.2, 13.0)]);
            circle(p(4.6, 4.6), 2.0, false);
            circle(p(11.4, 11.4), 2.0, false);
        }
        Icon::Gauge | Icon::GaugeHigh => {
            line(&arc(8.0, 9.0, 6.4, 135.0, 405.0));
            let angle: f32 = if icon == Icon::Gauge { 270.0 } else { 315.0 };
            let a = angle.to_radians();
            line(&[p(8.0, 9.0), p(8.0 + 4.3 * a.cos(), 9.0 + 4.3 * a.sin())]);
            circle(p(8.0, 9.0), 1.3, true);
        }
        Icon::Sparkles => {
            sparkle(6.5, 9.5, 5.6);
            sparkle(12.6, 3.6, 2.8);
            sparkle(12.8, 12.6, 1.9);
        }
        Icon::Lightbulb => {
            line(&arc(8.0, 6.2, 4.4, 125.0, 415.0));
            line(&[p(5.5, 9.8), p(6.4, 11.4), p(9.6, 11.4), p(10.5, 9.8)]);
            line(&[p(6.6, 13.0), p(9.4, 13.0)]);
            line(&[p(7.2, 14.5), p(8.8, 14.5)]);
        }
        Icon::ClockArrow => {
            line(&arc(8.5, 8.0, 5.9, 200.0, 515.0));
            line(&[p(1.3, 4.6), p(3.0, 6.0), p(4.8, 4.5)]);
            line(&[p(8.5, 4.8), p(8.5, 8.0), p(10.7, 9.4)]);
        }
        Icon::Flame => closed(&[
            p(8.4, 1.6),
            p(10.9, 4.6),
            p(12.6, 8.2),
            p(12.3, 11.4),
            p(10.4, 13.8),
            p(8.0, 14.5),
            p(5.6, 13.8),
            p(3.8, 11.6),
            p(3.6, 8.6),
            p(5.0, 6.0),
            p(6.3, 8.2),
            p(7.0, 4.6),
        ]),
        Icon::Diamond => closed(&[p(8.0, 1.5), p(14.5, 8.0), p(8.0, 14.5), p(1.5, 8.0)]),
        Icon::ArrowDownRightCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            line(&[p(5.3, 5.3), p(10.6, 10.6)]);
            line(&[p(10.7, 6.4), p(10.7, 10.7), p(6.4, 10.7)]);
        }
        Icon::CheckmarkSeal => {
            let seal = (0..24)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 12.0;
                    let r = if i % 2 == 0 { 7.1 } else { 6.1 };
                    p(8.0 + r * a.cos(), 8.0 + r * a.sin())
                })
                .collect::<Vec<_>>();
            closed(&seal);
            line(&[p(5.2, 8.2), p(7.2, 10.2), p(10.9, 5.9)]);
        }
        Icon::PersonCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            circle(p(8.0, 6.4), 2.3, false);
            line(&arc(8.0, 14.6, 4.6, 222.0, 318.0));
        }
        Icon::SportsCourt => {
            painter.rect_stroke(
                rect_of(p(1.5, 3.5), p(14.5, 12.5)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(8.0, 3.5), p(8.0, 12.5)]);
            circle(p(8.0, 8.0), 2.0, false);
        }
        Icon::CloudSun => {
            circle(p(5.6, 5.4), 2.2, false);
            for (dx, dy) in [(0.0, -1.0), (-1.0, 0.0), (-0.7, -0.7), (0.7, -0.7)] {
                line(&[
                    p(5.6 + dx * 3.3, 5.4 + dy * 3.3),
                    p(5.6 + dx * 4.4, 5.4 + dy * 4.4),
                ]);
            }
            let mut cloud = arc(12.3, 11.4, 2.2, 90.0, -70.0);
            cloud.extend(arc(9.4, 9.9, 3.0, -25.0, -175.0));
            cloud.extend(arc(5.4, 11.7, 1.9, -120.0, -270.0));
            closed(&cloud);
        }
        Icon::ArrowUpRight => {
            line(&[p(4.0, 12.0), p(12.0, 4.0)]);
            line(&[p(6.0, 4.0), p(12.0, 4.0), p(12.0, 10.0)]);
        }
        Icon::ArrowDownRight => {
            line(&[p(4.0, 4.0), p(12.0, 12.0)]);
            line(&[p(12.0, 6.0), p(12.0, 12.0), p(6.0, 12.0)]);
        }
        Icon::ArrowUp => {
            line(&[p(8.0, 13.5), p(8.0, 2.8)]);
            line(&[p(3.8, 7.0), p(8.0, 2.8), p(12.2, 7.0)]);
        }
        Icon::ArrowDown => {
            line(&[p(8.0, 2.5), p(8.0, 13.2)]);
            line(&[p(3.8, 9.0), p(8.0, 13.2), p(12.2, 9.0)]);
        }
        Icon::Pencil => {
            closed(&[
                p(10.6, 2.4),
                p(13.6, 5.4),
                p(5.6, 13.4),
                p(2.2, 13.8),
                p(2.6, 10.4),
            ]);
            line(&[p(9.0, 4.0), p(12.0, 7.0)]);
        }
        Icon::PauseFill => {
            painter.rect_filled(rect_of(p(3.8, 2.5), p(6.6, 13.5)), round(0.9), color);
            painter.rect_filled(rect_of(p(9.4, 2.5), p(12.2, 13.5)), round(0.9), color);
        }
        Icon::PlayFill => fill_poly(vec![p(4.2, 2.2), p(13.6, 8.0), p(4.2, 13.8)]),
        Icon::Ellipsis => {
            for x in [3.0, 8.0, 13.0] {
                circle(p(x, 8.0), 1.35, true);
            }
        }
        Icon::Repeat => {
            line(&[p(2.5, 9.0), p(2.5, 5.0), p(13.0, 5.0)]);
            line(&[p(10.8, 2.8), p(13.0, 5.0), p(10.8, 7.2)]);
            line(&[p(13.5, 7.0), p(13.5, 11.0), p(3.0, 11.0)]);
            line(&[p(5.2, 8.8), p(3.0, 11.0), p(5.2, 13.2)]);
        }
        Icon::OneCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(6.4, 5.6), p(8.4, 4.4), p(8.4, 11.6)]);
        }
        Icon::Trash => {
            line(&[p(2.0, 4.0), p(14.0, 4.0)]);
            line(&[p(6.0, 4.0), p(6.0, 2.2), p(10.0, 2.2), p(10.0, 4.0)]);
            line(&[p(3.6, 4.0), p(4.5, 14.0), p(11.5, 14.0), p(12.4, 4.0)]);
            line(&[p(6.6, 6.8), p(6.8, 11.4)]);
            line(&[p(9.4, 6.8), p(9.2, 11.4)]);
        }
        Icon::BellSlash => {
            closed(&[
                p(3.5, 11.5),
                p(4.5, 10.0),
                p(4.5, 7.0),
                p(5.5, 4.5),
                p(8.0, 3.5),
                p(10.5, 4.5),
                p(11.5, 7.0),
                p(11.5, 10.0),
                p(12.5, 11.5),
            ]);
            line(&[p(6.5, 13.3), p(9.5, 13.3)]);
            line(&[p(2.0, 2.0), p(14.0, 14.0)]);
        }
        Icon::PauseCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(6.4, 5.4), p(6.4, 10.6)]);
            line(&[p(9.6, 5.4), p(9.6, 10.6)]);
        }
        Icon::CheckCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(5.0, 8.2), p(7.2, 10.4), p(11.2, 5.8)]);
        }
        Icon::ClockBadge => {
            line(&arc(7.0, 8.5, 5.6, 300.0, 600.0));
            line(&[p(7.0, 5.6), p(7.0, 8.5), p(9.0, 9.6)]);
            line(&[p(13.6, 2.0), p(13.6, 6.0)]);
            circle(p(13.6, 8.4), 0.9, true);
        }
        Icon::LineDiagonal => line(&[p(2.5, 13.5), p(13.5, 2.5)]),
        Icon::LineTrend => {
            line(&[p(2.8, 13.2), p(13.2, 2.8)]);
            circle(p(2.8, 13.2), 1.5, true);
            circle(p(13.2, 2.8), 1.5, true);
        }
        Icon::FibPoints => {
            // Three points joined by a dotted triangle, like `point.3.connected.trianglepath.dotted`.
            let dots = |a: Pos2, b: Pos2| {
                for k in 1..5 {
                    let t = k as f32 / 5.0;
                    painter.circle_filled(a + (b - a) * t, 0.55 * scale, color);
                }
            };
            let (top, left, right) = (p(8.0, 2.6), p(2.6, 13.0), p(13.4, 13.0));
            dots(top, left);
            dots(left, right);
            dots(top, right);
            for point in [top, left, right] {
                painter.circle_stroke(point, 1.7 * scale, stroke);
            }
        }
        Icon::Paintbrush => {
            closed(&[p(13.8, 1.8), p(14.4, 2.4), p(8.4, 9.6), p(6.6, 8.0)]);
            fill_poly(vec![
                p(6.0, 8.6),
                p(7.8, 10.2),
                p(7.0, 12.6),
                p(4.2, 14.2),
                p(1.6, 14.4),
                p(2.6, 12.6),
                p(3.4, 10.0),
            ]);
        }
        Icon::SunMaxFill => {
            circle(p(8.0, 8.0), 3.1, true);
            for k in 0..8 {
                let a = (k as f32 * 45.0).to_radians();
                line(&[
                    p(8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin()),
                    p(8.0 + 7.0 * a.cos(), 8.0 + 7.0 * a.sin()),
                ]);
            }
        }
        Icon::MoonFill => {
            // A strip between the outer disc's lit edge and the shadow disc's edge, horn to
            // horn (the circles' intersections), so the concave crescent fills cleanly.
            let along = |cx: f32, cy: f32, r: f32, from: f32, to: f32, k: usize| {
                let a = (from + (to - from) * k as f32 / 24.0).to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            };
            let mut mesh = Mesh::default();
            for k in 0..=24 {
                mesh.colored_vertex(along(8.0, 8.0, 6.3, 6.9, 263.1, k), color);
                mesh.colored_vertex(along(11.2, 4.8, 5.0, 52.3, 217.6, k), color);
                if k > 0 {
                    let i = 2 * k as u32;
                    mesh.add_triangle(i - 2, i - 1, i);
                    mesh.add_triangle(i - 1, i, i + 1);
                }
            }
            painter.add(Shape::mesh(mesh));
        }
        Icon::CircleLeftHalfFill => {
            circle(p(8.0, 8.0), 6.6, false);
            fill_poly(arc(8.0, 8.0, 6.6, 90.0, 270.0));
        }
        Icon::Highlighter => {
            closed(&[p(9.6, 1.8), p(14.2, 6.4), p(8.6, 12.0), p(4.0, 7.4)]);
            line(&[
                p(4.0, 7.4),
                p(2.2, 9.2),
                p(2.2, 11.4),
                p(4.4, 11.4),
                p(6.2, 9.6),
            ]);
            line(&[p(2.0, 14.4), p(9.5, 14.4)]);
        }
        Icon::ArrowMarkerTool => {
            fill_poly(vec![
                p(2.2, 12.6),
                p(2.9, 11.2),
                p(8.6, 8.6),
                p(7.6, 4.6),
                p(14.0, 2.0),
                p(11.4, 8.4),
                p(7.4, 7.4),
                p(4.8, 13.1),
            ]);
        }
        Icon::ArrowTool => {
            line(&[p(2.4, 13.6), p(13.2, 2.8)]);
            line(&[p(7.0, 2.8), p(13.2, 2.8), p(13.2, 9.0)]);
        }
        Icon::ArrowMarkUp => {
            fill_poly(vec![
                p(8.0, 1.8),
                p(13.6, 8.2),
                p(10.2, 8.2),
                p(10.2, 14.2),
                p(5.8, 14.2),
                p(5.8, 8.2),
                p(2.4, 8.2),
            ]);
        }
        Icon::ArrowMarkDown => {
            fill_poly(vec![
                p(8.0, 14.2),
                p(13.6, 7.8),
                p(10.2, 7.8),
                p(10.2, 1.8),
                p(5.8, 1.8),
                p(5.8, 7.8),
                p(2.4, 7.8),
            ]);
        }
        Icon::ShapeRectangle => {
            painter.rect_stroke(
                rect_of(p(2.0, 3.5), p(14.0, 12.5)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
        }
        Icon::ShapeRotatedRect => {
            closed(&[p(2.4, 9.6), p(9.0, 2.6), p(13.6, 6.4), p(7.0, 13.4)]);
        }
        Icon::ShapePath => {
            line(&[p(2.4, 12.6), p(6.0, 5.2), p(10.0, 10.4), p(13.6, 3.6)]);
            for (x, y) in [(2.4, 12.6), (6.0, 5.2), (10.0, 10.4), (13.6, 3.6)] {
                circle(p(x, y), 1.3, true);
            }
        }
        Icon::ShapeCircle => {
            circle(p(8.0, 8.0), 5.8, false);
        }
        Icon::ShapeEllipse => {
            let points: Vec<Pos2> = (0..28)
                .map(|i| {
                    let a = i as f32 / 28.0 * std::f32::consts::TAU;
                    p(8.0 + 6.4 * a.cos(), 8.0 + 4.2 * a.sin())
                })
                .collect();
            closed(&points);
        }
        Icon::ShapePolyline => {
            closed(&[
                p(2.6, 11.4),
                p(4.8, 3.8),
                p(10.6, 2.8),
                p(13.6, 9.0),
                p(8.0, 13.4),
            ]);
        }
        Icon::ShapeTriangle => {
            closed(&[p(8.0, 2.6), p(14.0, 13.0), p(2.0, 13.0)]);
        }
        Icon::ShapeArc => {
            line(&arc(8.0, 13.5, 8.6, -152.0, -28.0));
            let start = p(
                8.0 + 8.6 * (-152.0f32).to_radians().cos(),
                13.5 + 8.6 * (-152.0f32).to_radians().sin(),
            );
            let end = p(
                8.0 + 8.6 * (-28.0f32).to_radians().cos(),
                13.5 + 8.6 * (-28.0f32).to_radians().sin(),
            );
            painter.circle_filled(start, 1.3 * scale, color);
            painter.circle_filled(end, 1.3 * scale, color);
        }
        Icon::ShapeCurve => {
            let points: Vec<Pos2> = (0..=18)
                .map(|i| {
                    let t = i as f32 / 18.0;
                    let u = 1.0 - t;
                    let (x0, y0, cx, cy, x1, y1) = (2.0, 12.4, 8.0, -1.4, 14.0, 12.4);
                    p(
                        u * u * x0 + 2.0 * u * t * cx + t * t * x1,
                        u * u * y0 + 2.0 * u * t * cy + t * t * y1,
                    )
                })
                .collect();
            line(&points);
        }
        Icon::ShapeDoubleCurve => {
            let points: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    let u = 1.0 - t;
                    let (x0, y0, x1, y1, x2, y2, x3, y3) =
                        (2.0, 12.6, 5.6, -1.6, 10.4, 17.6, 14.0, 3.4);
                    p(
                        u * u * u * x0
                            + 3.0 * u * u * t * x1
                            + 3.0 * u * t * t * x2
                            + t * t * t * x3,
                        u * u * u * y0
                            + 3.0 * u * u * t * y1
                            + 3.0 * u * t * t * y2
                            + t * t * t * y3,
                    )
                })
                .collect();
            line(&points);
        }
        Icon::LineRay => {
            line(&[p(3.4, 12.6), p(13.6, 2.4)]);
            line(&[p(9.4, 2.4), p(13.6, 2.4), p(13.6, 6.6)]);
            circle(p(2.8, 13.2), 1.5, true);
        }
        Icon::LineInfo => {
            line(&[p(2.4, 13.6), p(9.6, 6.4)]);
            circle(p(2.4, 13.6), 1.3, true);
            circle(p(9.6, 6.4), 1.3, true);
            painter.rect_stroke(
                rect_of(p(7.4, 1.8), p(14.6, 6.6)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(9.0, 3.4), p(13.0, 3.4)]);
            line(&[p(9.0, 5.0), p(12.0, 5.0)]);
        }
        Icon::LineExtended => {
            line(&[p(1.6, 14.4), p(14.4, 1.6)]);
            line(&[p(1.6, 10.4), p(1.6, 14.4), p(5.6, 14.4)]);
            line(&[p(10.4, 1.6), p(14.4, 1.6), p(14.4, 5.6)]);
            circle(p(6.0, 10.0), 1.3, true);
            circle(p(10.0, 6.0), 1.3, true);
        }
        Icon::LineAngle => {
            line(&[p(2.6, 13.4), p(13.4, 4.6)]);
            line(&[p(2.6, 13.4), p(13.6, 13.4)]);
            line(&arc(2.6, 13.4, 6.4, -39.0, 0.0));
            circle(p(2.6, 13.4), 1.3, true);
        }
        Icon::LineHorizontal => {
            line(&[p(1.4, 8.0), p(14.6, 8.0)]);
            circle(p(8.0, 8.0), 1.6, true);
        }
        Icon::LineHorizontalRay => {
            line(&[p(3.0, 8.0), p(14.6, 8.0)]);
            line(&[p(11.4, 5.4), p(14.6, 8.0), p(11.4, 10.6)]);
            circle(p(2.8, 8.0), 1.6, true);
        }
        Icon::LineVertical => {
            line(&[p(8.0, 1.4), p(8.0, 14.6)]);
            circle(p(8.0, 8.0), 1.6, true);
        }
        Icon::LineCross => {
            line(&[p(1.4, 8.0), p(14.6, 8.0)]);
            line(&[p(8.0, 1.4), p(8.0, 14.6)]);
            circle(p(8.0, 8.0), 1.6, true);
        }
        Icon::ChannelParallel => {
            line(&[p(1.8, 9.2), p(14.2, 3.0)]);
            line(&[p(1.8, 14.0), p(14.2, 7.8)]);
            circle(p(1.8, 9.2), 1.2, true);
            circle(p(14.2, 7.8), 1.2, true);
        }
        Icon::ChannelRegression => {
            line(&[p(1.8, 11.4), p(14.2, 4.6)]);
            line(&[p(1.8, 7.8), p(14.2, 1.4)]);
            line(&[p(1.8, 14.8), p(14.2, 8.2)]);
            for (x, y) in [(4.6, 11.6), (7.8, 6.6), (11.2, 6.4)] {
                circle(p(x, y), 0.9, true);
            }
        }
        Icon::ChannelFlat => {
            line(&[p(1.8, 3.4), p(14.2, 3.4)]);
            line(&[p(1.8, 13.4), p(14.2, 6.0)]);
            circle(p(1.8, 13.4), 1.2, true);
            circle(p(14.2, 3.4), 1.2, true);
        }
        Icon::ChannelDisjoint => {
            line(&[p(1.4, 8.6), p(9.4, 3.8)]);
            line(&[p(6.6, 13.8), p(14.6, 9.0)]);
            circle(p(1.4, 8.6), 1.2, true);
            circle(p(14.6, 9.0), 1.2, true);
        }
        Icon::ForkAndrews => {
            // Three prongs from a base, the median starting at the first pivot.
            line(&[p(2.0, 8.0), p(14.4, 8.0)]);
            line(&[p(7.0, 2.6), p(14.4, 2.6)]);
            line(&[p(7.0, 13.4), p(14.4, 13.4)]);
            line(&[p(7.0, 2.6), p(7.0, 13.4)]);
            circle(p(2.0, 8.0), 1.4, true);
        }
        Icon::ForkSchiff => {
            // The median starts level with the first pivot's time, halfway down to the second.
            line(&[p(2.0, 9.4), p(14.4, 9.4)]);
            line(&[p(7.0, 4.0), p(14.4, 4.0)]);
            line(&[p(7.0, 14.8), p(14.4, 14.8)]);
            line(&[p(7.0, 4.0), p(7.0, 14.8)]);
            painter.extend(Shape::dashed_line(
                &[p(2.0, 9.4), p(2.0, 2.8)],
                stroke,
                1.6 * scale,
                1.4 * scale,
            ));
            circle(p(2.0, 2.8), 1.4, false);
        }
        Icon::ForkModifiedSchiff => {
            // The median starts halfway between the first two pivots in time as well.
            line(&[p(4.6, 9.4), p(14.4, 9.4)]);
            line(&[p(7.6, 4.0), p(14.4, 4.0)]);
            line(&[p(7.6, 14.8), p(14.4, 14.8)]);
            line(&[p(7.6, 4.0), p(7.6, 14.8)]);
            painter.extend(Shape::dashed_line(
                &[p(4.6, 9.4), p(2.0, 2.8)],
                stroke,
                1.6 * scale,
                1.4 * scale,
            ));
            circle(p(2.0, 2.8), 1.4, false);
        }
        Icon::ForkInside => {
            // The prongs start on the base and the median runs on to the third pivot.
            line(&[p(2.0, 2.6), p(13.0, 2.6)]);
            line(&[p(2.0, 13.4), p(13.0, 13.4)]);
            line(&[p(2.0, 2.6), p(2.0, 13.4)]);
            line(&[p(2.0, 8.0), p(12.0, 8.0)]);
            circle(p(12.0, 8.0), 1.6, true);
        }
        Icon::PatternXabcd => {
            line(&[
                p(1.6, 12.6),
                p(4.8, 2.8),
                p(8.0, 10.4),
                p(11.0, 5.0),
                p(14.4, 12.8),
            ]);
            painter.extend(Shape::dashed_line(
                &[p(1.6, 12.6), p(8.0, 10.4)],
                stroke,
                1.4 * scale,
                1.2 * scale,
            ));
            painter.extend(Shape::dashed_line(
                &[p(4.8, 2.8), p(11.0, 5.0)],
                stroke,
                1.4 * scale,
                1.2 * scale,
            ));
            circle(p(1.6, 12.6), 1.2, true);
            circle(p(14.4, 12.8), 1.2, true);
        }
        Icon::PatternCypher => {
            line(&[
                p(1.6, 11.4),
                p(4.6, 4.4),
                p(7.2, 9.4),
                p(11.4, 2.2),
                p(14.4, 10.6),
            ]);
            painter.extend(Shape::dashed_line(
                &[p(1.6, 11.4), p(11.4, 2.2)],
                stroke,
                1.4 * scale,
                1.2 * scale,
            ));
            circle(p(1.6, 11.4), 1.2, true);
            circle(p(14.4, 10.6), 1.2, true);
        }
        Icon::PatternHeadShoulders => {
            line(&[
                p(1.2, 12.4),
                p(3.6, 6.8),
                p(5.6, 11.2),
                p(8.0, 2.6),
                p(10.4, 11.2),
                p(12.4, 6.8),
                p(14.8, 12.4),
            ]);
            line(&[p(1.6, 11.2), p(14.4, 11.2)]);
        }
        Icon::PatternAbcd => {
            line(&[p(2.0, 13.0), p(6.0, 3.4), p(10.0, 10.8), p(14.0, 1.8)]);
            painter.extend(Shape::dashed_line(
                &[p(2.0, 13.0), p(10.0, 10.8)],
                stroke,
                1.4 * scale,
                1.2 * scale,
            ));
            circle(p(2.0, 13.0), 1.2, true);
            circle(p(14.0, 1.8), 1.2, true);
        }
        Icon::PatternTriangle => {
            line(&[p(1.8, 3.0), p(4.4, 12.8), p(8.0, 5.4), p(10.4, 10.6)]);
            line(&[p(1.8, 3.0), p(14.4, 7.6)]);
            line(&[p(4.4, 12.8), p(14.4, 8.4)]);
            circle(p(1.8, 3.0), 1.2, true);
            circle(p(10.4, 10.6), 1.2, true);
        }
        Icon::PatternThreeDrives => {
            line(&[
                p(1.4, 13.6),
                p(3.8, 8.0),
                p(5.2, 10.4),
                p(8.2, 4.8),
                p(9.6, 7.2),
                p(13.0, 1.8),
            ]);
            circle(p(1.4, 13.6), 1.2, true);
            circle(p(13.0, 1.8), 1.2, true);
        }
        Icon::WaveImpulse => {
            line(&[
                p(1.4, 13.6),
                p(4.4, 7.0),
                p(6.2, 10.6),
                p(10.4, 3.6),
                p(11.8, 7.4),
                p(14.6, 2.8),
            ]);
            glyph('1', 2.4, 1.2, 4.2);
            glyph('5', 12.0, 8.6, 4.2);
        }
        Icon::WaveCorrection => {
            line(&[p(1.6, 3.4), p(6.4, 12.4), p(9.2, 7.6), p(14.4, 13.4)]);
            glyph('A', 1.2, 6.6, 4.2);
            glyph('C', 11.6, 6.0, 4.2);
        }
        Icon::WaveTriangle => {
            line(&[
                p(1.4, 12.0),
                p(4.0, 3.2),
                p(6.4, 10.8),
                p(8.8, 5.4),
                p(10.8, 9.6),
                p(13.2, 6.4),
            ]);
            glyph('A', 2.4, 12.4, 3.4);
            glyph('E', 11.8, 10.0, 3.4);
        }
        Icon::WaveDouble => {
            line(&[p(1.4, 4.0), p(5.0, 12.6), p(8.6, 7.4), p(14.6, 13.2)]);
            glyph('W', 1.2, 7.6, 3.8);
            glyph('Y', 11.6, 6.0, 3.8);
        }
        Icon::WaveTriple => {
            line(&[
                p(1.4, 4.2),
                p(4.2, 11.6),
                p(6.4, 6.8),
                p(9.2, 12.6),
                p(11.4, 8.0),
                p(14.4, 13.4),
            ]);
            glyph('W', 1.0, 7.0, 3.6);
            glyph('Z', 12.0, 4.6, 3.6);
        }
        Icon::CycleLines => {
            for x in [2.4, 6.1, 9.8, 13.5] {
                line(&[p(x, 2.0), p(x, 14.0)]);
            }
            circle(p(2.4, 8.0), 1.3, true);
            circle(p(6.1, 8.0), 1.3, true);
        }
        Icon::CycleArcs => {
            line(&[p(1.2, 12.4), p(14.8, 12.4)]);
            line(&arc(4.8, 12.4, 3.2, 180.0, 360.0));
            line(&arc(11.2, 12.4, 3.2, 180.0, 360.0));
        }
        Icon::CycleSine => {
            let wave: Vec<Pos2> = (0..=28)
                .map(|k| {
                    let t = k as f32 / 28.0;
                    p(
                        1.4 + 13.2 * t,
                        8.0 - 4.6 * (std::f32::consts::TAU * t).sin(),
                    )
                })
                .collect();
            line(&wave);
        }
        Icon::TextTool => {
            // A capital T.
            line(&[p(2.8, 3.0), p(13.2, 3.0)]);
            line(&[p(8.0, 3.0), p(8.0, 13.4)]);
            line(&[p(5.8, 13.4), p(10.2, 13.4)]);
        }
        Icon::NoteTool => {
            // A text box with a thin leader down to its dot.
            painter.rect_stroke(
                rect_of(p(1.8, 2.2), p(14.2, 9.2)),
                round(1.8),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(4.4, 4.9), p(11.6, 4.9)]);
            line(&[p(4.4, 6.9), p(9.0, 6.9)]);
            line(&[p(8.0, 9.2), p(8.0, 12.6)]);
            circle(p(8.0, 13.3), 1.3, true);
        }
        Icon::PriceNoteTool => {
            // A note over a price level, its leader meeting the level at a dot.
            painter.rect_stroke(
                rect_of(p(1.8, 2.0), p(14.2, 7.6)),
                round(1.8),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(4.4, 4.8), p(8.6, 4.8)]);
            line(&[p(5.5, 7.6), p(5.5, 11.8)]);
            line(&[p(1.8, 12.2), p(14.2, 12.2)]);
            circle(p(5.5, 12.2), 1.3, true);
        }
        Icon::PinTool => {
            // A map pin: the head is a circle, the point is where it is pinned.
            let mut pin = arc(8.0, 6.2, 4.4, 140.0, 400.0);
            pin.push(p(8.0, 14.6));
            closed(&pin);
            circle(p(8.0, 6.2), 1.5, false);
        }
        Icon::TableTool => {
            painter.rect_stroke(
                rect_of(p(1.8, 2.6), p(14.2, 13.4)),
                round(1.6),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(1.8, 6.2), p(14.2, 6.2)]);
            line(&[p(1.8, 9.8), p(14.2, 9.8)]);
            line(&[p(6.0, 2.6), p(6.0, 13.4)]);
            line(&[p(10.0, 2.6), p(10.0, 13.4)]);
        }
        Icon::CalloutTool => {
            // A speech box whose tail points at the thing it is about.
            closed(&[
                p(1.8, 2.4),
                p(14.2, 2.4),
                p(14.2, 9.6),
                p(7.6, 9.6),
                p(4.0, 13.6),
                p(4.4, 9.6),
                p(1.8, 9.6),
            ]);
            line(&[p(4.4, 5.0), p(11.6, 5.0)]);
            line(&[p(4.4, 7.0), p(8.8, 7.0)]);
        }
        Icon::CommentTool => {
            // A small round bubble with three dots.
            painter.rect_stroke(
                rect_of(p(1.8, 2.4), p(14.2, 10.6)),
                round(4.0),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(4.6, 10.6), p(3.6, 13.6), p(7.8, 10.6)]);
            for x in [5.0, 8.0, 11.0] {
                circle(p(x, 6.5), 0.8, true);
            }
        }
        Icon::PriceLabelTool => {
            // A tag pointing left at the price it names.
            closed(&[
                p(1.6, 8.0),
                p(4.8, 4.0),
                p(14.4, 4.0),
                p(14.4, 12.0),
                p(4.8, 12.0),
            ]);
            line(&[p(7.2, 8.0), p(12.0, 8.0)]);
        }
        Icon::SignpostTool => {
            // A star in a circle: the marker on the pole.
            circle(p(8.0, 8.0), 6.3, false);
            fill_poly(star_points(3.7, 1.6));
        }
        Icon::FlagTool => {
            line(&[p(4.0, 1.8), p(4.0, 14.6)]);
            fill_poly(vec![p(4.0, 2.2), p(13.6, 5.0), p(4.0, 8.2)]);
        }
        Icon::ImageTool => {
            painter.rect_stroke(
                rect_of(p(1.8, 2.8), p(14.2, 13.2)),
                round(1.8),
                stroke,
                StrokeKind::Middle,
            );
            circle(p(5.4, 6.4), 1.3, true);
            line(&[
                p(2.4, 12.2),
                p(6.2, 8.6),
                p(8.8, 11.0),
                p(10.8, 9.0),
                p(13.6, 12.2),
            ]);
        }
        Icon::Magnet => {
            // A horseshoe: one thick outline, and its two poles struck solid below a gap so the
            // tips read as the ends that pull.
            let mut body = vec![p(1.4, 14.4)];
            body.extend(arc(8.0, 7.8, 6.6, 180.0, 360.0));
            body.extend([p(14.6, 14.4), p(10.4, 14.4)]);
            body.extend(arc(8.0, 7.8, 2.4, 360.0, 180.0));
            body.push(p(5.6, 14.4));
            closed(&body);
            for (left, right) in [(1.4, 5.6), (10.4, 14.6)] {
                line(&[p(left, 9.9), p(right, 9.9)]);
                painter.rect_filled(rect_of(p(left, 11.7), p(right, 14.4)), 0.0, color);
            }
        }
        Icon::KeepDrawing => {
            closed(&[
                p(1.8, 14.2),
                p(2.6, 10.8),
                p(10.8, 2.6),
                p(13.2, 5.0),
                p(5.0, 13.2),
            ]);
            line(&[p(1.8, 14.2), p(5.0, 13.2)]);
            line(&[p(12.2, 10.4), p(12.2, 14.6)]);
            line(&[p(10.1, 12.5), p(14.3, 12.5)]);
        }
        Icon::LockOpen => {
            painter.rect_stroke(
                rect_of(p(3.0, 7.4), p(13.0, 14.0)),
                round(1.6),
                stroke,
                StrokeKind::Middle,
            );
            let mut shackle = arc(8.0, 6.6, 3.2, 180.0, 360.0);
            shackle.push(p(11.2, 3.4));
            line(&shackle);
        }
        Icon::BringForward => {
            painter.rect_stroke(
                rect_of(p(1.8, 1.8), p(9.8, 9.8)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
            painter.rect_filled(rect_of(p(6.2, 6.2), p(14.2, 14.2)), round(1.2), color);
        }
        Icon::SendBackward => {
            painter.rect_filled(rect_of(p(1.8, 1.8), p(9.8, 9.8)), round(1.2), color);
            painter.rect_stroke(
                rect_of(p(6.2, 6.2), p(14.2, 14.2)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
        }
        Icon::Ruler => {
            closed(&[p(1.4, 10.6), p(10.6, 1.4), p(14.6, 5.4), p(5.4, 14.6)]);
            for (k, long) in [
                (0.0, true),
                (1.0, false),
                (2.0, true),
                (3.0, false),
                (4.0, true),
            ] {
                let base = p(4.3 + k * 1.75, 7.7 - k * 1.75);
                let reach = if long { 2.4 } else { 1.4 };
                line(&[base, base + vec2(reach, reach) * scale]);
            }
        }
        Icon::ArrowLeftRight => {
            line(&[p(2.0, 5.2), p(13.5, 5.2)]);
            line(&[p(10.6, 2.4), p(13.5, 5.2), p(10.6, 8.0)]);
            line(&[p(14.0, 10.8), p(2.5, 10.8)]);
            line(&[p(5.4, 8.0), p(2.5, 10.8), p(5.4, 13.6)]);
        }
        Icon::SpeakerWave => {
            fill_poly(vec![
                p(1.8, 6.0),
                p(4.6, 6.0),
                p(8.0, 2.8),
                p(8.0, 13.2),
                p(4.6, 10.0),
                p(1.8, 10.0),
            ]);
            line(&arc(8.4, 8.0, 2.6, -50.0, 50.0));
            line(&arc(8.4, 8.0, 5.0, -50.0, 50.0));
        }
        Icon::MacWindow => {
            painter.rect_stroke(
                rect_of(p(1.5, 2.5), p(14.5, 13.5)),
                round(2.0),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(1.8, 5.6), p(14.2, 5.6)]);
            for x in [3.6, 5.4, 7.2] {
                circle(p(x, 4.05), 0.55, true);
            }
        }
        Icon::InfoCircle => {
            circle(p(8.0, 8.0), 7.0, true);
            let knock = Stroke::new((1.6 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(8.0, 7.2), p(8.0, 11.6)], knock);
            painter.circle_filled(p(8.0, 4.9), 0.95 * scale, Color32::from_black_alpha(200));
        }
        Icon::LockFill => {
            painter.rect_filled(rect_of(p(3.0, 7.2), p(13.0, 14.2)), round(1.6), color);
            line(&arc(8.0, 7.2, 3.2, 180.0, 360.0));
            line(&[p(4.8, 7.2), p(4.8, 6.6)]);
        }
        Icon::SplitColumns => {
            painter.rect_stroke(
                rect_of(p(1.5, 3.0), p(14.5, 13.0)),
                round(1.6),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(5.9, 3.2), p(5.9, 12.8)]);
            line(&[p(10.1, 3.2), p(10.1, 12.8)]);
        }
        Icon::Tag => {
            closed(&[
                p(2.4, 2.4),
                p(8.4, 2.4),
                p(14.0, 8.0),
                p(8.0, 14.0),
                p(2.4, 8.4),
            ]);
            circle(p(5.4, 5.4), 1.1, true);
        }
        Icon::Bolt => closed(&[
            p(9.2, 1.4),
            p(3.6, 9.0),
            p(7.4, 9.0),
            p(6.6, 14.6),
            p(12.4, 6.8),
            p(8.6, 6.8),
        ]),
        Icon::BoltSlash => {
            closed(&[
                p(9.2, 1.4),
                p(3.6, 9.0),
                p(7.4, 9.0),
                p(6.6, 14.6),
                p(12.4, 6.8),
                p(8.6, 6.8),
            ]);
            line(&[p(2.0, 2.0), p(14.0, 14.0)]);
        }
        // A camera: the body with its viewfinder bump, and the lens.
        Icon::Camera => {
            line(&[p(4.8, 4.6), p(6.0, 2.8), p(10.0, 2.8), p(11.2, 4.6)]);
            painter.rect_stroke(
                rect_of(p(1.5, 4.6), p(14.5, 13.2)),
                round(2.2),
                stroke,
                StrokeKind::Middle,
            );
            circle(p(8.0, 8.9), 2.5, false);
        }
        // A price axis: the line with three ticks of different lengths running in from it.
        Icon::PriceScale => {
            line(&[p(12.0, 2.0), p(12.0, 14.0)]);
            line(&[p(12.0, 3.6), p(6.4, 3.6)]);
            line(&[p(12.0, 8.0), p(3.6, 8.0)]);
            line(&[p(12.0, 12.4), p(7.6, 12.4)]);
        }
        // The X mark: a heavy stroke down to the right, a light one up to the right.
        Icon::XLogo => {
            fill_poly(vec![p(2.2, 2.4), p(5.4, 2.4), p(13.8, 13.6), p(10.6, 13.6)]);
            line(&[p(13.2, 2.4), p(2.8, 13.6)]);
        }
        Icon::Link => {
            line(&[p(6.8, 9.2), p(9.2, 6.8)]);
            line(&[
                p(7.4, 4.6),
                p(8.2, 3.8),
                p(11.4, 3.8),
                p(12.4, 4.8),
                p(12.4, 7.0),
                p(11.6, 7.8),
            ]);
            line(&[
                p(8.6, 11.4),
                p(7.8, 12.2),
                p(4.6, 12.2),
                p(3.6, 11.2),
                p(3.6, 9.0),
                p(4.4, 8.2),
            ]);
        }
        // Two links of a chain on the diagonal, locked into each other.
        Icon::ChainLinked => {
            closed(&capsule(5.3, 10.7, 2.3, 2.3, -45.0));
            closed(&capsule(10.7, 5.3, 2.3, 2.3, -45.0));
        }
        // The same links pulled apart, with a cut between them.
        Icon::ChainBroken => {
            closed(&capsule(4.3, 11.7, 1.5, 2.2, -45.0));
            closed(&capsule(11.7, 4.3, 1.5, 2.2, -45.0));
            line(&[p(5.2, 5.2), p(10.8, 10.8)]);
        }
        Icon::DocText => {
            closed(&[
                p(3.0, 1.8),
                p(9.6, 1.8),
                p(13.0, 5.2),
                p(13.0, 14.2),
                p(3.0, 14.2),
            ]);
            line(&[p(9.4, 1.8), p(9.4, 5.4), p(13.0, 5.4)]);
            for y in [8.0, 10.2, 12.2] {
                line(&[p(5.4, y), p(10.6, y)]);
            }
        }
        Icon::ShareUp => {
            line(&[
                p(5.6, 6.0),
                p(3.0, 6.0),
                p(3.0, 14.2),
                p(13.0, 14.2),
                p(13.0, 6.0),
                p(10.4, 6.0),
            ]);
            line(&[p(8.0, 1.6), p(8.0, 10.0)]);
            line(&[p(5.4, 4.2), p(8.0, 1.6), p(10.6, 4.2)]);
        }
    }
}

/// An icon-only button: a square hit target with a hover/press fill, like a toolbar item.
pub fn button(ui: &mut Ui, icon: Icon, size: f32, color: Color32, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let response = super::accessibility::label(response, tooltip).on_hover_text(tooltip);
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            super::theme::ink(26)
        } else if response.hovered() {
            super::theme::ink(14)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(size * 0.62)),
            icon,
            color,
        );
    }
    response
}

/// An inline glyph, `size` square, allocated in the layout.
pub fn inline(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), rect, icon, color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, pos2};

    #[test]
    fn every_icon_paints_shapes() {
        use Icon::*;
        let all = [
            SidebarLeft,
            SidebarRight,
            ChartLine,
            ChartLineFlat,
            ChartUptrend,
            RectLeft,
            RectTop,
            RectBottom,
            Star,
            StarFill,
            Plus,
            Magnifier,
            ChevronDown,
            ChevronUp,
            ChevronRight,
            ChevronLeft,
            Maximize,
            Restore,
            ChevronUpDown,
            Eye,
            EyeSlash,
            Curlybraces,
            Check,
            CheckCircleFill,
            XmarkOctagonFill,
            Triangle,
            TriangleFill,
            XmarkCircleFill,
            Xmark,
            Sliders,
            CircleDashed,
            Key,
            Books,
            ArrowUpRightSquare,
            CopyDoc,
            Clipboard,
            Bell,
            BellFill,
            Clock,
            Download,
            Switch,
            Calendar,
            Hash,
            TextFormat,
            List,
            Palette,
            BarChart,
            Waveform,
            Cycle,
            Bands,
            Circle,
            BitcoinCircle,
            BuildingColumns,
            Percent,
            Gauge,
            GaugeHigh,
            Sparkles,
            Lightbulb,
            ClockArrow,
            Flame,
            Diamond,
            ArrowDownRightCircle,
            CheckmarkSeal,
            PersonCircle,
            SportsCourt,
            CloudSun,
            ArrowUpRight,
            ArrowDownRight,
            ArrowUp,
            ArrowDown,
            Pencil,
            PauseFill,
            PlayFill,
            Ellipsis,
            Repeat,
            OneCircle,
            Trash,
            BellSlash,
            PauseCircle,
            CheckCircle,
            ClockBadge,
            LineDiagonal,
            LineTrend,
            FibPoints,
            Paintbrush,
            SunMaxFill,
            MoonFill,
            CircleLeftHalfFill,
            Ruler,
            ArrowLeftRight,
            SpeakerWave,
            MacWindow,
            InfoCircle,
            LockFill,
            SplitColumns,
            Tag,
            Bolt,
            BoltSlash,
            Link,
            Camera,
            PriceScale,
            XLogo,
            ChainLinked,
            ChainBroken,
            DocText,
            ShareUp,
            Minus,
            RectFill,
            RectDashed,
            Undo,
            Highlighter,
            ArrowMarkerTool,
            ArrowTool,
            ArrowMarkUp,
            ArrowMarkDown,
            ShapeRectangle,
            ShapeRotatedRect,
            ShapePath,
            ShapeCircle,
            ShapeEllipse,
            ShapePolyline,
            ShapeTriangle,
            ShapeArc,
            ShapeCurve,
            ShapeDoubleCurve,
            LineRay,
            LineInfo,
            LineExtended,
            LineAngle,
            LineHorizontal,
            LineHorizontalRay,
            LineVertical,
            LineCross,
            ChannelParallel,
            ChannelRegression,
            ChannelFlat,
            ChannelDisjoint,
            ForkAndrews,
            ForkSchiff,
            ForkModifiedSchiff,
            ForkInside,
            PatternXabcd,
            PatternCypher,
            PatternHeadShoulders,
            PatternAbcd,
            PatternTriangle,
            PatternThreeDrives,
            WaveImpulse,
            WaveCorrection,
            WaveTriangle,
            WaveDouble,
            WaveTriple,
            CycleLines,
            CycleArcs,
            CycleSine,
            Magnet,
            KeepDrawing,
            LockOpen,
            BringForward,
            SendBackward,
            TextTool,
            NoteTool,
            PriceNoteTool,
            PinTool,
            TableTool,
            CalloutTool,
            CommentTool,
            PriceLabelTool,
            SignpostTool,
            FlagTool,
            ImageTool,
        ];
        for icon in all {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(
                    ui.painter(),
                    Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(16.0)),
                    icon,
                    Color32::WHITE,
                );
            });
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty(), "{icon:?} drew nothing");
        }
    }

    /// The magnet at the sizes it is used, next to its neighbours, in both tints. Run by hand
    /// with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_magnet() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let mut screen = Screen::new(egui::vec2(360.0, 150.0));
        let muted = Color32::from_gray(150);
        let accent = crate::ui::kit::ACCENT;
        let mut last = None;
        for _ in 0..3 {
            last = Some(screen.run(Vec::new(), |ui| {
                let painter = ui.painter();
                let mut x = 12.0;
                for (size, tint) in [(16.0, muted), (18.0, muted), (18.0, accent), (24.0, accent)] {
                    let rect = Rect::from_min_size(pos2(x, 12.0), Vec2::splat(size));
                    paint(painter, rect, Icon::Magnet, tint);
                    x += size + 14.0;
                }
                for (k, icon) in [Icon::Magnet, Icon::KeepDrawing, Icon::LockOpen]
                    .into_iter()
                    .enumerate()
                {
                    let rect =
                        Rect::from_min_size(pos2(12.0 + k as f32 * 76.0, 56.0), Vec2::splat(64.0));
                    paint(painter, rect, icon, accent);
                }
            }));
        }
        screen.save("magnet", last.expect("a frame"));
    }
}
