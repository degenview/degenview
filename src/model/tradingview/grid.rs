//! TradingView's chart grids, as DegenView's columns. A layout names its arrangement with a code
//! (`s`, `2h`, `3s`…); the table here is the web app's own: `h(…)` puts its parts side by side,
//! `v(…)` one above the other, and a number is the chart at that place in the layout's list.

/// The arrangement of every code.
const ARRANGEMENTS: &[(&str, &str)] = &[
    ("s", "0"),
    ("2h", "h(0 1)"),
    ("2v", "v(0 1)"),
    ("2-1", "v(h(0 2) 1)"),
    ("1-2", "v(0 h(1 2))"),
    ("3h", "h(0 1 2)"),
    ("3v", "v(0 1 2)"),
    ("3s", "h(0 v(1 2))"),
    ("3r", "h(v(0 1) 2)"),
    ("4", "v(h(0 2) h(1 3))"),
    ("4h", "h(0 1 2 3)"),
    ("4v", "v(0 1 2 3)"),
    ("4s", "h(0 v(1 2 3))"),
    ("4s-l", "h(v(1 2 3) 0)"),
    ("1-3", "v(0 h(1 2 3))"),
    ("3-1", "v(h(0 1 2) 3)"),
    ("2-2-l", "h(0 1 v(2 3))"),
    ("2-2-r", "h(v(0 1) 2 3)"),
    ("2-2", "v(h(0 1) v(2 3))"),
    ("1-4", "v(0 h(1 2 3 4))"),
    ("5h", "h(0 1 2 3 4)"),
    ("5v", "v(0 1 2 3 4)"),
    ("5s", "h(0 v(1 2 3 4))"),
    ("5s-l", "h(v(1 2 3 4) 0)"),
    ("2-3", "v(h(0 1) h(2 3 4))"),
    ("3-2", "v(h(0 1 2) h(3 4))"),
    ("4-1", "v(h(0 1 2 3) 4)"),
    ("2-3-l", "h(0 1 v(2 3 4))"),
    ("2-3-r", "h(v(0 1 2) 3 4)"),
    ("6", "v(h(0 2 4) h(1 3 5))"),
    ("6h", "h(0 1 2 3 4 5)"),
    ("6v", "v(0 1 2 3 4 5)"),
    ("6c", "v(h(0 1) h(2 3) h(4 5))"),
    ("2-4", "v(h(0 1) h(2 3 4 5))"),
    ("4-2", "v(h(0 1 2 3) h(4 5))"),
    ("4-3", "v(h(0 1 2 3) h(4 5 6))"),
    ("7h", "h(0 1 2 3 4 5 6)"),
    ("7s", "h(0 v(1 2 3 4 5 6))"),
    ("8", "v(h(0 2 4 6) h(1 3 5 7))"),
    ("8c", "v(h(0 1) h(2 3) h(4 5) h(6 7))"),
    ("8h", "h(0 1 2 3 4 5 6 7)"),
    ("8v", "v(0 1 2 3 4 5 6 7)"),
    ("9s", "v(h(0 1 2) h(3 4 5) h(6 7 8))"),
    ("5-4", "v(h(0 1 2 3 4) h(5 6 7 8))"),
    ("9h", "h(0 1 2 3 4 5 6 7 8)"),
    ("9v", "v(0 1 2 3 4 5 6 7 8)"),
    ("10c5", "v(h(0 2 4 6 8) h(1 3 5 7 9))"),
    ("10h", "h(0 1 2 3 4 5 6 7 8 9)"),
    ("10v", "v(0 1 2 3 4 5 6 7 8 9)"),
    ("12c6", "v(h(0 2 4 6 8 10) h(1 3 5 7 9 11))"),
    ("12c4", "v(h(0 4 8) h(1 5 9) h(2 6 10) h(3 7 11))"),
    ("12h", "h(0 1 2 3 4 5 6 7 8 9 10 11)"),
    ("14c7", "v(h(0 2 4 6 8 10 12) h(1 3 5 7 9 11 13))"),
    ("16c8", "v(h(0 2 4 6 8 10 12 14) h(1 3 5 7 9 11 13 15))"),
    (
        "16c4",
        "v(h(0 4 8 12) h(1 5 9 13) h(2 6 10 14) h(3 7 11 15))",
    ),
];

/// Columns from left to right, each holding chart places from top to bottom. A DegenView grid is
/// columns only, so a chart spanning several of TradingView's columns sits in the first of them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Grid {
    pub columns: Vec<Vec<usize>>,
}

impl Grid {
    /// The arrangement `code` names. A code not in the table is laid out as a near-square grid
    /// of `stored` charts.
    pub fn of(code: &str, stored: usize) -> Self {
        let known = ARRANGEMENTS
            .iter()
            .find(|(name, _)| *name == code)
            .and_then(|(_, expression)| columns(&mut expression.chars().peekable()));
        match known {
            Some(columns) => Self { columns },
            None => Self::square(stored),
        }
    }

    /// `count` charts filled row by row.
    pub fn square(count: usize) -> Self {
        let width = (count as f64).sqrt().ceil().max(1.0) as usize;
        let mut columns = vec![Vec::new(); width.min(count)];
        for place in 0..count {
            columns[place % width].push(place);
        }
        Self { columns }
    }

    /// How many charts the grid shows.
    #[cfg(test)]
    pub fn cells(&self) -> usize {
        self.columns.iter().map(Vec::len).sum()
    }
}

/// The columns of one part of an arrangement.
fn columns(input: &mut std::iter::Peekable<std::str::Chars>) -> Option<Vec<Vec<usize>>> {
    while input.next_if_eq(&' ').is_some() {}
    match input.next()? {
        split @ ('h' | 'v') => {
            input.next_if_eq(&'(')?;
            let mut parts = Vec::new();
            while input.next_if_eq(&')').is_none() {
                parts.push(columns(input)?);
                while input.next_if_eq(&' ').is_some() {}
            }
            Some(if split == 'h' {
                parts.into_iter().flatten().collect()
            } else {
                // One above the other: their columns line up from the left.
                let width = parts.iter().map(Vec::len).max()?;
                (0..width)
                    .map(|column| {
                        parts
                            .iter()
                            .filter_map(|part| part.get(column))
                            .flatten()
                            .copied()
                            .collect()
                    })
                    .collect()
            })
        }
        digit => {
            let mut number = digit.to_digit(10)? as usize;
            while let Some(next) = input.next_if(char::is_ascii_digit) {
                number = number * 10 + next.to_digit(10)? as usize;
            }
            Some(vec![vec![number]])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(code: &str) -> Vec<Vec<usize>> {
        Grid::of(code, 16).columns
    }

    #[test]
    fn h_is_side_by_side_and_v_is_stacked() {
        assert_eq!(of("2h"), [[0], [1]]);
        assert_eq!(of("2v"), [[0, 1]]);
        assert_eq!(of("3s"), [vec![0], vec![1, 2]]);
        assert_eq!(of("4"), [[0, 1], [2, 3]]);
        assert_eq!(of("2-1"), [vec![0, 1], vec![2]]);
    }

    #[test]
    fn every_arrangement_places_each_chart_once() {
        for (code, expression) in ARRANGEMENTS {
            let grid = Grid::of(code, 0);
            let mut places: Vec<usize> = grid.columns.concat();
            places.sort_unstable();
            assert_eq!(places, (0..grid.cells()).collect::<Vec<_>>(), "{code}");
            let numbers = expression
                .split(|c: char| !c.is_ascii_digit())
                .filter(|part| !part.is_empty())
                .count();
            assert_eq!(grid.cells(), numbers, "{code}");
        }
    }

    #[test]
    fn an_unknown_code_is_a_square_of_what_is_stored() {
        assert_eq!(Grid::of("99z", 3).columns, [vec![0, 2], vec![1]]);
        assert_eq!(Grid::of("99z", 1).columns, [[0]]);
    }
}
