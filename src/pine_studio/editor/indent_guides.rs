//! The indent guides: a faint line at each indent level a block of lines shares.

use crate::model::editor_theme::SurfaceColor;
use crate::ui::editor_appearance::Appearance;
use eframe::egui::{self, Shape, Stroke};

/// Leading-space level of each line, with blank lines keeping only the guides both neighbours
/// share.
pub(super) fn indent_levels(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.split('\n').collect();
    let raw: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                let mut width = 0;
                for c in line.chars() {
                    match c {
                        ' ' => width += 1,
                        '\t' => width += 4,
                        _ => break,
                    }
                }
                Some(width / 4)
            }
        })
        .collect();
    (0..raw.len())
        .map(|i| match raw[i] {
            Some(level) => level,
            None => {
                let before = raw[..i].iter().rev().find_map(|l| *l);
                let after = raw[i + 1..].iter().find_map(|l| *l);
                match (before, after) {
                    (Some(a), Some(b)) => a.min(b),
                    _ => 0,
                }
            }
        })
        .collect()
}

pub(super) fn indent_guides(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    char_width: f32,
    caret_index: Option<usize>,
    look: &Appearance,
) -> Vec<Shape> {
    let levels = indent_levels(source);
    let caret_line = caret_index.map(|c| source.chars().take(c).filter(|&ch| ch == '\n').count());
    let active_level = caret_line.and_then(|line| levels.get(line).copied());
    // The active guide runs over the contiguous lines around the caret that still have it.
    let active_span = caret_line
        .zip(active_level)
        .filter(|(_, l)| *l > 0)
        .map(|(line, level)| {
            let mut start = line;
            while start > 0 && levels[start - 1] >= level {
                start -= 1;
            }
            let mut end = line;
            while end + 1 < levels.len() && levels[end + 1] >= level {
                end += 1;
            }
            (start, end, level)
        });
    let mut shapes = Vec::new();
    for (line, level) in levels.iter().enumerate() {
        let Some(row) = galley.rows.get(line) else {
            break;
        };
        let top = origin.y + row.pos.y;
        let bottom = top + row.row.size.y;
        for guide in 1..=*level {
            let x = origin.x + char_width * (4 * (guide - 1)) as f32 + 0.5;
            let active = active_span.is_some_and(|(s, e, l)| guide == l && line >= s && line <= e);
            let color = look.surface(if active {
                SurfaceColor::IndentGuideActive
            } else {
                SurfaceColor::IndentGuide
            });
            shapes.push(Shape::line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom)],
                Stroke::new(1.0, color),
            ));
        }
    }
    shapes
}
