//! The chips the picker lists, and how the arrow keys move between them. Pure: no drawing.

use crate::model::{Section, Timeframe};

/// Chips in one row.
pub(super) const COLUMNS: usize = 4;

/// An arrow key's way across the chips.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Step {
    Up,
    Down,
    Left,
    Right,
}

/// The presets and the pinned timeframes, grouped by section and sorted by duration within one.
pub(super) struct Grid {
    sections: Vec<(Section, Vec<Timeframe>)>,
}

impl Grid {
    pub(super) fn new(pinned: &[Timeframe]) -> Self {
        let sections = [Section::Minutes, Section::Hours, Section::DaysAndUp]
            .into_iter()
            .map(|section| {
                let mut shown: Vec<Timeframe> = Vec::new();
                for timeframe in Timeframe::PRESETS.iter().chain(pinned) {
                    if timeframe.section() == section && !shown.contains(timeframe) {
                        shown.push(*timeframe);
                    }
                }
                // Stable: lengths that tie (`7D` and `1W`) keep the order they were met in.
                shown.sort_by_key(|timeframe| timeframe.nominal_ms());
                (section, shown)
            })
            .collect();
        Self { sections }
    }

    pub(super) fn sections(&self) -> &[(Section, Vec<Timeframe>)] {
        &self.sections
    }

    pub(super) fn all(&self) -> impl Iterator<Item = Timeframe> + '_ {
        self.sections
            .iter()
            .flat_map(|(_, timeframes)| timeframes.iter().copied())
    }

    pub(super) fn contains(&self, timeframe: Timeframe) -> bool {
        self.all().any(|shown| shown == timeframe)
    }

    /// Where `from` goes under `step`, or `None` at an edge. Up and Down keep the column and
    /// clamp to the row they land in; Left and Right walk the chips in reading order. From a
    /// timeframe that is no chip (a typed one) Down and Right go to the first chip.
    pub(super) fn step(&self, from: Timeframe, step: Step) -> Option<Timeframe> {
        let rows: Vec<&[Timeframe]> = self
            .sections
            .iter()
            .flat_map(|(_, timeframes)| timeframes.chunks(COLUMNS))
            .collect();
        let place = rows.iter().enumerate().find_map(|(row, chips)| {
            chips
                .iter()
                .position(|chip| *chip == from)
                .map(|column| (row, column))
        });
        let Some((row, column)) = place else {
            return match step {
                Step::Down | Step::Right => self.all().next(),
                Step::Up | Step::Left => None,
            };
        };
        match step {
            Step::Up => row
                .checked_sub(1)
                .map(|above| rows[above][column.min(rows[above].len() - 1)]),
            Step::Down => rows
                .get(row + 1)
                .map(|below| below[column.min(below.len() - 1)]),
            Step::Left => self
                .all()
                .position(|chip| chip == from)
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| self.all().nth(index)),
            Step::Right => self
                .all()
                .position(|chip| chip == from)
                .and_then(|index| self.all().nth(index + 1)),
        }
    }
}

/// A section's heading.
pub(super) fn title(section: Section) -> &'static str {
    match section {
        Section::Minutes => "Minutes",
        Section::Hours => "Hours",
        Section::DaysAndUp => "Days and up",
    }
}
