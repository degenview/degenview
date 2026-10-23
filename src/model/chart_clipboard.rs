//! What Copy puts on the clipboard and Paste accepts: a versioned, bounded envelope of drawings.
//!
//! The text is `degenview-clipboard/v1` on its own line, then JSON. Anything else on the system
//! clipboard (a price, a URL, a different version) is refused before a byte of it is trusted, and
//! every drawing inside is checked the way a saved chart's rows are, so a hostile or corrupt
//! payload cannot put a non-finite price, a rogue vertex count or a ruler on a chart.
//!
//! Pure: no egui, no clipboard access. The caller owns where the text comes from and goes.

use super::drawings::{self, Anchor, Drawing, MAX_POINTS};
use serde::{Deserialize, Serialize};
use std::fmt;

const HEADER: &str = "degenview-clipboard/v1\n";
const VERSION: u32 = 1;
const KIND: &str = "drawings";
/// The longest payload decoded: far above any hand-drawn selection, far below a memory problem.
pub const MAX_BYTES: usize = 256 * 1024;
/// The most drawings one payload may carry.
pub const MAX_DRAWINGS: usize = 500;

/// Why a payload was refused, in words a toast can show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipError {
    /// Nothing DegenView wrote.
    NotDrawings,
    /// Larger than [`MAX_BYTES`].
    TooLarge,
    /// A DegenView payload this version cannot read.
    Unsupported,
    /// Readable but empty or holding a drawing that cannot be drawn.
    Invalid,
    /// The target price cannot take the copy (log scale needs positive prices).
    OutOfRange,
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotDrawings => "The clipboard holds no DegenView drawings.",
            Self::TooLarge => "The copied drawings are too large to paste.",
            Self::Unsupported => "These drawings come from a different version of DegenView.",
            Self::Invalid => "The copied drawings are damaged and were not pasted.",
            Self::OutOfRange => "The drawings do not fit at that price on this scale.",
        })
    }
}

#[derive(Deserialize, Serialize)]
struct Wire {
    version: u32,
    kind: String,
    /// The chart they were copied from, for the notice only; never used to route anything.
    #[serde(default)]
    symbol: String,
    #[serde(default, with = "drawings::lenient")]
    drawings: Vec<Drawing>,
}

/// Drawings taken off a chart, ready to be written or pasted.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingClip {
    pub symbol: String,
    drawings: Vec<Drawing>,
}

impl DrawingClip {
    /// The pasteable part of `drawings` (rulers are transient and unreadable rows are not ours to
    /// copy); `None` when nothing is left.
    pub fn copy<'a>(symbol: &str, drawings: impl IntoIterator<Item = &'a Drawing>) -> Option<Self> {
        let drawings: Vec<Drawing> = drawings
            .into_iter()
            .filter(|drawing| drawing.tool().is_some() && !drawing.is_ruler())
            .take(MAX_DRAWINGS)
            .cloned()
            .collect();
        (!drawings.is_empty()).then(|| Self {
            symbol: symbol.to_owned(),
            drawings,
        })
    }

    /// How many drawings it holds.
    pub fn count(&self) -> usize {
        self.drawings.len()
    }

    /// The text for the clipboard.
    pub fn encode(&self) -> String {
        let wire = Wire {
            version: VERSION,
            kind: KIND.to_owned(),
            symbol: self.symbol.clone(),
            drawings: self.drawings.clone(),
        };
        // A drawing is plain data; serializing it cannot fail.
        let json = serde_json::to_string(&wire).unwrap_or_default();
        format!("{HEADER}{json}")
    }

    /// Reads and validates clipboard text.
    pub fn decode(text: &str) -> Result<Self, ClipError> {
        if text.len() > MAX_BYTES {
            return Err(ClipError::TooLarge);
        }
        let body = text.strip_prefix(HEADER).ok_or(ClipError::NotDrawings)?;
        let wire: Wire = serde_json::from_str(body).map_err(|_| ClipError::Invalid)?;
        if wire.version != VERSION || wire.kind != KIND {
            return Err(ClipError::Unsupported);
        }
        let sound = !wire.drawings.is_empty()
            && wire.drawings.len() <= MAX_DRAWINGS
            && wire
                .drawings
                .iter()
                .all(|drawing| drawing.tool().is_some() && !drawing.is_ruler())
            && wire
                .drawings
                .iter()
                .map(|d| d.anchors().len())
                .sum::<usize>()
                <= MAX_POINTS * 2;
        if !sound {
            return Err(ClipError::Invalid);
        }
        Ok(Self {
            symbol: wire.symbol,
            drawings: wire.drawings,
        })
    }

    /// The point the copy is positioned by: the first anchor of the first drawing.
    pub fn origin(&self) -> Option<Anchor> {
        self.drawings
            .iter()
            .find_map(|drawing| drawing.anchors().first().copied())
    }

    /// New drawings with this copy's shapes, every anchor moved together so the first sits at
    /// `at`: the same time shift for all, and the same price shift (or, on a log scale, the same
    /// price ratio) for all, so the shapes keep their relationships. Ids count up from
    /// `first_id`; a pasted drawing is unlocked and shown.
    pub fn placed(&self, at: Anchor, log: bool, first_id: u64) -> Result<Vec<Drawing>, ClipError> {
        let origin = self.origin().ok_or(ClipError::Invalid)?;
        if !at.price.is_finite() || (log && (at.price <= 0.0 || origin.price <= 0.0)) {
            return Err(ClipError::OutOfRange);
        }
        let shift_time = at.time_ms.saturating_sub(origin.time_ms);
        let shift_price = at.price - origin.price;
        let ratio = at.price / origin.price;
        let mut placed = Vec::with_capacity(self.drawings.len());
        for (index, drawing) in self.drawings.iter().enumerate() {
            let mut copy = drawing.clone().with_id(first_id + index as u64);
            for anchor in copy.anchors_mut() {
                anchor.time_ms = anchor.time_ms.saturating_add(shift_time);
                anchor.price = if log {
                    anchor.price * ratio
                } else {
                    anchor.price + shift_price
                };
                if !anchor.price.is_finite() || (log && anchor.price <= 0.0) {
                    return Err(ClipError::OutOfRange);
                }
            }
            copy.set_locked(false);
            copy.set_hidden(false);
            placed.push(copy);
        }
        Ok(placed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::drawing_style::Visibility;

    fn anchor(time_ms: i64, price: f64) -> Anchor {
        Anchor { time_ms, price }
    }

    fn line(id: u64, a: Anchor, b: Anchor) -> Drawing {
        let mut line = Drawing::build(
            crate::model::Tool::TrendLine,
            id,
            &[a, b],
            crate::model::Tool::TrendLine.default_style(),
        )
        .expect("a trend line");
        line.set_locked(true);
        line.set_hidden(true);
        line.set_visibility(Visibility::default());
        line
    }

    fn clip() -> DrawingClip {
        DrawingClip::copy(
            "BTCUSDT",
            &[
                line(7, anchor(1_000, 100.0), anchor(2_000, 110.0)),
                line(9, anchor(1_500, 90.0), anchor(2_500, 95.0)),
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_copy_round_trips_through_text() {
        let original = clip();
        let text = original.encode();
        assert!(text.starts_with("degenview-clipboard/v1\n"));
        assert_eq!(DrawingClip::decode(&text), Ok(original));
    }

    #[test]
    fn rulers_and_nothing_are_not_copied() {
        let ruler = Drawing::Ruler {
            id: 1,
            a: anchor(0, 1.0),
            b: anchor(1, 2.0),
        };
        assert!(DrawingClip::copy("X", &[ruler]).is_none());
        assert!(DrawingClip::copy("X", &[]).is_none());
    }

    #[test]
    fn foreign_text_is_refused_untouched() {
        assert_eq!(
            DrawingClip::decode("118150.99"),
            Err(ClipError::NotDrawings)
        );
        assert_eq!(DrawingClip::decode(""), Err(ClipError::NotDrawings));
        assert_eq!(
            DrawingClip::decode("{\"version\":1}"),
            Err(ClipError::NotDrawings)
        );
    }

    #[test]
    fn damaged_and_unsupported_payloads_are_refused() {
        let header = "degenview-clipboard/v1\n";
        assert_eq!(
            DrawingClip::decode(&format!("{header}not json")),
            Err(ClipError::Invalid)
        );
        assert_eq!(
            DrawingClip::decode(&format!(
                "{header}{{\"version\":2,\"kind\":\"drawings\",\"drawings\":[]}}"
            )),
            Err(ClipError::Unsupported)
        );
        assert_eq!(
            DrawingClip::decode(&format!(
                "{header}{{\"version\":1,\"kind\":\"alerts\",\"drawings\":[]}}"
            )),
            Err(ClipError::Unsupported)
        );
        // Empty, and a row this version cannot draw, are both invalid.
        assert_eq!(
            DrawingClip::decode(&format!(
                "{header}{{\"version\":1,\"kind\":\"drawings\",\"drawings\":[]}}"
            )),
            Err(ClipError::Invalid)
        );
        assert_eq!(
            DrawingClip::decode(&format!(
                "{header}{{\"version\":1,\"kind\":\"drawings\",\"drawings\":[{{\"Spiral\":{{\"id\":1}}}}]}}"
            )),
            Err(ClipError::Invalid)
        );
    }

    #[test]
    fn a_ruler_smuggled_into_a_payload_is_refused() {
        let text = format!(
            "{HEADER}{{\"version\":1,\"kind\":\"drawings\",\"drawings\":[{{\"Ruler\":{{\"id\":1,\"a\":{{\"time_ms\":0,\"price\":1.0}},\"b\":{{\"time_ms\":1,\"price\":2.0}}}}}}]}}"
        );
        assert_eq!(DrawingClip::decode(&text), Err(ClipError::Invalid));
    }

    #[test]
    fn oversized_text_is_refused_before_parsing() {
        let text = format!("{HEADER}{}", " ".repeat(MAX_BYTES));
        assert_eq!(DrawingClip::decode(&text), Err(ClipError::TooLarge));
    }

    #[test]
    fn too_many_drawings_are_refused() {
        // Small ones, so it is their number that is refused and not their size.
        let many: Vec<Drawing> = (0..=MAX_DRAWINGS as u64)
            .map(|id| {
                Drawing::build(
                    crate::model::Tool::Fib,
                    id,
                    &[anchor(0, 1.0), anchor(1, 2.0)],
                    Default::default(),
                )
                .expect("a Fibonacci")
            })
            .collect();
        let wire = Wire {
            version: VERSION,
            kind: KIND.to_owned(),
            symbol: String::new(),
            drawings: many,
        };
        let text = format!("{HEADER}{}", serde_json::to_string(&wire).unwrap());
        assert_eq!(DrawingClip::decode(&text), Err(ClipError::Invalid));
    }

    #[test]
    fn paste_moves_every_anchor_together_and_assigns_new_ids() {
        let placed = clip().placed(anchor(10_000, 200.0), false, 40).unwrap();
        assert_eq!(placed.len(), 2);
        assert_eq!(placed[0].id(), 40);
        assert_eq!(placed[1].id(), 41);
        // The first anchor lands on the target; the second keeps its offset from it.
        assert_eq!(placed[0].anchors()[0], anchor(10_000, 200.0));
        assert_eq!(placed[0].anchors()[1], anchor(11_000, 210.0));
        assert_eq!(placed[1].anchors()[0], anchor(10_500, 190.0));
        assert_eq!(placed[1].anchors()[1], anchor(11_500, 195.0));
        assert!(placed.iter().all(|d| !d.locked() && !d.hidden()));
    }

    #[test]
    fn paste_keeps_ratios_on_a_log_scale() {
        let placed = clip().placed(anchor(1_000, 200.0), true, 1).unwrap();
        let [first, second] = [placed[0].anchors(), placed[1].anchors()];
        assert!((first[1].price - 220.0).abs() < 1e-9);
        assert!((second[0].price - 180.0).abs() < 1e-9);
    }

    #[test]
    fn paste_refuses_a_price_the_scale_cannot_hold() {
        assert_eq!(
            clip().placed(anchor(0, f64::NAN), false, 1),
            Err(ClipError::OutOfRange)
        );
        assert_eq!(
            clip().placed(anchor(0, 0.0), true, 1),
            Err(ClipError::OutOfRange)
        );
        // A linear shift that would push an anchor negative is fine on a linear scale.
        assert!(clip().placed(anchor(0, 50.0), false, 1).is_ok());
    }

    #[test]
    fn the_source_drawings_are_untouched() {
        let original = clip();
        let before = original.clone();
        let _ = original.placed(anchor(5, 5.0), false, 1);
        assert_eq!(original, before);
    }
}
