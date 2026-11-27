//! The card under the list for a provider that needs a login or could not be searched.

use super::lookup::Lookup;
use super::search::{Listing, Scope};
use crate::i18n::tr;
use crate::model::DataSource;
use crate::service::{self, ProviderError};
use crate::ui::chart::picker_kit::widgets;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{self, Rect, Stroke, Ui, Vec2, pos2, vec2};

/// Whether the view is about `source` alone: its tab, or a `VENUE:` prefix or the pull-down
/// naming it.
fn targets(source: DataSource, lookup: &Lookup, scope: Scope) -> bool {
    lookup.venue == Some(source) || matches!(scope.venues().as_slice(), [only] if *only == source)
}

/// The card to show, if any. `blocked` is the login a pick was just refused for. Credential
/// prompts follow the provider carried by the error, and an explicit venue search takes
/// precedence over providers advertised on broad searches.
pub(super) fn notice_for(
    listing: &Listing,
    lookup: &Lookup,
    scope: Scope,
    blocked: Option<SettingsPage>,
) -> Option<Notice> {
    let missing = |targeted: bool| {
        listing.errors.iter().find_map(|(_, error)| {
            let ProviderError::MissingKey(source) = error else {
                return None;
            };
            let page = SettingsPage::for_source(*source)?;
            let visible = if targeted {
                targets(*source, lookup, scope)
            } else {
                page.advertises_on_broad_search()
            };
            visible.then(|| page.credentials_notice(false))
        })
    };
    blocked
        .map(|page| page.credentials_notice(true))
        .or_else(|| missing(true))
        .or_else(|| missing(false))
        .or_else(|| {
            listing.errors.iter().find_map(|(source, error)| {
                if matches!(error, ProviderError::MissingKey(_)) {
                    return None;
                }
                let title = SettingsPage::for_source(*source)?.search_failure_title()?;
                Some(Notice {
                    title,
                    detail: error.to_string(),
                    settings: None,
                })
            })
        })
}

pub(super) const NOTICE_HEIGHT: f32 = 60.0;
pub(super) const NOTICE_BUTTON_HEIGHT: f32 = 28.0;
/// The button's gap to the card's right edge: the one it has above and below.
pub(super) const NOTICE_BUTTON_INSET: f32 = (NOTICE_HEIGHT - NOTICE_BUTTON_HEIGHT) / 2.0;

/// Which Settings page a notice's button opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettingsPage {
    Alpaca,
    Oanda,
}

impl SettingsPage {
    pub(super) fn for_source(source: DataSource) -> Option<Self> {
        match source {
            DataSource::Alpaca => Some(Self::Alpaca),
            DataSource::Oanda => Some(Self::Oanda),
            _ => None,
        }
    }

    pub(super) fn has_credentials(self) -> bool {
        use service::keychain::api_keys;
        match self {
            Self::Alpaca => api_keys::alpaca_credentials().is_some(),
            Self::Oanda => api_keys::oanda_credentials().is_some(),
        }
    }

    pub(super) fn advertises_on_broad_search(self) -> bool {
        matches!(self, Self::Alpaca)
    }

    pub(super) fn search_failure_title(self) -> Option<&'static str> {
        match self {
            Self::Alpaca => Some(tr!("Stocks could not be searched")),
            Self::Oanda => None,
        }
    }

    pub(super) fn credentials_notice(self, for_pick: bool) -> Notice {
        let (title, detail) = match self {
            Self::Alpaca => (
                if for_pick {
                    tr!("Set up Alpaca to open a stock")
                } else {
                    tr!("Stocks need an Alpaca key")
                },
                tr!("Add your free Alpaca API keys to search and chart US stocks."),
            ),
            Self::Oanda => (
                tr!("Set up Oanda to search forex"),
                tr!("Add your Oanda credentials to search forex, metals and index CFDs."),
            ),
        };
        Notice::missing_key(title, detail.to_owned(), self)
    }
}

/// A card under the list: a title, one line of detail and, when Settings can fix it, a button.
pub(super) struct Notice {
    pub(super) title: &'static str,
    pub(super) detail: String,
    /// The Settings page its button opens; `None` when it has no button.
    pub(super) settings: Option<SettingsPage>,
}

impl Notice {
    pub(super) fn missing_key(title: &'static str, detail: String, page: SettingsPage) -> Self {
        Self {
            title,
            detail,
            settings: Some(page),
        }
    }
}

/// Where the notice's button of `width` sits: at the card's right, as far from its edge as from its
/// top and bottom.
pub(super) fn notice_button_rect(card: Rect, width: f32) -> Rect {
    let size = vec2(width, NOTICE_BUTTON_HEIGHT);
    Rect::from_min_size(
        pos2(
            card.right() - NOTICE_BUTTON_INSET - size.x,
            card.center().y - size.y / 2.0,
        ),
        size,
    )
}

/// Draws `notice` in `card`; returns whether its button was pressed.
pub(super) fn show_notice(ui: &mut Ui, card: Rect, notice: &Notice) -> bool {
    ui.painter()
        .rect_filled(card, kit::CARD_RADIUS, theme::card_fill(false));
    ui.painter().rect_stroke(
        card,
        kit::CARD_RADIUS,
        Stroke::new(1.0, theme::card_border(false)),
        egui::StrokeKind::Inside,
    );
    let badge = Rect::from_center_size(
        pos2(card.left() + 14.0 + 17.0, card.center().y),
        Vec2::splat(34.0),
    );
    kit::paint_card_badge(ui, badge, Icon::Key);

    let mut pressed = false;
    let mut right = card.right() - NOTICE_BUTTON_INSET;
    if notice.settings.is_some() {
        let label = tr!("Open Settings");
        let button = notice_button_rect(card, kit::sheet_button_width(ui, label, false));
        let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(button));
        pressed = widgets::sheet_button(&mut cell, label, false, true).clicked();
        right = button.left() - 12.0;
    }
    let text_left = badge.right() + 12.0;
    let width = (right - text_left).max(40.0);
    let title = kit::galley(
        ui,
        notice.title,
        14.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let detail = kit::galley_rows(
        ui,
        &notice.detail,
        12.5,
        REGULAR,
        theme::text_secondary(),
        width,
        2,
    );
    let block = title.size().y + 2.0 + detail.size().y;
    let top = card.center().y - block / 2.0;
    let detail_top = top + title.size().y + 2.0;
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());
    ui.painter()
        .galley(pos2(text_left, detail_top), detail, theme::text_secondary());
    pressed
}
