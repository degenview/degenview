//! The display text of the model layer follows the language in use, and the text that is also an
//! identifier (sent to MCP clients, stored, parsed back) does not. Each test reads the same
//! function in English and in the pseudo-locale, which brackets and accents what went through
//! `tr!`.

use crate::i18n::{Locale, with_locale};
use crate::model::alert::AlertCurrency;
use crate::model::alert::draft::Direction;
use crate::model::alert::pine::{ArmingFacts, blocker, frequency_name};
use crate::model::broker::chart::Unavailable as BrokerUnavailable;
use crate::model::broker::ticket::status_label;
use crate::model::chart::context::{in_replay, remove_drawings_label, remove_indicators_label};
use crate::model::chart::kind::{ChartGroup, LineDash, PriceSource};
use crate::model::chart::snapshot::{HostError, SnapshotAction, UploadError};
use crate::model::chart::template::{ApplyPlan, ApplyScope, TemplateError};
use crate::model::drawings::tool::Tool;
use crate::model::editor_theme::SyntaxRole;
use crate::model::indicators::BuiltIn;
use crate::model::market::timeframe::Timeframe;
use crate::model::mcp::activity::{McpActivityRange, McpActivityStatus};
use crate::model::paper::{PaperOrderSide, PaperOrderType};
use crate::model::portfolio::card::CardKind;
use crate::model::session_stats::{Filter, MarketHours, Note, Outcome, Side, Unavailable};
use crate::model::tab::link::LinkColor;
use crate::model::watchlist::WatchlistError;
use crate::model::webhook::WebhookDeliveryError;
use crate::model::webhook::policy::url_message;
use crate::model::{ChartType, DataSource};
use degenview_brokers::OrderStatus;

fn pseudo<R>(body: impl FnOnce() -> R) -> R {
    with_locale(Locale::PSEUDO, body)
}

/// `text` in English and in the pseudo-locale: the second differs, and is the English put through
/// a catalog (bracketed), not a different sentence.
fn follows_the_language(text: impl Fn() -> String) {
    let english = text();
    let shown = pseudo(&text);
    assert_ne!(
        english, shown,
        "{english:?} did not change in the pseudo-locale"
    );
    assert!(shown.contains('['), "{shown:?} is not bracketed");
}

fn stays_english(text: impl Fn() -> String) {
    assert_eq!(text(), pseudo(&text));
}

#[test]
fn broker_and_order_wording_follows_the_language() {
    follows_the_language(|| BrokerUnavailable::NotConnected.text().to_owned());
    follows_the_language(|| BrokerUnavailable::InReplay.text().to_owned());
    follows_the_language(|| status_label(OrderStatus::Open).0.to_owned());
    follows_the_language(|| PaperOrderSide::Buy.label().to_owned());
    follows_the_language(|| PaperOrderType::StopLimit.label().to_owned());
}

#[test]
fn alert_wording_follows_the_language() {
    follows_the_language(|| Direction::Above.title().to_owned());
    follows_the_language(|| Direction::Rises.blurb().to_owned());
    follows_the_language(|| {
        frequency_name(crate::pine::builtins::alert::frequency::AlertFrequency::All).to_owned()
    });
    follows_the_language(|| AlertCurrency::Eur.display_name().to_owned());
    follows_the_language(|| {
        blocker(ArmingFacts {
            loaded: true,
            instance_exists: false,
            source_loaded: false,
            call_sites: 0,
            duplicate_exists: false,
        })
        .map(|blocker| blocker.title.to_owned())
        .unwrap_or_default()
    });
}

#[test]
fn alert_values_and_codes_stay_as_they_are() {
    stays_english(|| AlertCurrency::Eur.code().to_owned());
    stays_english(|| AlertCurrency::Eur.format_price(1234.5));
}

#[test]
fn chart_wording_follows_the_language() {
    follows_the_language(|| ChartType::HollowCandles.label().to_owned());
    follows_the_language(|| ChartGroup::PriceBased.label().to_owned());
    follows_the_language(|| LineDash::Dashed.label().to_owned());
    follows_the_language(|| remove_drawings_label(3));
    follows_the_language(|| remove_indicators_label(0));
    follows_the_language(|| in_replay().to_owned());
    follows_the_language(|| ApplyScope::ColorsAndCanvas.label().to_owned());
    follows_the_language(|| TemplateError::DuplicateName.message());
    follows_the_language(|| {
        ApplyPlan {
            steps: Vec::new(),
            kept_type: 0,
        }
        .message("Renko")
    });
    follows_the_language(|| SnapshotAction::CopyLink.label().to_owned());
    follows_the_language(|| HostError::NotAUrl.message().to_owned());
    follows_the_language(|| UploadError::Timeout.message("ImgBB"));
    follows_the_language(|| {
        ChartType::VolumeCandles
            .unsupported_reason_text(DataSource::CoinGecko)
            .unwrap_or_default()
            .to_owned()
    });
}

#[test]
fn the_names_a_chart_type_goes_by_to_tools_stay_english() {
    stays_english(|| ChartType::HollowCandles.slug().to_owned());
    stays_english(|| {
        ChartType::VolumeCandles
            .unsupported_reason(DataSource::CoinGecko)
            .unwrap_or_default()
            .to_owned()
    });
    // Pine's own series names are the same in every language.
    stays_english(|| PriceSource::Hlc3.label().to_owned());
}

#[test]
fn drawing_and_editor_wording_follows_the_language() {
    follows_the_language(|| Tool::TrendLine.label().to_owned());
    follows_the_language(|| Tool::Cursor.label().to_owned());
    follows_the_language(|| SyntaxRole::Keyword.label().to_owned());
    follows_the_language(|| SyntaxRole::Comment.hint().to_owned());
}

#[test]
fn indicator_names_shown_differ_from_the_names_sent_to_tools() {
    follows_the_language(|| BuiltIn::Volume.display_title().to_owned());
    follows_the_language(|| BuiltIn::TrendFlips.display_title().to_owned());
    stays_english(|| BuiltIn::Volume.menu_title().to_owned());
    stays_english(|| BuiltIn::Bollinger.menu_title().to_owned());
}

#[test]
fn session_statistics_split_what_is_shown_from_what_tools_are_told() {
    follows_the_language(|| Outcome::GapFill.label().to_owned());
    follows_the_language(|| Outcome::GapFill.hit_when().to_owned());
    follows_the_language(|| Outcome::GapFill.display_question().to_owned());
    follows_the_language(|| MarketHours::UsStock.display_label().to_owned());
    follows_the_language(|| Unavailable::NoBars.display_text().to_owned());
    follows_the_language(|| Note::NoGaps.display_text().to_owned());
    let filter = Filter {
        gap: Side::Up,
        prior_day: Side::Down,
        ..Filter::default()
    };
    follows_the_language(|| filter.display_describe());

    stays_english(|| Outcome::GapFill.question().to_owned());
    stays_english(|| MarketHours::UsStock.label().to_owned());
    stays_english(|| Unavailable::NoBars.text().to_owned());
    stays_english(|| Note::NoGaps.text().to_owned());
    stays_english(|| filter.describe());
    stays_english(|| Outcome::GapFill.slug().to_owned());
}

#[test]
fn timeframes_are_described_in_words_but_named_by_their_code() {
    let timeframe = Timeframe::parse("22m").unwrap();
    follows_the_language(|| timeframe.description());
    follows_the_language(|| Timeframe::DAY_1.description());
    stays_english(|| timeframe.label());
    stays_english(|| timeframe.interval());
}

#[test]
fn portfolio_cards_are_stored_under_their_english_name() {
    follows_the_language(|| CardKind::Allocation.display_title().to_owned());
    follows_the_language(|| CardKind::Allocation.summary().to_owned());
    stays_english(|| CardKind::Allocation.title().to_owned());
}

#[test]
fn watchlist_tab_link_webhook_and_activity_wording_follows_the_language() {
    follows_the_language(|| WatchlistError::ListFull.notice().title.to_owned());
    follows_the_language(|| WatchlistError::Duplicate("BTC".into()).notice().message);
    follows_the_language(|| LinkColor::Red.title().to_owned());
    follows_the_language(|| url_message(WebhookDeliveryError::UnsupportedPort).to_owned());
    follows_the_language(|| McpActivityStatus::Done.label().to_owned());
    follows_the_language(|| McpActivityRange::Today.label().to_owned());
}

#[test]
fn what_is_stored_or_parsed_back_stays_english() {
    stays_english(|| LinkColor::Red.key().to_owned());
    stays_english(|| DataSource::Binance.display_name().to_owned());
    stays_english(|| {
        crate::model::portfolio::csv::type_name(
            crate::model::portfolio::PortfolioTransactionType::TransferIn,
        )
        .to_owned()
    });
}
