//! The words and the view of the Edit sheet. The transitions (what is asked when, when a review
//! stands, what is sent) are tested where the state machine lives, in the crate
//! (`degenview_brokers::edit_session`); these check that each reason it reports is said, and said
//! the same way as before.

use std::cell::RefCell;
use std::str::FromStr;
use std::time::Duration;

use degenview_brokers::edit_session::EditSource;
use degenview_brokers::model::orders::{EditOutcome, OrderChange, OrderFingerprint, Problem};
use degenview_brokers::model::preview::PREVIEW_DEBOUNCE;
use degenview_brokers::{BrokerError, ErrorKind, OrderStatus, Pending, ProductId, Restrictions};

use super::*;

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn order() -> LiveOrder {
    LiveOrder {
        id: OrderId::new("o-1"),
        client_id: None,
        product: ProductId::new("BTC-USD"),
        side: Side::Sell,
        order_type: OrderType::Limit,
        status: OrderStatus::Open,
        tif: None,
        size: Some(OrderSize::Base(d("1"))),
        limit_price: Some(d("100")),
        stop_price: None,
        stop_direction: None,
        post_only: false,
        filled_size: d("0"),
        filled_value: d("0"),
        average_price: None,
        fees: d("0"),
        created_ms: Some(0),
        updated_ms: None,
        reason: None,
        stop_triggered: false,
    }
}

fn spec() -> ProductSpec {
    ProductSpec {
        id: ProductId::new("BTC-USD"),
        base: "BTC".to_owned(),
        quote: "USD".to_owned(),
        display_name: "BTC-USD".to_owned(),
        price_increment: d("0.01"),
        base_increment: d("0.001"),
        quote_increment: d("0.01"),
        base_min: d("0.01"),
        base_max: None,
        quote_min: d("1"),
        quote_max: None,
        restrictions: Restrictions::default(),
        fetched_ms: 0,
    }
}

fn accepted(order: &LiveOrder, account: &AccountId) -> EditPreview {
    EditPreview {
        order_total: Some(d("99")),
        fees: Some(d("0.5")),
        base: Some(OrderFingerprint::of(account, order)),
        ..EditPreview::default()
    }
}

type Answer<T> = Option<Result<T, BrokerError>>;

/// A venue that answers at once with what the test set.
#[derive(Default)]
struct Venue {
    preview: RefCell<Answer<EditPreview>>,
    amend: RefCell<Answer<EditOutcome>>,
}

fn answer<T: Clone>(slot: &RefCell<Answer<T>>) -> Pending<T> {
    Pending::ready(
        slot.borrow()
            .clone()
            .unwrap_or_else(|| Err(BrokerError::new(ErrorKind::Invalid, "nothing was set up"))),
    )
}

impl EditSource for Venue {
    fn preview_edit(&self, _: OrderId, _: OrderChange) -> Pending<EditPreview> {
        answer(&self.preview)
    }

    fn amend(&self, _: OrderId, _: OrderChange, _: OrderFingerprint) -> Pending<EditOutcome> {
        answer(&self.amend)
    }
}

impl EditVenue for Venue {
    fn world(&self, _: &OrderId) -> EditWorld {
        EditWorld::default()
    }
}

struct Rig {
    session: EditSession,
    venue: Venue,
    now: Instant,
    live: Option<LiveOrder>,
    account: Option<AccountId>,
    spec: Option<ProductSpec>,
}

impl Rig {
    fn new() -> Self {
        let now = Instant::now();
        let rig = Self {
            session: EditSession::new(order(), Some(d("101")), now),
            venue: Venue::default(),
            now,
            live: Some(order()),
            account: Some(AccountId::new("acct")),
            spec: Some(spec()),
        };
        rig.venue
            .preview
            .replace(Some(Ok(accepted(&order(), rig.account.as_ref().unwrap()))));
        rig
    }

    fn update(&mut self) -> Step {
        let frame = EditFrame {
            now: self.now,
            live: self.live.as_ref(),
            account: self.account.as_ref(),
            generation: 1,
            spec: self.spec.as_ref(),
        };
        self.session.update(&frame, Some(&self.venue))
    }

    fn press(&mut self, event: EditEvent) -> Step {
        self.session.handle(event, self.now, &self.venue)
    }

    fn confirming(&mut self) {
        self.update();
        self.now += PREVIEW_DEBOUNCE;
        self.update();
        self.update();
        assert!(self.session.is_ready(), "the preview should have arrived");
        self.press(EditEvent::Review);
        assert!(self.session.is_confirming());
    }

    fn editing(&self) -> EditForm {
        let EditView::Editing(form) = self.session.view() else {
            panic!("editing")
        };
        form
    }
}

fn notice(step: Step) -> Option<Notice> {
    match step {
        Step::Closed(notice) => notice,
        Step::Open => panic!("the sheet should have closed"),
    }
}

#[test]
fn a_rule_the_change_breaks_is_said_in_words_before_the_broker_is_asked() {
    let mut rig = Rig::new();
    "100.005".clone_into(rig.session.fields_mut().0);
    rig.update();
    assert!(
        rig.editing().problems[0].starts_with("The price must be a multiple of 0.01"),
        "{:?}",
        rig.editing().problems
    );

    "0.0005".clone_into(rig.session.fields_mut().1);
    "101".clone_into(rig.session.fields_mut().0);
    rig.update();
    let problems = rig.editing().problems;
    assert!(problems.iter().any(|p| p.contains("below the minimum")));
    assert!(problems.iter().any(|p| p.contains("multiple of 0.001")));
}

#[test]
fn the_size_cannot_go_below_what_has_filled() {
    let mut rig = Rig::new();
    let mut partly = order();
    partly.filled_size = d("0.6");
    rig.live = Some(partly);
    "0.5".clone_into(rig.session.fields_mut().1);
    rig.update();
    assert_eq!(
        rig.editing().problems,
        ["The size cannot be less than what has already filled."]
    );
}

#[test]
fn text_that_is_not_a_number_or_not_a_change_says_so() {
    let mut rig = Rig::new();
    "abc".clone_into(rig.session.fields_mut().0);
    rig.update();
    assert_eq!(
        rig.editing().typed_error.as_deref(),
        Some("The price is not a number.")
    );
    "100".clone_into(rig.session.fields_mut().0);
    "x".clone_into(rig.session.fields_mut().1);
    rig.update();
    assert_eq!(
        rig.editing().typed_error.as_deref(),
        Some("The size is not a number.")
    );
    "1".clone_into(rig.session.fields_mut().1);
    rig.update();
    let form = rig.editing();
    assert_eq!(
        form.typed_error, None,
        "unchanged is not an error, only nothing to review"
    );
    assert!(!form.can_review);
}

#[test]
fn the_brokers_own_problems_and_total_are_shown() {
    let mut rig = Rig::new();
    let mut refused = accepted(&order(), rig.account.as_ref().unwrap());
    refused.problems = vec![Problem {
        code: None,
        text: "Insufficient funds".to_owned(),
    }];
    rig.venue.preview.replace(Some(Ok(refused)));
    rig.update();
    rig.now += PREVIEW_DEBOUNCE;
    rig.update();
    rig.update();
    let form = rig.editing();
    assert_eq!(form.broker_problems, ["Insufficient funds"]);
    assert_eq!(form.broker_total, None, "no total for an order it refuses");
    assert!(!form.can_review);

    let mut rig = Rig::new();
    rig.update();
    rig.now += PREVIEW_DEBOUNCE;
    rig.update();
    rig.update();
    assert_eq!(rig.editing().broker_total.as_deref(), Some("99"));
}

#[test]
fn a_review_is_shown_with_the_old_and_new_terms_and_the_brokers_figures() {
    let mut rig = Rig::new();
    rig.confirming();
    let EditView::Reviewing(lines) = rig.session.view() else {
        panic!("reviewing")
    };
    let text: Vec<(&str, &str)> = lines.iter().map(|l| (l.label, l.value.as_str())).collect();
    assert_eq!(
        text,
        [
            ("Market", "BTC-USD"),
            ("Action", "Sell"),
            ("Price", "100 → 101"),
            ("Total (broker's estimate)", "99"),
            ("Fee (broker's estimate)", "0.5"),
        ]
    );
}

#[test]
fn the_sheet_says_why_a_review_was_dropped_or_a_change_was_not_sent() {
    let mut rig = Rig::new();
    let mut unchecked = accepted(&order(), rig.account.as_ref().unwrap());
    unchecked.base = None;
    rig.venue.preview.replace(Some(Ok(unchecked)));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    assert_eq!(
        rig.session.error(),
        Some("This change has not been checked.")
    );

    let mut rig = Rig::new();
    rig.confirming();
    let mut filled = order();
    filled.filled_size = d("0.2");
    rig.live = Some(filled);
    assert!(matches!(rig.update(), Step::Open));
    assert!(rig.session.error().unwrap().contains("order changed"));
    assert_eq!(
        rig.session.filled_size(),
        d("0.2"),
        "the sheet shows the order as it is now"
    );

    let mut rig = Rig::new();
    rig.confirming();
    rig.now += Duration::from_secs(61);
    rig.update();
    assert!(rig.session.error().unwrap().contains("out of date"));
}

#[test]
fn a_failed_amend_shows_the_reason_and_a_sent_one_closes_with_a_notice() {
    let mut rig = Rig::new();
    rig.venue.amend.replace(Some(Err(BrokerError::new(
        ErrorKind::Invalid,
        "The price is too far from the market.",
    ))));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    assert!(matches!(rig.update(), Step::Open));
    assert!(!rig.session.is_sending());
    assert_eq!(
        rig.session.error(),
        Some("The price is too far from the market.")
    );

    let mut sent = Rig::new();
    sent.venue.amend.replace(Some(Ok(EditOutcome::Native)));
    sent.confirming();
    sent.press(EditEvent::Confirm);
    let notice = notice(sent.update()).unwrap();
    assert!(!notice.error && notice.text.starts_with("Edit sent."));
}

#[test]
fn an_amend_whose_outcome_is_unknown_closes_and_says_it_is_read_again() {
    let mut rig = Rig::new();
    rig.venue.amend.replace(Some(Err(BrokerError::new(
        ErrorKind::OutcomeUnknown,
        "The connection dropped.",
    ))));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    let notice = notice(rig.update()).unwrap();
    assert!(notice.error);
    assert!(notice.text.contains("being read again"));
}

#[test]
fn the_sheet_closes_with_a_word_when_the_order_or_the_broker_is_gone() {
    let mut rig = Rig::new();
    rig.live = None;
    let said = notice(rig.update()).unwrap();
    assert!(said.error && said.text.contains("no longer open"));

    let mut rig = Rig::new();
    rig.account = None;
    assert!(notice(rig.update()).unwrap().text.contains("not connected"));

    // Leaving on purpose, and having no venue at all, close without a word.
    let mut rig = Rig::new();
    assert!(notice(rig.press(EditEvent::Cancel)).is_none());
    let frame = EditFrame {
        now: rig.now,
        live: rig.live.as_ref(),
        account: rig.account.as_ref(),
        generation: 1,
        spec: None,
    };
    assert!(notice(rig.session.update(&frame, None)).is_none());
}

#[test]
fn a_stop_limit_is_headed_and_titled_as_one() {
    let mut stop = order();
    stop.order_type = OrderType::StopLimit;
    stop.stop_price = Some(d("95"));
    let mut rig = Rig::new();
    rig.session = EditSession::new(stop.clone(), None, rig.now);
    rig.live = Some(stop);
    rig.update();
    let form = rig.editing();
    assert_eq!(form.price_title, "Stop price");
    assert_eq!(form.heading, "Sell 1 BTC-USD · Stop-limit");
}
