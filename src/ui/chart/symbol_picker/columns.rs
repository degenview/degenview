//! Where the list's columns sit. Every row of the list and the captions above them take their
//! positions from one [`Columns`], so artwork, descriptions and exchanges line up down the list
//! and nothing moves when a row is highlighted.

use super::search::{PREDICTION_PROVIDERS, PROVIDERS, market_kind};
use crate::i18n::tr;
use crate::model::DataSource;
use crate::ui::kit::{self, MEDIUM, REGULAR};
use crate::ui::theme;
use eframe::egui::Ui;

/// From a row's left edge to its artwork, and from the last mark to its right edge.
pub const PAD: f32 = 12.0;
pub const END: f32 = 8.0;
/// The artwork's size, and the gap after it.
pub const ICON: f32 = 32.0;
const ICON_GAP: f32 = 12.0;
/// The symbol column's width while there is a description beside it.
const SYMBOL: f32 = 136.0;
const GAP: f32 = 12.0;
/// The exchange's logo and the gap after it.
pub const VENUE_LOGO: f32 = 24.0;
pub const VENUE_GAP: f32 = 10.0;
/// The widest a venue's name is allowed to make its column.
const VENUE_TEXT_MAX: f32 = 132.0;
/// One mark at the right edge: the plus, the check or the compare button.
pub const MARK: f32 = 28.0;
/// The narrowest a description is still worth a column of its own.
const MIN_DESCRIPTION: f32 = 110.0;

pub const VENUE_SIZE: f32 = 14.0;
pub const VENUE_KIND_SIZE: f32 = 13.0;

/// The columns of one list, as offsets from a row's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    /// Where the symbol (and a title) starts.
    pub symbol: f32,
    pub symbol_width: f32,
    /// Where the description starts; `None` in a list too narrow for the column, which says the
    /// description under the symbol instead.
    pub description: Option<f32>,
    pub description_width: f32,
    /// Where the exchange's logo starts.
    pub venue: f32,
    /// The room for the exchange's name.
    pub venue_width: f32,
}

impl Columns {
    /// The columns of rows `width` wide, whose exchange names are at most `venue_text` wide.
    /// `comparing` keeps room for the compare button beside the plus, whether or not the row
    /// under the pointer offers it, so the columns hold still.
    pub fn for_width(width: f32, venue_text: f32, comparing: bool) -> Self {
        let actions = if comparing { 2.0 * MARK + 2.0 } else { MARK };
        let venue_width = venue_text.min(VENUE_TEXT_MAX).ceil();
        let venue = width - END - actions - GAP - venue_width - VENUE_GAP - VENUE_LOGO;
        let symbol = PAD + ICON + ICON_GAP;
        let description = symbol + SYMBOL + GAP;
        let description_width = venue - GAP - description;
        if description_width >= MIN_DESCRIPTION {
            Self {
                symbol,
                symbol_width: SYMBOL,
                description: Some(description),
                description_width,
                venue,
                venue_width,
            }
        } else {
            Self {
                symbol,
                symbol_width: (venue - GAP - symbol).max(40.0),
                description: None,
                description_width: 0.0,
                venue,
                venue_width,
            }
        }
    }

    /// The room from the symbol column to the exchange: what a title that is a sentence takes.
    pub fn title_width(&self) -> f32 {
        (self.venue - GAP - self.symbol).max(40.0)
    }
}

/// The widest venue name or market kind of any provider, as the venue column writes them. The
/// column is the same on every tab, so switching tabs does not move it.
pub fn venue_text_width(ui: &Ui) -> f32 {
    PROVIDERS
        .into_iter()
        .chain(PREDICTION_PROVIDERS)
        .chain([DataSource::CoinMarketCap])
        .flat_map(|source| {
            [
                (source.display_name(), VENUE_SIZE, MEDIUM),
                (market_kind(source), VENUE_KIND_SIZE, REGULAR),
            ]
        })
        // A Binance perpetual says so where its exchange's other markets say "Crypto".
        .chain([(tr!("Perpetual"), VENUE_KIND_SIZE, REGULAR)])
        .map(|(text, size, weight)| {
            kit::galley(ui, text, size, weight, theme::text_primary(), f32::INFINITY)
                .size()
                .x
        })
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_full_sheet_holds_three_columns() {
        let columns = Columns::for_width(628.0, 90.0, false);
        assert_eq!(columns.symbol, 56.0);
        assert_eq!(columns.description, Some(204.0));
        assert!(columns.description_width > 200.0);
        assert_eq!(columns.venue, 628.0 - 8.0 - 28.0 - 12.0 - 90.0 - 34.0);
    }

    #[test]
    fn a_narrow_list_drops_the_description_and_widens_the_symbol() {
        let columns = Columns::for_width(432.0, 90.0, true);
        assert_eq!(columns.description, None);
        assert_eq!(columns.symbol_width, columns.title_width());
        assert!(columns.symbol_width > 136.0);
    }

    #[test]
    fn the_compare_button_has_its_room_whether_or_not_it_shows() {
        let plain = Columns::for_width(628.0, 90.0, false);
        let comparing = Columns::for_width(628.0, 90.0, true);
        assert_eq!(plain.venue - comparing.venue, 30.0);
    }
}
